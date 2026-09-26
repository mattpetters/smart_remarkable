//! A temporary red-ballpoint profile, with the user's pen and toolbar restored.
//! Coordinates are verified on Paper Pro firmware 3.27 in normalized page space.
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use std::future::Future;

use crate::device::DeviceModel;
use crate::screenshot::Screenshot;
use crate::touch::{Touch, TriggerCorner};

const TOGGLE: (i32, i32) = (28, 28);
const PEN: (i32, i32) = (28, 80);
const TYPES: [(i32, i32); 9] = [
    (96, 119),
    (149, 119),
    (201, 119),
    (96, 171),
    (149, 171),
    (201, 171),
    (96, 225),
    (149, 225),
    (201, 225),
];
const SIZES: [(i32, i32); 3] = [(96, 331), (149, 331), (201, 331)];
const COLORS: [(i32, i32); 9] = [
    (96, 407),
    (149, 407),
    (201, 407),
    (96, 461),
    (149, 461),
    (201, 461),
    (96, 515),
    (149, 515),
    (201, 515),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settings {
    kind: usize,
    size: usize,
    color: usize,
}

// Keep the medium stroke used by the validated compact answer layout.
const AI: Settings = Settings { kind: 0, size: 1, color: 4 };

#[derive(Clone, Copy, Debug)]
struct State {
    toolbar: bool,
    tool_y: Option<i32>,
    popover: bool,
    settings: Option<Settings>,
}

fn selected(ss: &Screenshot, cells: &[(i32, i32)], color: bool) -> Option<usize> {
    let mut matches = cells.iter().enumerate().filter_map(|(index, &(x, y))| {
        let dy = if color { [-12, 20] } else { [-20, 20] };
        let count = [-20, 20]
            .iter()
            .flat_map(|dx| dy.iter().map(move |dy| (x + dx, y + dy)))
            .filter(|&(x, y)| ss.get_pixel(x as u32, y as u32).map(|(r, g, b)| r.max(g).max(b) < 90).unwrap_or(false))
            .count();
        (count >= 3).then_some(index)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn has_popover(ss: &Screenshot) -> bool {
    // The left border distinguishes this popover from page ink in the same area.
    let border = [70, 130, 240, 330, 400, 470]
        .iter()
        .filter(|&&y| ss.get_pixel(55, y).map(|(r, g, b)| r.max(g).max(b) < 180).unwrap_or(false))
        .count();
    border >= 5
}

fn read_settings(ss: &Screenshot) -> Option<Settings> {
    if !has_popover(ss) {
        return None;
    }
    Some(Settings {
        kind: selected(ss, &TYPES, false)?,
        size: selected(ss, &SIZES, false)?,
        color: selected(ss, &COLORS, true)?,
    })
}

#[async_trait]
trait Ui: Send {
    async fn read(&mut self) -> Result<State>;
    async fn tap(&mut self, point: (i32, i32)) -> Result<()>;
}

struct DeviceUi(Touch);

#[async_trait]
impl Ui for DeviceUi {
    async fn read(&mut self) -> Result<State> {
        let mut ss = Screenshot::new()?;
        ss.take_screenshot()?;
        let toolbar = Touch::screenshot_palette_open(&ss);
        // Preserve the detected selection's position generically, including
        // tools this client does not name. Only the temporary pen slot needs
        // a known identity; every other selection is restored to its own row.
        let tool_y = if toolbar {
            Touch::screenshot_selected_tool_y(&ss).map(|y| if (y - 80).abs() < 20 { 80 } else { y })
        } else {
            None
        };
        let settings = if tool_y == Some(80) { read_settings(&ss) } else { None };
        Ok(State {
            toolbar,
            tool_y,
            popover: toolbar && has_popover(&ss),
            settings,
        })
    }

    async fn tap(&mut self, point: (i32, i32)) -> Result<()> {
        self.0.tap(point).await
    }
}

struct Session<U: Ui> {
    ui: U,
    initial: State,
    cached: Option<State>,
    previous_tool: Option<i32>,
    original: Option<Settings>,
    ballpoint: Option<Settings>,
}

impl<U: Ui> Session<U> {
    async fn read(&mut self) -> Result<State> {
        if let Some(state) = self.cached {
            return Ok(state);
        }
        let state = self.ui.read().await?;
        self.cached = Some(state);
        Ok(state)
    }

    async fn tap(&mut self, point: (i32, i32)) -> Result<()> {
        self.cached = None;
        self.ui.tap(point).await
    }

    async fn panel(&mut self) -> Result<Settings> {
        let mut state = self.read().await?;
        if !state.toolbar {
            self.tap(TOGGLE).await?;
            state = self.read().await?;
        }
        ensure!(state.toolbar, "Could not reveal pen toolbar");
        if state.tool_y != Some(80) {
            ensure!(state.tool_y.is_some(), "Could not identify active tool");
            self.tap(PEN).await?;
            state = self.read().await?;
        }
        ensure!(state.tool_y == Some(80), "Could not select first pen slot");
        if state.settings.is_none() {
            ensure!(!state.popover, "Pen settings are open but could not be read safely");
            self.tap(PEN).await?;
            state = self.read().await?;
        }
        state.settings.context("Unrecognized pen settings; leaving profile unchanged")
    }

    async fn set(&mut self, target: Settings) -> Result<()> {
        let mut current = self.panel().await?;
        if current.kind != target.kind {
            self.tap(TYPES[target.kind]).await?;
            // Selecting a pen type can dismiss the popover. Reopen only if needed.
            current = self.panel().await?;
            ensure!(current.kind == target.kind, "Pen type did not change as expected");
        }
        if current.size != target.size {
            self.tap(SIZES[target.size]).await?;
        }
        if current.color != target.color {
            self.tap(COLORS[target.color]).await?;
        }
        ensure!(self.read().await?.settings == Some(target), "Pen settings verification failed");
        Ok(())
    }

    async fn close_panel(&mut self) -> Result<()> {
        let state = self.read().await?;
        if state.popover && state.tool_y == Some(80) {
            self.tap(PEN).await?;
            ensure!(!self.read().await?.popover, "Pen settings menu is still open");
        }
        Ok(())
    }

    async fn prepare(&mut self) -> Result<()> {
        if !self.initial.toolbar {
            self.tap(TOGGLE).await?;
        }
        let visible = self.read().await?;
        self.previous_tool = visible.tool_y;
        ensure!(self.previous_tool.is_some(), "Cannot preserve an unidentified active tool");
        let original = self.panel().await?;
        self.original = Some(original);
        if original.kind != AI.kind {
            self.tap(TYPES[AI.kind]).await?;
        }
        let ballpoint = self.panel().await?;
        ensure!(ballpoint.kind == AI.kind, "Actual Ballpoint pen was not selected");
        self.ballpoint = Some(ballpoint);
        self.set(AI).await?;
        self.close_panel().await?;
        if !self.initial.toolbar {
            self.tap(TOGGLE).await?;
        }
        log::info!("Temporary answer pen: Ballpoint, red, medium; original settings captured");
        Ok(())
    }

    async fn restore(&mut self) -> Result<()> {
        self.cached = None;
        // Restore Ballpoint's own saved profile too; changing pen type may load
        // independent color/size preferences rather than sharing the slot's values.
        if let Some(ballpoint) = self.ballpoint {
            self.set(ballpoint).await?;
        }
        if let Some(original) = self.original {
            self.set(original).await?;
        }
        self.close_panel().await?;
        if let Some(tool_y) = self.previous_tool {
            let mut state = self.read().await?;
            if !state.toolbar {
                self.tap(TOGGLE).await?;
                state = self.read().await?;
            }
            if state.tool_y != Some(tool_y) {
                self.tap((28, tool_y)).await?;
            }
            ensure!(self.read().await?.tool_y == Some(tool_y), "Could not restore active tool");
        }
        if self.initial.popover && !self.read().await?.popover {
            if let Some(y) = self.previous_tool {
                self.tap((28, y)).await?;
            }
        }
        if !self.initial.toolbar && self.read().await?.toolbar {
            self.tap(TOGGLE).await?;
        }
        let final_state = self.read().await?;
        ensure!(final_state.toolbar == self.initial.toolbar, "Could not restore toolbar visibility");
        ensure!(final_state.popover == self.initial.popover, "Could not restore pen popover visibility");
        log::info!("Original pen profiles and active tool restored");
        Ok(())
    }
}

async fn with_ui<U, F, Fut, T>(mut ui: U, operation: F) -> Result<T>
where
    U: Ui,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let initial = ui.read().await?;
    let mut session = Session {
        ui,
        initial,
        cached: Some(initial),
        previous_tool: initial.tool_y,
        original: None,
        ballpoint: None,
    };
    let outcome = match session.prepare().await {
        Ok(()) => operation().await,
        Err(error) => Err(error.context("Could not prepare temporary answer pen")),
    };
    let restored = session.restore().await;
    match (outcome, restored) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(error.context("Answer completed, but restoring pen settings failed")),
        (Err(error), Err(restore)) => Err(anyhow::anyhow!("{error:#}; restoring pen settings also failed: {restore:#}")),
    }
}

/// Restore profiles after either a successful answer or an ordinary request,
/// rendering, or cancellation error. Process termination cannot run UI cleanup.
pub async fn with_red_ballpoint<F, Fut, T>(operation: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    ensure!(
        matches!(DeviceModel::detect(), DeviceModel::RemarkablePaperPro),
        "Temporary answer pen currently supports Paper Pro only"
    );
    with_ui(DeviceUi(Touch::new(false, TriggerCorner::UpperRight)), operation).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq)]
    struct World {
        toolbar: bool,
        tool: i32,
        menu: bool,
        kind: usize,
        profiles: [Settings; 9],
    }

    struct FakeUi {
        world: Arc<Mutex<World>>,
        fail_red_once: bool,
        dismiss_on_type: bool,
    }

    #[async_trait]
    impl Ui for FakeUi {
        async fn read(&mut self) -> Result<State> {
            let w = self.world.lock().unwrap();
            Ok(State {
                toolbar: w.toolbar,
                tool_y: w.toolbar.then_some(w.tool),
                popover: w.toolbar && w.menu,
                settings: (w.toolbar && w.menu && w.tool == 80).then_some(w.profiles[w.kind]),
            })
        }

        async fn tap(&mut self, point: (i32, i32)) -> Result<()> {
            let mut w = self.world.lock().unwrap();
            if point == TOGGLE {
                w.toolbar = !w.toolbar;
                w.menu = false;
            } else if point.0 == 28 {
                ensure!(w.toolbar, "Toolbar is hidden");
                w.menu = point == PEN && w.tool == 80 && !w.menu;
                w.tool = point.1;
            } else {
                ensure!(w.menu && w.tool == 80, "Control tapped with no pen popover");
                if let Some(index) = TYPES.iter().position(|p| *p == point) {
                    w.kind = index;
                    if self.dismiss_on_type {
                        w.menu = false;
                    }
                } else if let Some(index) = SIZES.iter().position(|p| *p == point) {
                    let kind = w.kind;
                    w.profiles[kind].size = index;
                } else if let Some(index) = COLORS.iter().position(|p| *p == point) {
                    if self.fail_red_once && index == AI.color {
                        self.fail_red_once = false;
                        anyhow::bail!("Simulated color-control failure");
                    }
                    let kind = w.kind;
                    w.profiles[kind].color = index;
                } else {
                    anyhow::bail!("Unexpected control");
                }
            }
            Ok(())
        }
    }

    fn world(tool: i32, toolbar: bool) -> World {
        World {
            toolbar,
            tool,
            menu: false,
            kind: 5,
            profiles: std::array::from_fn(|kind| Settings {
                kind,
                size: kind % 3,
                color: (kind + 1) % 9,
            }),
        }
    }

    #[tokio::test]
    async fn restores_generic_selection_and_both_profiles_on_success_or_failure() {
        // Includes a tool row the client does not name, plus a hidden toolbar.
        for tool in [80, 130, 187, 240, 295, 347] {
            for toolbar in [false, true] {
                for fail in [false, true] {
                    let initial = world(tool, toolbar);
                    let shared = Arc::new(Mutex::new(initial.clone()));
                    let check = Arc::clone(&shared);
                    let ui = FakeUi {
                        world: Arc::clone(&shared),
                        fail_red_once: false,
                        dismiss_on_type: fail,
                    };
                    let result = with_ui(ui, || async move {
                        {
                            let w = check.lock().unwrap();
                            assert_eq!(w.profiles[w.kind], AI);
                            assert_eq!(w.tool, 80);
                            assert!(!w.menu);
                            assert_eq!(w.toolbar, toolbar);
                        }
                        if fail {
                            anyhow::bail!("Simulated request failure");
                        }
                        Ok(())
                    })
                    .await;
                    assert_eq!(result.is_err(), fail);
                    assert_eq!(*shared.lock().unwrap(), initial);
                }
            }
        }
    }

    #[tokio::test]
    async fn rolls_back_partial_setup_without_running_the_request() {
        let initial = world(130, false);
        let shared = Arc::new(Mutex::new(initial.clone()));
        let ui = FakeUi {
            world: Arc::clone(&shared),
            fail_red_once: true,
            dismiss_on_type: true,
        };
        let result = with_ui(ui, || async {
            panic!("Must not submit or draw after setup failure");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await;
        assert!(result.is_err());
        assert_eq!(*shared.lock().unwrap(), initial);
    }

    fn panel(settings: Settings, ambiguous: bool) -> Screenshot {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in 54..570 {
            image.put_pixel(55, y, image::Rgb([0, 0, 0]));
        }
        let mut cells = vec![TYPES[settings.kind], SIZES[settings.size], COLORS[settings.color]];
        if ambiguous {
            cells.push(TYPES[(settings.kind + 1) % 9]);
        }
        for (x, y) in cells {
            for yy in y - 23..y + 24 {
                for xx in x - 23..x + 24 {
                    image.put_pixel(xx as u32, yy as u32, image::Rgb([0, 0, 0]));
                }
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Screenshot::from_png_data(bytes.into_inner())
    }

    #[test]
    fn reads_profile_selection_and_rejects_ambiguous_panels() {
        for index in 0..9 {
            let settings = Settings {
                kind: index,
                size: index % 3,
                color: 8 - index,
            };
            assert_eq!(read_settings(&panel(settings, false)), Some(settings));
            assert_eq!(read_settings(&panel(settings, true)), None);
        }
    }
}
