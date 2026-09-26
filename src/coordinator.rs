use anyhow::Result;
use base64::prelude::*;
use log::{debug, info};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch, Mutex as TokioMutex};
use tokio::time::{sleep, Duration};

use crate::cancellation::SmartRemarkableCancellation;
use crate::answer_ui::{draw_status, AnswerStatus};
use crate::config::Config;
use crate::embedded_assets::load_config;
use crate::keyboard::Keyboard;
use crate::pen::Pen;
use crate::llm_engine::{LLMEngine, ModelExecutionStatus};
use crate::screenshot::Screenshot;
use crate::segmenter::ImageAnalyzer;
use crate::simulation::SimulationConfig;
use crate::touch::{Rect, Touch, TriggerSource};

/// Events that can trigger AI processing
#[derive(Debug, Clone)]
pub enum TriggerEvent {
    /// User touched the trigger corner
    UserTouch { source: TriggerSource },
    /// User touched the trigger corner, then tapped the corners of a
    /// selection box and an answer-placement box (select mode)
    UserSelection {
        selection: Rect,
        placement: Rect,
        source: TriggerSource,
    },
    /// Trigger via web API (for testing/simulation)
    WebTrigger,
}

/// Progress states during AI processing
/// Uses ModelExecutionStatus for LLM operations, plus additional states for the full workflow
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressState {
    /// No processing happening
    Idle,
    /// Waiting for user trigger
    WaitingForTrigger,
    /// Taking screenshot
    TakingScreenshot,
    /// LLM execution state
    LlmState(ModelExecutionStatus),
    /// Processing completed successfully
    Done,
}

/// Message from coordinator to processing task
#[derive(Debug)]
pub struct ProcessingRequest {
    /// The trigger event that started this
    pub trigger: TriggerEvent,
}

/// Communication channels for the coordinator
pub struct CoordinatorChannels {
    /// Send trigger events to coordinator
    pub trigger_tx: mpsc::Sender<TriggerEvent>,
    /// Receive trigger events in coordinator
    pub trigger_rx: mpsc::Receiver<TriggerEvent>,

    /// Broadcast progress state updates
    pub progress_tx: watch::Sender<ProgressState>,
    /// Receive progress state updates
    pub progress_rx: watch::Receiver<ProgressState>,
}

impl CoordinatorChannels {
    pub fn new() -> Self {
        let (trigger_tx, trigger_rx) = mpsc::channel(10);
        let (progress_tx, progress_rx) = watch::channel(ProgressState::Idle);

        Self {
            trigger_tx,
            trigger_rx,
            progress_tx,
            progress_rx,
        }
    }
}

impl Default for CoordinatorChannels {
    fn default() -> Self {
        Self::new()
    }
}

/// Consume extra gestures as they arrive while a request is active. Keeping the
/// receiver drained prevents a burst from blocking the listener and replaying
/// buffered sends after the answer finishes.
pub async fn finish_processing<T>(
    mut processing: tokio::task::JoinHandle<T>,
    triggers: &mut mpsc::Receiver<TriggerEvent>,
) -> std::result::Result<T, tokio::task::JoinError> {
    let result = loop {
        tokio::select! {
            result = &mut processing => break result,
            Some(_) = triggers.recv() => info!("Ignoring trigger: an answer is already in progress"),
        }
    };
    while triggers.try_recv().is_ok() {
        info!("Ignoring trigger received during processing");
    }
    result
}

#[cfg(test)]
mod trigger_tests {
    use super::*;

    #[tokio::test]
    async fn discards_bursts_while_busy_and_accepts_next_idle_trigger() {
        let (tx, mut rx) = mpsc::channel(10);
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let processing = tokio::spawn(async move { finish_rx.await.unwrap() });
        let sender = tx.clone();
        let burst = tokio::spawn(async move {
            // More than the channel capacity: the producer must not stay
            // blocked until completion and leak a deferred request afterward.
            for _ in 0..100 {
                sender.send(TriggerEvent::WebTrigger).await.unwrap();
            }
            finish_tx.send(42).unwrap();
        });
        let result = tokio::time::timeout(Duration::from_secs(2), finish_processing(processing, &mut rx)).await.unwrap().unwrap();
        assert_eq!(result, 42);
        burst.await.unwrap();
        assert!(rx.try_recv().is_err());
        tx.send(TriggerEvent::WebTrigger).await.unwrap();
        assert!(matches!(rx.recv().await, Some(TriggerEvent::WebTrigger)));
    }
}

/// Task that waits for triggers and notifies the coordinator
pub async fn trigger_task(
    touch: Arc<tokio::sync::RwLock<Touch>>,
    trigger_tx: mpsc::Sender<TriggerEvent>,
    cancellation: Arc<SmartRemarkableCancellation>,
    no_trigger: bool,
    collect_taps: bool,
) -> Result<()> {
    info!("Trigger task starting");

    loop {
        debug!("Trigger loop looping");

        if no_trigger {
            debug!("No-trigger mode: auto-triggering");
            if trigger_tx
                .send(TriggerEvent::UserTouch {
                    source: TriggerSource::Touch,
                })
                .await
                .is_err()
            {
                info!("Trigger receiver dropped, exiting trigger task");
                break;
            }
            // In no-trigger mode, wait a bit before next auto-trigger or check for cancellation
            tokio::select! {
                _ = sleep(Duration::from_millis(100)) => {
                    if cancellation.should_cancel_main() {
                        info!("Trigger task: cancelled in no-trigger mode");
                        break;
                    }
                }
                _ = async {
                    while !cancellation.should_cancel_main() {
                        sleep(Duration::from_millis(10)).await;
                    }
                } => {
                    info!("Trigger task: cancelled in no-trigger mode");
                    break;
                }
            }
            continue;
        }

        info!("Trigger task: waiting for touch trigger...");

        debug!("Trigger task: about to acquire touch write lock");
        let mut touch_guard = touch.write().await;
        debug!("Trigger task: acquired touch write lock, calling wait_for_trigger");

        match touch_guard.wait_for_trigger(&cancellation).await {
            Ok(()) => {
                debug!("Trigger task: wait_for_trigger returned Ok, touch detected");
                info!("Trigger task: touch detected");

                let source = touch_guard.last_trigger_source();

                // In select mode, collect the selection and placement box corners
                // while we still hold the touch event stream
                let event = if collect_taps && touch_guard.is_real() {
                    match collect_selection(&mut touch_guard, &cancellation, source).await {
                        Ok(event) => event,
                        Err(e) => {
                            if e.to_string().contains("cancelled") {
                                info!("Trigger task: cancelled during selection");
                                return Ok(());
                            }
                            info!("Trigger task: selection failed ({}), ignoring trigger", e);
                            continue;
                        }
                    }
                } else {
                    TriggerEvent::UserTouch { source }
                };

                // Drop the lock before sending the event so processing_task can acquire it
                drop(touch_guard);
                debug!("Trigger task: dropped touch write lock");

                if trigger_tx.send(event).await.is_err() {
                    info!("Trigger receiver dropped, exiting trigger task");
                    break;
                }
                debug!("Trigger task: sent trigger event, continuing loop");

                // Give processing_task a moment to acquire the lock before we loop back
                sleep(Duration::from_millis(50)).await;
            }
            Err(e) => {
                debug!("Trigger task: wait_for_trigger returned Err: {}", e);
                if e.to_string().contains("cancelled") {
                    info!("Trigger task: cancelled (likely config change)");
                    return Ok(()); // Clean exit for restart
                } else {
                    info!("Trigger task: error waiting for trigger: {}", e);
                    return Err(e);
                }
            }
        }
    }

    debug!("Escaped from trigger task loop");

    Ok(())
}

/// Collect the four taps that define the selection box (what to answer)
/// and the placement box (where to draw the answer): two opposite corners each.
async fn collect_selection(
    touch: &mut Touch,
    cancellation: &SmartRemarkableCancellation,
    source: TriggerSource,
) -> Result<TriggerEvent> {
    info!("Select mode: tap two corners of the handwriting to select");
    let sel_a = touch.wait_for_tap(cancellation).await?;
    let sel_b = touch.wait_for_tap(cancellation).await?;
    let selection = Rect::from_corners(sel_a, sel_b);
    info!("Select mode: selection box {:?}; now tap two corners for the answer box", selection);

    let place_a = touch.wait_for_tap(cancellation).await?;
    let place_b = touch.wait_for_tap(cancellation).await?;
    let placement = Rect::from_corners(place_a, place_b);
    info!("Select mode: placement box {:?}", placement);

    Ok(TriggerEvent::UserSelection {
        selection,
        placement,
        source,
    })
}

/// Task that monitors for cancel touch during processing
pub async fn cancel_monitor_task(touch: Arc<tokio::sync::RwLock<Touch>>, cancellation: Arc<SmartRemarkableCancellation>) -> Result<()> {
    info!("Cancel monitor task: starting");

    // Wait for any touch to cancel
    match touch.write().await.wait_for_trigger(&cancellation).await {
        Ok(()) => {
            info!("Cancel monitor task: touch detected, cancelling processing");
            cancellation.cancel_execution();
            Ok(())
        }
        Err(e) => {
            if e.to_string().contains("cancelled") {
                info!("Cancel monitor task: processing completed before touch");
                Ok(())
            } else {
                info!("Cancel monitor task: error: {}", e);
                Err(e)
            }
        }
    }
}

/// Task that displays progress updates on the keyboard
pub async fn progress_task(
    keyboard: Arc<Mutex<Keyboard>>,
    mut progress_rx: watch::Receiver<ProgressState>,
    cancellation: Arc<SmartRemarkableCancellation>,
) -> Result<()> {
    info!("Progress task starting");

    let mut current_state = ProgressState::Idle;
    let cancel_token = cancellation.execution_token();

    loop {
        tokio::select! {
            // Check for cancellation
            _ = cancel_token.cancelled() => {
                info!("Progress task cancelled");
                // Clear any progress display
                if let Ok(mut kb) = keyboard.lock() {
                    let _ = kb.progress_end();
                }
                return Ok(());
            }

            // Watch for progress updates
            result = progress_rx.changed() => {
                if result.is_err() {
                    info!("Progress sender dropped, exiting progress task");
                    break;
                }

                let new_state = progress_rx.borrow().clone();
                if new_state != current_state {
                    current_state = new_state.clone();

                    match &current_state {
                        ProgressState::Idle => {
                            info!("Progress: Idle");
                            if let Ok(mut kb) = keyboard.lock() {
                                let _ = kb.progress_end();
                            }
                        }
                        ProgressState::WaitingForTrigger => {
                            info!("Progress: Waiting for trigger");
                        }
                        ProgressState::TakingScreenshot => {
                            info!("Progress: Taking screenshot...");
                        }
                        ProgressState::LlmState(ModelExecutionStatus::BuildingContext) => {
                            info!("Progress: Building context...");
                            if let Ok(mut kb) = keyboard.lock() {
                                let _ = kb.progress("Thinking");
                            }
                        }
                        ProgressState::LlmState(ModelExecutionStatus::LlmProcessing) => {
                            info!("Progress: Thinking...");
                        }
                        ProgressState::LlmState(ModelExecutionStatus::ProcessingResponse) => {
                            info!("Progress: Processing response...");
                        }
                        ProgressState::LlmState(ModelExecutionStatus::CallingTools) => {
                            info!("Progress: Executing tools...");
                            if let Ok(mut kb) = keyboard.lock() {
                                let _ = kb.progress_end();
                            }
                        }
                        ProgressState::LlmState(ModelExecutionStatus::Done) => {
                            debug!("Progress: LLM Done");
                        }
                        ProgressState::LlmState(ModelExecutionStatus::Error(msg)) => {
                            debug!("Progress: Error - {}", msg);
                        }
                        ProgressState::Done => {
                            debug!("Progress: Done");
                        }
                    }
                }
            }

            // Add dots for thinking state
            _ = sleep(Duration::from_millis(500)) => {
                if matches!(current_state, ProgressState::LlmState(ModelExecutionStatus::LlmProcessing)) {
                    if let Ok(mut kb) = keyboard.lock() {
                        let _ = kb.progress(".");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Task that processes a trigger: screenshot → LLM → tool execution
pub async fn processing_task(
    config: Config,
    engine: Arc<TokioMutex<Box<dyn LLMEngine>>>,
    progress_tx: watch::Sender<ProgressState>,
    cancellation: Arc<SmartRemarkableCancellation>,
    touch: Arc<tokio::sync::RwLock<Touch>>,
    selection: Option<(Rect, Rect)>,
    placement_slot: Arc<Mutex<Option<Rect>>>,
    selection_slot: Arc<Mutex<Option<Rect>>>,
    input_image_slot: Arc<Mutex<Option<String>>>,
    pen: Arc<Mutex<Pen>>,
    answer_marker_slot: Arc<Mutex<Option<Rect>>>,
    answer_delivery_result: crate::answer_delivery::DeliveryResult,
    trigger_source: TriggerSource,
) -> Result<()> {
    info!("Processing task: starting");
    let _awake = crate::awake::RequestWakeLock::acquire(!config.is_test_mode())?;

    // Update progress: taking screenshot
    info!("Setting ProgressState::TakingScreenshot");
    let _ = progress_tx.send(ProgressState::TakingScreenshot);
    tokio::time::sleep(Duration::from_millis(10)).await; // Give progress_task time

    // Load prompt. The Draw button overrides the normal select-mode prompt
    // with prompts/draw.json regardless of --prompt/config.prompt, since it's
    // a distinct action (sketch/refine) from the LLM button's Q&A behavior.
    let prompt_name = if config.select_mode && trigger_source == TriggerSource::DrawButton {
        // With an image-generation model configured, the LLM plans the
        // drawing (prompt-writing) instead of authoring SVG itself
        if config.image_model.is_some() {
            "draw_image.json".to_string()
        } else {
            "draw.json".to_string()
        }
    } else {
        config.prompt.clone()
    };
    let prompt_general_raw = load_config(&prompt_name);
    let prompt_general_json = serde_json::from_str::<serde_json::Value>(prompt_general_raw.as_str())?;
    let mut prompt = prompt_general_json["prompt"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Prompt file '{}' missing required 'prompt' field", prompt_name))?
        .to_string();

    let include_page_context = prompt_general_json["include_page_context"].as_bool().unwrap_or(false)
        && crate::preferences::load()?.page_context;
    let show_answer_status = prompt_general_json["answer_status_marker"].as_bool().unwrap_or(false);
    let temporary_red_ballpoint = prompt_general_json["temporary_red_ballpoint"].as_bool().unwrap_or(false);
    let paginate = prompt_general_json["paginate_answer"].as_bool().unwrap_or(false);
    if let Ok(mut result) = answer_delivery_result.lock() { *result = None; }

    // Take screenshot
    let screenshot_path = config.save_screenshot.clone();
    let automatic_placement = selection.is_none() && config.select_mode;
    let mut selection = selection;
    let mut page_context = None;
    let mut append_source = None;
    let base64_image = if let Some(input_png) = &config.input_png {
        BASE64_STANDARD.encode(std::fs::read(input_png)?)
    } else {
        let mut screenshot = if config.is_test_mode() {
            let simulation_config = SimulationConfig::from_config(&config);
            Screenshot::new_simulated(simulation_config)?
        } else {
            Screenshot::new()?
        };
        screenshot.take_screenshot()?;
        if let Some(save_screenshot) = &config.save_screenshot {
            info!("Saving screenshot to {}", save_screenshot);
            screenshot.save_image(save_screenshot)?;
        }

        // Select mode without tapped boxes (four-finger trigger): look for the
        // native selection-tool marquee in the screenshot and answer below it
        if selection.is_none() && config.select_mode {
            match screenshot.detect_selection_rect() {
                Some(marquee) => {
                    info!("Detected selection marquee {:?}; preparing append placement", marquee);
                    // Capture question/context first; resolve the final viewport
                    // and placement only after the pen dismisses the lasso menu.
                    selection = Some((marquee, marquee));
                }
                None => {
                    info!("No selection marquee found; ignoring trigger (select something first)");
                    let _ = progress_tx.send(ProgressState::Done);
                    return Ok(());
                }
            }
        }

        let image = if let Some((selection_rect, _)) = &selection {
            if include_page_context {
                // Both images come from this capture, before any status ink or
                // UI interaction. Never recapture a different page as context.
                page_context = Some(screenshot.base64()?);
            }
            screenshot.base64_cropped(*selection_rect)?
        } else {
            screenshot.base64()?
        };
        if automatic_placement {
            append_source = Some(screenshot);
        }
        image
    };

    if config.no_submit {
        info!("Skipping LLM submission (no_submit mode)");
        let _ = progress_tx.send(ProgressState::Done);
        return Ok(());
    }
    let use_red_pen = temporary_red_ballpoint && show_answer_status
        && !config.no_draw && !config.is_test_mode() && selection.is_some();
    let request = async {
        if let (Some(screen), Some((question, _))) = (append_source.as_ref(), selection) {
            let rect = if use_red_pen {
                crate::page_layout::prepare_append(question).await?
            } else {
                crate::page_layout::append_rect(screen, question)?
            };
            info!("Append answer placement: {:?}", rect);
            selection = Some((question, rect));
        }

        // Arm the placement slot so the draw_svg tool scales the answer into
        // the box the user chose, and the selection slot so the Draw button's
        // draw_sketch tool can redraw into the ORIGINAL lassoed box instead
        // (when the model reports the selection was already a drawing).
        if let Some((selection_rect, placement_rect)) = &selection {
            if let Ok(mut slot) = placement_slot.lock() {
                *slot = Some(*placement_rect);
            }
            if let Ok(mut slot) = selection_slot.lock() {
                *slot = Some(*selection_rect);
            }
        }
        // Arm the input-image slot so the image-generation draw tool can attach
        // the cropped selection to its request (sketch-enhancement mode)
        if let Ok(mut slot) = input_image_slot.lock() {
            *slot = Some(base64_image.clone());
        }

        // Tap middle bottom to position cursor for text input (before showing
        // "Thinking"). Skipped in select mode: the tap dismisses the active
        // marquee and its floating menu, which the in-place redraw needs (the
        // draw tool deletes the lassoed strokes via that menu's trash button).
        if !config.select_mode {
            if let Err(e) = touch.write().await.tap_middle_bottom().await {
                info!("Failed to tap middle bottom: {}", e);
            }
        }

        // Update progress: building context
        let _ = progress_tx.send(ProgressState::LlmState(ModelExecutionStatus::BuildingContext));
        tokio::time::sleep(Duration::from_millis(10)).await; // Give progress_task time

        // Apply segmentation if requested
        let segmentation_description = if config.apply_segmentation {
            let image_path = config
                .input_png
                .as_ref()
                .or(screenshot_path.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Segmentation requires either input_png or save_screenshot"))?;

            info!("Applying segmentation to {}", image_path);
            let analyzer = ImageAnalyzer::new(0.001, 10); // min_region_size=0.1%, max_regions=10
            match analyzer.analyze_image(image_path) {
                Ok(result) => {
                    let description = analyzer.generate_description(&result);
                    info!("Segmentation found {} regions", result.regions.len());
                    Some(description)
                }
                Err(e) => {
                    info!("Segmentation failed: {}, continuing without it", e);
                    None
                }
            }
        } else {
            None
        };

        // Add segmentation to prompt if available
        if let Some(seg_desc) = segmentation_description {
            prompt.push_str("\n\nImage Analysis:\n");
            prompt.push_str(&seg_desc);
        }
        if show_answer_status {
            if let Some((_, rect)) = selection {
                let max_lines = if paginate { 128 } else { ((rect.h - 34) / 31).clamp(1, 16) };
                let max_chars = (((rect.w - 28) as f32 / 13.2).floor() as i32).clamp(12, 52);
                prompt.push_str(&format!("\nReply layout: use at most {max_lines} lines, each at most {max_chars} characters. Answer all parts of the question concisely."));
                if paginate {
                    prompt.push_str("\nReply pagination: additional note pages are available. Complete the answer; the application handles page breaks at the original font size.");
                }
            }
        }

        // Prepare engine
        let mut engine_guard = engine.lock().await;
        engine_guard.clear_content();
        if page_context.is_some() {
            engine_guard.add_text_content("Image 1: selected handwriting, the latest user question.");
        }
        engine_guard.add_image_content(&base64_image);
        if let Some(page) = &page_context {
            engine_guard.add_text_content("Image 2: surrounding visible page, including earlier user notes and AI replies. Off-screen content is not included.");
            engine_guard.add_image_content(page);
        }
        engine_guard.add_text_content(&prompt);
        info!("Request context: selected image, visible page included={}", page_context.is_some());

        if show_answer_status && !config.no_draw && !config.is_test_mode() {
            crate::preferences::prepare_label().await;
            if let Some((_, rect)) = selection {
                match draw_status(Arc::clone(&pen), rect, AnswerStatus::Pending, use_red_pen).await {
                    Ok(()) => {
                        info!("Request pending marker drawn");
                        if let Ok(mut slot) = answer_marker_slot.lock() {
                            *slot = Some(rect);
                        }
                    }
                    Err(error) => info!("Could not draw request status: {}", error),
                }
            }
        }

        // Create status callback that wraps model execution status in LlmState
        let progress_tx_clone = progress_tx.clone();
        let status_callback = Some(Box::new(move |status: ModelExecutionStatus| {
            let _ = progress_tx_clone.send(ProgressState::LlmState(status));
        }) as Box<dyn FnMut(ModelExecutionStatus) + Send>);

        // Execute LLM with proper error handling
        info!("Processing task: calling LLM");
        let mut execution_result = engine_guard.execute(&cancellation, status_callback).await;
        let inference_failed = execution_result.is_err();
        if paginate && !config.no_draw && !config.is_test_mode() {
            let delivery = answer_delivery_result.lock().ok().and_then(|mut result| result.take());
            if execution_result.is_ok() {
                execution_result = match delivery {
                    Some(Ok(())) => Ok(()),
                    Some(Err(error)) => Err(anyhow::anyhow!("Answer delivery incomplete: {error}")),
                    None => Err(anyhow::anyhow!("No complete answer was drawn")),
                };
            }
        }

        // A successful draw_answer consumes this slot and checks the box. An API
        // error or response without a rendered answer leaves it pending: cross it.
        let pending = answer_marker_slot.lock().ok().and_then(|mut slot| slot.take());
        if let Some(rect) = pending {
            if inference_failed && paginate && use_red_pen && !cancellation.should_cancel() {
                // Inference failed before any answer lines were drawn. This
                // known empty answer area can show a readable failure notice.
                let width = (((rect.w - 28) as f32 / 13.2).floor() as usize).clamp(12, 52);
                let diagnostic = execution_result.as_ref().err().map(|e| format!("{e:#}")).unwrap_or_default();
                let notice = if diagnostic.contains("bridge") || diagnostic.contains("Connection") || diagnostic.contains("transport") {
                    ["Mac connection unavailable.", "Reconnect, then try again."]
                } else if diagnostic.contains("possible tool actions") {
                    ["A tool may have already run.", "Stopped to avoid repeating it."]
                } else if diagnostic.contains("All configured backends") {
                    ["All selected backends failed.", "Please check the Mac services."]
                } else { ["The backend did not finish.", "Please try again."] };
                crate::preferences::set_phase(notice[0]);
                if let Ok(lines) = crate::answer_delivery::wrap_lines(&notice.map(str::to_string), width) {
                    if let Ok(fragments) = crate::answer_ui::answer_svgs(&lines, rect) {
                        for svg in fragments {
                            let _ = tokio::task::block_in_place(|| pen.lock().map_err(|_| anyhow::anyhow!("Pen lock unavailable"))?.draw_svg_centerline(&svg));
                        }
                    }
                }
            }
            if let Err(error) = draw_status(Arc::clone(&pen), rect, AnswerStatus::Failed, use_red_pen).await {
                info!("Could not update failed request marker: {}", error);
            }
        }

        execution_result
    };
    // The same cleanup covers model failures, render failures, and cancellation.
    let execution_result = if use_red_pen {
        crate::ink_session::with_red_ballpoint(|| request).await
    } else {
        request.await
    };

    // Write model output if configured
    if let Some(model_output_file) = &config.model_output_file {
        info!("Would write model output to {}", model_output_file);
        // Note: The actual model output would need to be captured from the engine
        // This is a placeholder - the LLMEngine trait would need to expose the raw response
    }

    // Disarm both slots so later non-select runs draw normally
    if let Ok(mut slot) = placement_slot.lock() {
        slot.take();
    }
    if let Ok(mut slot) = selection_slot.lock() {
        slot.take();
    }
    if let Ok(mut slot) = input_image_slot.lock() {
        slot.take();
    }

    // Handle execution result
    match execution_result {
        Ok(_) => {
            let _ = progress_tx.send(ProgressState::Done);
            info!("Processing task: completed successfully");
            Ok(())
        }
        Err(e) => {
            let error_msg = e.to_string();
            info!("Processing task: LLM error: {}", error_msg);

            // Only send error state if not already cancelled
            if !error_msg.contains("cancelled") && !error_msg.contains("canceled") {
                let _ = progress_tx.send(ProgressState::LlmState(ModelExecutionStatus::Error(error_msg.clone())));
                // Keep error visible for a moment
                sleep(Duration::from_secs(2)).await;
            }

            // Return to idle state
            let _ = progress_tx.send(ProgressState::Idle);
            Err(e)
        }
    }
}
