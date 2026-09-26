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
    // The registered framebuffer excludes stride padding, moving the normalized
    // menu border from x=55 to x=53. Accept either capture geometry, but require
    // one consistent vertical edge rather than unrelated dark pixels nearby.
    (52..=56).any(|x| {
        [70, 130, 240, 330, 400, 470]
            .iter()
            .filter(|&&y| ss.get_pixel(x, y).map(|(r, g, b)| r.max(g).max(b) < 180).unwrap_or(false))
            .count()
            >= 5
    })
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
    async fn remember_tool(&mut self, _y: i32) -> Result<()> {
        Ok(())
    }
    async fn locate_tool(&mut self, y: i32) -> Result<i32> {
        Ok(y)
    }
}

struct DeviceUi {
    touch: Touch,
    rotated: bool,
    icon: Option<Vec<bool>>,
}

fn tool_icon(ss: &Screenshot, y: i32) -> Vec<bool> {
    let inverted = ss.get_pixel(5, y as u32).map(|(r, g, b)| r.max(g).max(b) < 128).unwrap_or(false);
    (y - 20..y + 20)
        .flat_map(|yy| (8..48).map(move |x| (x, yy)))
        .map(|(x, yy)| {
            let dark = ss.get_pixel(x, yy as u32).map(|(r, g, b)| r.max(g).max(b) < 128).unwrap_or(false);
            dark != inverted
        })
        .collect()
}

fn find_tool_icon(ss: &Screenshot, saved: &[bool]) -> Result<i32> {
    let count = saved.iter().filter(|&&p| p).count();
    ensure!(count >= 12, "Original tool icon was not recognizable");
    let mut scores: Vec<_> = (170..480)
        .map(|y| {
            let current = tool_icon(ss, y);
            let total = count + current.iter().filter(|&&p| p).count();
            let shared = saved.iter().zip(&current).filter(|(a, b)| **a && **b).count();
            (y, 2.0 * shared as f32 / total.max(1) as f32)
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (y, score) = scores[0];
    let other = scores.iter().filter(|(yy, _)| (yy - y).abs() > 8).map(|(_, score)| *score).fold(0.0, f32::max);
    ensure!(score >= 0.8 && score > other + 0.1, "Could not uniquely relocate the original tool icon");
    Ok(y)
}

impl DeviceUi {
    fn capture(&self) -> Result<Screenshot> {
        let mut ss = Screenshot::new()?;
        ss.take_screenshot_oriented(self.rotated)?;
        Ok(ss)
    }
}

#[async_trait]
impl Ui for DeviceUi {
    async fn read(&mut self) -> Result<State> {
        let ss = self.capture()?;
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
        self.touch.tap(point).await
    }

    async fn remember_tool(&mut self, y: i32) -> Result<()> {
        // The two pen slots stay fixed. Other tools shift when PDFs gain a
        // native note page (which includes a Text tool); retain their actual icon.
        if y >= 160 {
            self.icon = Some(tool_icon(&self.capture()?, y));
        }
        Ok(())
    }

    async fn locate_tool(&mut self, y: i32) -> Result<i32> {
        match &self.icon {
            Some(icon) => find_tool_icon(&self.capture()?, icon),
            None => Ok(y),
        }
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
        self.ui.remember_tool(self.previous_tool.unwrap()).await?;
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
            let tool_y = self.ui.locate_tool(tool_y).await?;
            self.previous_tool = Some(tool_y);
            if state.tool_y != Some(tool_y) {
                self.tap((28, tool_y)).await?;
            }
            ensure!(
                self.read().await?.tool_y.is_some_and(|y| (y - tool_y).abs() <= 2),
                "Could not restore active tool"
            );
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
    with_ui(
        DeviceUi {
            touch: Touch::new(false, TriggerCorner::UpperRight),
            rotated: crate::util::ui_rotated_180(),
            icon: None,
        },
        operation,
    )
    .await
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

    fn panel(settings: Settings, ambiguous: bool, border_x: u32) -> Screenshot {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in 54..570 {
            image.put_pixel(border_x, y, image::Rgb([0, 0, 0]));
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
            for border_x in [53, 55] {
                assert_eq!(read_settings(&panel(settings, false, border_x)), Some(settings));
                assert_eq!(read_settings(&panel(settings, true, border_x)), None);
            }
            assert_eq!(read_settings(&panel(settings, false, 60)), None);
        }
    }

    #[test]
    fn relocates_selected_tool_when_note_page_adds_a_toolbar_row() {
        let screen = |rows: &[u32], selected: bool| {
            let mut img = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
            for &row in rows {
                if selected {
                    for y in row - 26..row + 27 {
                        for x in 2..54 {
                            img.put_pixel(x, y, image::Rgb([0; 3]));
                        }
                    }
                }
                // An asymmetric icon with several distinct strokes.
                for y in row - 10..row + 11 {
                    for x in 18..38 {
                        if x == 18 || y == row - 10 || (y > row && x > 30) {
                            img.put_pixel(x, y, image::Rgb([if selected { 255 } else { 0 }; 3]));
                        }
                    }
                }
            }
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(img).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            Screenshot::from_png_data(bytes.into_inner())
        };
        let saved = tool_icon(&screen(&[240], true), 240);
        assert_eq!(find_tool_icon(&screen(&[294], false), &saved).unwrap(), 294);
        assert_eq!(find_tool_icon(&screen(&[240], false), &saved).unwrap(), 240);
        assert!(find_tool_icon(&screen(&[240, 347], false), &saved).is_err());
        assert!(find_tool_icon(&screen(&[], false), &saved).is_err());
    }
}
