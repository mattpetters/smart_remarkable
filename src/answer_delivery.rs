//! Complete marked answers across native note pages, without replaying ink.
use crate::{
    answer_ui::{answer_svgs, status_svg, AnswerStatus},
    cancellation::SmartRemarkableCancellation,
    pen::Pen,
    touch::Rect,
};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub type DeliveryResult = Arc<Mutex<Option<Result<(), String>>>>;
const MAX_LINES: usize = 256;
const MAX_PAGES: usize = 12;

pub fn wrap_lines(lines: &[String], width: usize) -> Result<Vec<String>> {
    ensure!(
        width >= 12 && lines.iter().map(|s| s.chars().count()).sum::<usize>() <= 8192,
        "Answer exceeds supported bounds"
    );
    let mut result = Vec::new();
    for text in lines {
        ensure!(
            !text.chars().any(|c| c.is_control() && !c.is_whitespace()),
            "Answer contains control characters"
        );
        let mut line = String::new();
        for word in text.split_whitespace() {
            let chars: Vec<_> = word.chars().collect();
            if !line.is_empty() && line.chars().count() + 1 + chars.len() > width {
                result.push(std::mem::take(&mut line));
            }
            for part in chars.chunks(width) {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.extend(part);
                if line.chars().count() >= width {
                    result.push(std::mem::take(&mut line));
                }
            }
        }
        if !line.is_empty() {
            result.push(line);
        }
    }
    ensure!(!result.is_empty() && result.len() <= MAX_LINES, "Answer is empty or exceeds supported pages");
    Ok(result)
}

fn capacity(rect: Rect) -> Result<usize> {
    ensure!(rect.h >= 65 && rect.w >= 190, "Answer space is too small");
    Ok(((rect.h - 34) / 31) as usize)
}

#[async_trait]
trait Ui: Send {
    async fn draw(&mut self, svg: &str) -> Result<()>;
    async fn next_page(&mut self, x: i32) -> Result<Rect>;
    fn marker(&mut self, rect: Option<Rect>);
}

async fn deliver(ui: &mut impl Ui, lines: &[String], drawings: &[crate::illustration::Illustration], mut rect: Rect, mut pending: bool) -> Result<()> {
    crate::illustration::validate(drawings)?;
    let width = (((rect.w - 28) as f32 / 13.2).floor() as usize).clamp(12, 52);
    let required_width = rect.w;
    let lines = wrap_lines(lines, width)?;
    let (mut cursor, mut figure, mut pages) = (0, 0, 1);
    loop {
        let end = (cursor + capacity(rect)?).min(lines.len());
        ensure!(end > cursor || figure < drawings.len(), "No answer content remaining");
        if !pending {
            ui.draw(&status_svg(rect, AnswerStatus::Pending)?).await?;
        }
        ui.marker(Some(rect));
        let used = if cursor < lines.len() {
            for svg in answer_svgs(&lines[cursor..end], rect)? {
                ui.draw(&svg).await?;
            }
            let used = 34 + (end - cursor) as i32 * 31;
            cursor = end;
            used
        } else {
            ui.draw(&crate::illustration::svg(&drawings[figure], rect)?).await?;
            figure += 1;
            crate::illustration::HEIGHT
        };
        let more = cursor < lines.len() || figure < drawings.len();
        ui.draw(&status_svg(rect, if more { AnswerStatus::Continued } else { AnswerStatus::Complete })?)
            .await?;
        ui.marker(None);
        if !more {
            log::info!("Answer delivery complete: {} lines across {} page areas", cursor, pages);
            return Ok(());
        }
        if cursor == lines.len() && rect.w >= crate::illustration::MIN_WIDTH && rect.h - used - 18 >= crate::illustration::HEIGHT {
            rect.y += used + 18;
            rect.h -= used + 18;
            pending = false;
            continue;
        }
        ensure!(pages < MAX_PAGES, "Answer continuation limit reached");
        // Never replay a line or retry page insertion: either could duplicate
        // partially committed device edits. Only transport and UI reads retry.
        rect = ui.next_page(if cursor == lines.len() { 64 } else { rect.x }).await?;
        ensure!(rect.w >= required_width, "Continuation page is narrower than the answer");
        pages += 1;
        pending = false;
    }
}

struct DeviceUi {
    pen: Arc<Mutex<Pen>>,
    marker: Arc<Mutex<Option<Rect>>>,
    cancellation: Arc<SmartRemarkableCancellation>,
}

#[async_trait]
impl Ui for DeviceUi {
    async fn draw(&mut self, svg: &str) -> Result<()> {
        ensure!(!self.cancellation.should_cancel(), "Answer drawing cancelled");
        tokio::task::block_in_place(|| self.pen.lock().map_err(|_| anyhow::anyhow!("Pen lock unavailable"))?.draw_svg_centerline(svg))
    }
    async fn next_page(&mut self, x: i32) -> Result<Rect> {
        ensure!(!self.cancellation.should_cancel(), "Answer drawing cancelled");
        crate::page_layout::prepare_continuation(x).await
    }
    fn marker(&mut self, rect: Option<Rect>) {
        if let Ok(mut slot) = self.marker.lock() {
            *slot = rect;
        }
    }
}

pub async fn draw_complete_answer(
    lines: &[String],
    drawings: &[crate::illustration::Illustration],
    rect: Rect,
    pen: Arc<Mutex<Pen>>,
    marker: Arc<Mutex<Option<Rect>>>,
    cancellation: Arc<SmartRemarkableCancellation>,
) -> Result<()> {
    let pending = marker.lock().ok().and_then(|slot| *slot) == Some(rect);
    deliver(&mut DeviceUi { pen, marker, cancellation }, lines, drawings, rect, pending).await
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeUi {
        drawings: Vec<String>,
        pages: usize,
        marker: Option<Rect>,
        fail_page: bool,
        fail_draw: Option<usize>,
    }
    #[async_trait]
    impl Ui for FakeUi {
        async fn draw(&mut self, svg: &str) -> Result<()> {
            ensure!(self.fail_draw != Some(self.drawings.len()), "Simulated pen failure");
            self.drawings.push(svg.to_string());
            Ok(())
        }
        async fn next_page(&mut self, _x: i32) -> Result<Rect> {
            self.pages += 1;
            ensure!(!self.fail_page, "Simulated page failure");
            Ok(Rect { x: 64, y: 71, w: 694, h: 899 })
        }
        fn marker(&mut self, rect: Option<Rect>) {
            self.marker = rect;
        }
    }
    fn ui() -> FakeUi {
        FakeUi {
            drawings: vec![],
            pages: 0,
            marker: None,
            fail_page: false,
            fail_draw: None,
        }
    }
    fn initial() -> Rect {
        Rect { x: 64, y: 750, w: 694, h: 200 }
    }

    #[tokio::test]
    async fn illustrations_follow_text_use_clear_space_and_never_replay() {
        let drawing: crate::illustration::Illustration = serde_json::from_value(serde_json::json!({
            "title":"A plotted curve", "strokes":[[{"x":40,"y":40},{"x":500,"y":300}]],"labels":[]
        }))
        .unwrap();
        let mut spacious = ui();
        deliver(
            &mut spacious,
            &["An explanation.".into()],
            &[drawing.clone()],
            Rect { x: 64, y: 71, w: 694, h: 899 },
            true,
        )
        .await
        .unwrap();
        assert_eq!(spacious.pages, 0);
        assert_eq!(spacious.drawings.iter().filter(|s| s.contains("A plotted curve")).count(), 1);
        let mut crowded = ui();
        deliver(&mut crowded, &["An explanation.".into()], &[drawing.clone()], initial(), true)
            .await
            .unwrap();
        assert_eq!(crowded.pages, 1);
        assert_eq!(crowded.drawings.iter().filter(|s| s.contains("A plotted curve")).count(), 1);
        let mut failed = ui();
        failed.fail_page = true;
        assert!(deliver(&mut failed, &["An explanation.".into()], &[drawing], initial(), true).await.is_err());
        assert!(!failed.drawings.iter().any(|s| s.contains("A plotted curve")));
    }

    #[tokio::test]
    async fn every_line_is_drawn_once_across_multiple_pages_at_full_size() {
        let mut ui = ui();
        let lines: Vec<_> = (0..65).map(|i| format!("UNIQUE-LINE-{i:03}")).collect();
        deliver(&mut ui, &lines, &[], initial(), true).await.unwrap();
        assert_eq!(ui.pages, 3);
        for line in lines {
            assert_eq!(ui.drawings.iter().filter(|svg| svg.contains(&line)).count(), 1);
        }
        for svg in ui.drawings.iter().filter(|svg| svg.contains("UNIQUE-LINE")) {
            assert!(svg.contains("scale(1 1)"), "Text must stay full size");
        }
        assert!(ui.marker.is_none());
    }

    #[tokio::test]
    async fn no_page_is_added_for_a_short_answer_and_failed_edits_are_not_replayed() {
        let mut short = ui();
        deliver(&mut short, &["Yes.".into()], &[], initial(), true).await.unwrap();
        assert_eq!(short.pages, 0);
        let mut failed = ui();
        failed.fail_page = true;
        assert!(deliver(&mut failed, &vec!["A line".into(); 30], &[], initial(), true).await.is_err());
        assert_eq!(failed.pages, 1);
        assert!(failed.marker.is_none()); // old page coordinates are no longer trusted
        let mut failed = ui();
        failed.fail_draw = Some(2);
        assert!(deliver(&mut failed, &vec!["A line".into(); 30], &[], initial(), true).await.is_err());
        assert_eq!(failed.drawings.len(), 2);
        assert_eq!(failed.pages, 0);
        assert!(failed.marker.is_some());
    }

    #[test]
    fn wraps_long_words_and_unicode_without_losing_text() {
        let input = vec![
            "A long explanation\nwith another sentence.".into(),
            "abcdefghijklmnopqrstuv".into(),
            "ééééééééééééééééé".into(),
        ];
        let wrapped = wrap_lines(&input, 12).unwrap();
        assert!(wrapped.iter().all(|line| line.chars().count() <= 12));
        let compact = |s: String| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(compact(wrapped.join("")), compact(input.join("")));
    }
}
