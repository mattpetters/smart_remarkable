//! Append answers after visible page ink; never fall back above the question.
use crate::screenshot::Screenshot;
use crate::touch::{Rect, Touch, TriggerCorner};
use anyhow::{ensure, Result};
use async_trait::async_trait;

const WIDTH: usize = 768;
const HEIGHT: usize = 1024;
// Keep clear of the clipboard banner and bottom navigation chrome.
const PAGE_BOTTOM: usize = 980;

fn ink_mask(screen: &Screenshot) -> Vec<bool> {
    let is_move = crate::device::DeviceModel::detect() == crate::device::DeviceModel::RemarkablePaperProMove;
    let top = if is_move { 80 } else { 55 };
    let left = if !is_move && Touch::screenshot_palette_open(screen) { 60 } else { 10 };
    let right = WIDTH - 10;
    let mut ink = vec![false; WIDTH * HEIGHT];
    for y in top..PAGE_BOTTOM {
        for x in left..right {
            ink[y * WIDTH + x] = screen
                .get_pixel(x as u32, y as u32)
                .map(|(r, g, b)| {
                    let lo = r.min(g).min(b);
                    let hi = r.max(g).max(b);
                    hi < 220 || (hi - lo > 50 && lo < 220)
                })
                .unwrap_or(false);
        }
    }

    // Remove long continuous rules/grid lines. Dotted templates are rejected
    // later as tiny components. Text crossing a rule remains on adjacent rows.
    let mut rules = vec![false; WIDTH * HEIGHT];
    for y in top..PAGE_BOTTOM {
        let mut start = left;
        while start < right {
            if !ink[y * WIDTH + start] {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < right && ink[y * WIDTH + end] {
                end += 1;
            }
            if end - start > (right - left) * 3 / 5 {
                rules[y * WIDTH + start..y * WIDTH + end].fill(true);
            }
            start = end;
        }
    }
    for x in left..right {
        let mut start = top;
        while start < PAGE_BOTTOM {
            if !ink[start * WIDTH + x] {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < PAGE_BOTTOM && ink[end * WIDTH + x] {
                end += 1;
            }
            if end - start > HEIGHT * 3 / 4 {
                for y in start..end {
                    rules[y * WIDTH + x] = true;
                }
            }
            start = end;
        }
    }

    // Detect both axes before erasing either: horizontal removal would break
    // vertical grid lines into short segments that look like handwriting.
    for (pixel, rule) in ink.iter_mut().zip(rules) {
        *pixel &= !rule;
    }

    let mut cleaned = vec![false; WIDTH * HEIGHT];
    let mut stack = Vec::new();
    let mut component = Vec::new();
    for y in top..PAGE_BOTTOM {
        for x in left..right {
            let seed = y * WIDTH + x;
            if !ink[seed] {
                continue;
            }
            ink[seed] = false;
            stack.push((x, y));
            component.clear();
            let (mut top, mut last) = (y, y);
            while let Some((x, y)) = stack.pop() {
                component.push(y * WIDTH + x);
                top = top.min(y);
                last = last.max(y);
                for yy in y.saturating_sub(1)..=(y + 1).min(PAGE_BOTTOM - 1) {
                    for xx in x.saturating_sub(1).max(left)..=(x + 1).min(right - 1) {
                        let index = yy * WIDTH + xx;
                        if ink[index] {
                            ink[index] = false;
                            stack.push((xx, yy));
                        }
                    }
                }
            }
            let min_x = component.iter().map(|index| index % WIDTH).min().unwrap();
            let max_x = component.iter().map(|index| index % WIDTH).max().unwrap();
            let template_dot = component.len() <= 9 && last - top <= 2 && max_x - min_x <= 2;
            // Native continuous-page scrollbar: its narrow fixed gutter can
            // extend to y=907 even when the writing ends near the page top.
            let scrollbar = min_x >= 732 && max_x <= 742 && last - top >= 40;
            if component.len() >= 6 && last - top >= 2 && !template_dot && !scrollbar {
                for &index in &component {
                    cleaned[index] = true;
                }
            }
        }
    }
    cleaned
}

fn bottom(mask: &[bool]) -> Option<i32> {
    mask.iter().rposition(|&ink| ink).map(|index| (index / WIDTH) as i32)
}

pub fn append_rect(screen: &Screenshot, selection: Rect) -> Result<Rect> {
    rect_after(bottom(&ink_mask(screen)).unwrap_or(55).max(selection.y + selection.h), selection.x)
}

fn rect_after(last: i32, x: i32) -> Result<Rect> {
    let top = if crate::device::DeviceModel::detect() == crate::device::DeviceModel::RemarkablePaperProMove { 80 } else { 55 };
    let y = last.max(top) + 16;
    let h = PAGE_BOTTOM as i32 - 10 - y;
    ensure!(
        h >= 96,
        "No clear space below the page's writing; reveal blank space below it before asking again"
    );
    let x = x.clamp(64, WIDTH as i32 - 310);
    Ok(Rect {
        x,
        y,
        w: WIDTH as i32 - 10 - x,
        h,
    })
}

// Prefer room for a useful reply; a stationary viewport can still provide a
// shorter answer with the same font size and a tighter model line budget.
const ANSWER_HEIGHT: i32 = 34 + 10 * 31;
const MIN_ANSWER_HEIGHT: i32 = 34 + 4 * 31;
const MAX_SCROLLS: usize = 4;

/// Register pre/post-scroll ink. A stalled swipe or changed page must never be
/// treated as empty space. Ignore screen chrome and use template-filtered ink.
fn upward_shift(before: &[bool], after: &[bool]) -> Option<i32> {
    // A second pan can start with all useful handwriting above mid-screen.
    // Restricting anchors to the bottom half then tracks only transient footer
    // chrome and rejects a real pan (e.g. 302px followed by a 198px boundary pan).
    let points: Vec<_> = (56..PAGE_BOTTOM)
        .flat_map(|y| (70..740).map(move |x| (x, y)))
        .filter(|&(x, y)| before[y * WIDTH + x])
        .collect();
    if points.len() < 16 {
        return None;
    }
    let samples: Vec<_> = points.iter().step_by((points.len() / 256).max(1)).copied().collect();
    let score = |dy: usize| -> f32 {
        // Some old ink can leave the viewport. Register only the overlapping
        // portion, but require enough anchors to reject a coincidental match.
        let visible: Vec<_> = samples.iter().filter(|&&(_, y)| y >= dy + 56).collect();
        if visible.len() < 16 || visible.len() * 4 < samples.len() {
            return 0.0;
        }
        let matches = visible
            .iter()
            .filter(|&&&(x, y)| {
                let yy = y - dy;
                (yy - 1..=yy + 1).any(|y| (x - 1..=x + 1).any(|x| after[y * WIDTH + x]))
            })
            .count();
        matches as f32 / visible.len() as f32
    };
    let stationary = score(0);
    let (shift, confidence) = (8..=600).map(|dy| (dy, score(dy))).max_by(|a, b| a.1.total_cmp(&b.1))?;
    log::info!("Append motion: shift={} confidence={:.2} stationary={:.2}", shift, confidence, stationary);
    (confidence >= 0.75 && confidence > stationary + 0.20).then_some(shift as i32)
}

#[async_trait]
trait PageUi: Send {
    async fn capture(&mut self) -> Result<Screenshot>;
    async fn scroll(&mut self) -> Result<()>;
    /// Insert after the current page and return only once its canvas is ready.
    async fn add_note_page(&mut self) -> Result<Screenshot>;
}

struct DevicePage(Touch);
#[async_trait]
impl PageUi for DevicePage {
    async fn capture(&mut self) -> Result<Screenshot> {
        let mut screen = Screenshot::new()?;
        screen.take_screenshot()?;
        Ok(screen)
    }
    async fn scroll(&mut self) -> Result<()> {
        self.0.scroll_page_down().await
    }
    async fn add_note_page(&mut self) -> Result<Screenshot> {
        crate::note_page::insert_after_current(&mut self.0).await
    }
}

async fn continue_on_note_page(ui: &mut impl PageUi, x: i32) -> Result<Rect> {
    let screen = ui.add_note_page().await?;
    // A late frame, retained template, or unexpected content must not become
    // permission to draw over another page. Page insertion is attempted once.
    let rect = rect_after(bottom(&ink_mask(&screen)).unwrap_or(55), x)?;
    ensure!(rect.h >= ANSWER_HEIGHT, "New note page has no verified clear answer space");
    log::info!("Append layout: continuing on a new note page");
    Ok(rect)
}

async fn prepare_with(ui: &mut impl PageUi, selection: Rect) -> Result<Rect> {
    let mut mask = ink_mask(&ui.capture().await?);
    let mut selection_bottom = selection.y + selection.h;
    for attempt in 0..=MAX_SCROLLS {
        let last = bottom(&mask).unwrap_or(55).max(selection_bottom);
        log::info!(
            "Append scan: ink bottom={:?}, selection floor={}, pass={}",
            bottom(&mask),
            selection_bottom,
            attempt
        );
        if let Ok(rect) = rect_after(last, selection.x) {
            if rect.h >= ANSWER_HEIGHT {
                return Ok(rect);
            }
        }
        if attempt == MAX_SCROLLS {
            return continue_on_note_page(ui, selection.x).await;
        }
        ui.scroll().await?;
        let after = ink_mask(&ui.capture().await?);
        let shift = match upward_shift(&mask, &after) {
            Some(shift) => shift,
            None => {
                // An unchanged viewport is not a failed request when there is
                // already clear room for a concise answer. Never accept an
                // unrelated/blank frame as evidence of free space.
                let count = mask.iter().filter(|&&p| p).count() + after.iter().filter(|&&p| p).count();
                let same = mask.iter().zip(&after).filter(|(a, b)| **a && **b).count() * 2;
                if count >= 32 && same as f32 / count as f32 >= 0.85 {
                    if let Ok(rect) = rect_after(bottom(&after).unwrap_or(55).max(selection_bottom), selection.x) {
                        if rect.h >= MIN_ANSWER_HEIGHT {
                            log::info!("Append layout: scroll stopped; using {} px of verified clear space", rect.h);
                            return Ok(rect);
                        }
                    }
                    return continue_on_note_page(ui, selection.x).await;
                }
                anyhow::bail!("Could not verify page scrolling; no answer was drawn");
            }
        };
        selection_bottom = (selection_bottom - shift).max(55);
        log::info!("Append layout: verified upward page motion of {} px", shift);
        mask = after;
    }
    unreachable!()
}

/// Called after capturing question/context and selecting the answer pen, which
/// dismisses the lasso overlay. Scroll first, then insert a note page if needed.
/// The caller retains the original question and context images throughout.
pub async fn prepare_append(selection: Rect) -> Result<Rect> {
    prepare_with(&mut DevicePage(Touch::new(false, TriggerCorner::FourFinger)), selection).await
}

pub async fn prepare_continuation(x: i32) -> Result<Rect> {
    continue_on_note_page(&mut DevicePage(Touch::new(false, TriggerCorner::FourFinger)), x).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn viewport(offset: u32, rows: &[u32]) -> Screenshot {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in (75..1024).step_by(25) {
            for x in 0..768 {
                image.put_pixel(x, y, image::Rgb([100, 100, 100]));
            }
        }
        for (i, &row) in rows.iter().enumerate() {
            for dy in 0..20 {
                for dx in 0..85 {
                    if dx % 17 < 4 || dy < 3 && dx % 17 < 11 || dy == 11 && dx % 17 < 9 {
                        if let Some(y) = (row + dy).checked_sub(offset).filter(|&y| y < 1024) {
                            image.put_pixel(140 + i as u32 * 97 + dx, y, image::Rgb([0, 0, 180]));
                        }
                    }
                }
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Screenshot::from_png_data(bytes.into_inner())
    }

    struct FakePage {
        frames: VecDeque<Screenshot>,
        scrolls: usize,
        inserted: usize,
    }
    #[async_trait]
    impl PageUi for FakePage {
        async fn capture(&mut self) -> Result<Screenshot> {
            Ok(self.frames.pop_front().expect("unexpected capture"))
        }
        async fn scroll(&mut self) -> Result<()> {
            self.scrolls += 1;
            Ok(())
        }
        async fn add_note_page(&mut self) -> Result<Screenshot> {
            self.inserted += 1;
            self.capture().await
        }
    }

    #[tokio::test]
    async fn scrolls_past_revealed_ink_and_leaves_room_for_full_size_text() {
        let mut ui = FakePage {
            frames: [viewport(0, &[880, 1100]), viewport(420, &[880, 1100]), viewport(840, &[880, 1100])].into(),
            scrolls: 0,
            inserted: 0,
        };
        let rect = prepare_with(&mut ui, Rect { x: 100, y: 90, w: 200, h: 50 }).await.unwrap();
        assert_eq!(ui.scrolls, 2);
        assert_eq!(rect.y, 295); // lowest revealed ink ends at 279
        assert!(rect.h >= ANSWER_HEIGHT);
    }

    #[tokio::test]
    async fn does_not_scroll_when_append_space_is_already_clear() {
        let mut ui = FakePage {
            frames: [viewport(0, &[300])].into(),
            scrolls: 0,
            inserted: 0,
        };
        let rect = prepare_with(&mut ui, Rect { x: 100, y: 90, w: 200, h: 50 }).await.unwrap();
        assert_eq!(ui.scrolls, 0);
        assert_eq!(ui.inserted, 0);
        assert_eq!(rect.y, 335);
    }

    #[tokio::test]
    async fn stopped_scroll_uses_clear_room_for_a_shorter_answer() {
        let mut ui = FakePage {
            frames: [viewport(0, &[700]), viewport(0, &[700])].into(),
            scrolls: 0,
            inserted: 0,
        };
        let rect = prepare_with(&mut ui, Rect { x: 100, y: 90, w: 200, h: 50 }).await.unwrap();
        assert_eq!(rect.y, 735);
        assert_eq!(rect.h, 235);
        assert_eq!(ui.scrolls, 1);
        assert_eq!(ui.inserted, 0);
    }

    #[tokio::test]
    async fn inserts_one_note_page_when_a_verified_full_page_cannot_scroll() {
        let mut ui = FakePage {
            frames: [viewport(0, &[900]), viewport(0, &[900]), viewport(0, &[])].into(),
            scrolls: 0,
            inserted: 0,
        };
        let rect = prepare_with(&mut ui, Rect { x: 100, y: 900, w: 200, h: 60 }).await.unwrap();
        assert_eq!(ui.scrolls, 1);
        assert_eq!(ui.inserted, 1);
        assert_eq!(rect.y, 71); // old question coordinates belong to the old page
        assert!(rect.h >= ANSWER_HEIGHT);
    }

    #[tokio::test]
    async fn does_not_keep_inserting_if_the_new_canvas_is_not_clear() {
        let mut ui = FakePage {
            frames: [viewport(0, &[900]), viewport(0, &[900]), viewport(0, &[900])].into(),
            scrolls: 0,
            inserted: 0,
        };
        assert!(prepare_with(&mut ui, Rect { x: 100, y: 900, w: 200, h: 60 }).await.is_err());
        assert_eq!(ui.inserted, 1);
    }

    #[tokio::test]
    async fn refuses_unrelated_blank_capture_without_adding_a_page() {
        for after in [viewport(0, &[]), viewport(0, &[100, 600])] {
            let mut ui = FakePage {
                frames: [viewport(0, &[880]), after].into(),
                scrolls: 0,
                inserted: 0,
            };
            let error = prepare_with(&mut ui, Rect { x: 100, y: 850, w: 200, h: 60 }).await.unwrap_err();
            assert!(error.to_string().contains("verify page scrolling"));
            assert_eq!(ui.inserted, 0);
            assert_eq!(ui.scrolls, 1);
        }
    }

    #[tokio::test]
    async fn translates_the_selected_question_floor_when_scrolling() {
        let mut ui = FakePage {
            frames: [viewport(0, &[650]), viewport(420, &[650])].into(),
            scrolls: 0,
            inserted: 0,
        };
        let rect = prepare_with(&mut ui, Rect { x: 100, y: 645, w: 200, h: 40 }).await.unwrap();
        assert_eq!(ui.scrolls, 1);
        assert!(rect.y >= 280 && rect.y <= 283);
    }

    #[test]
    fn tracks_a_boundary_pan_after_handwriting_moves_above_mid_screen() {
        let before = ink_mask(&viewport(302, &[653]));
        let after = ink_mask(&viewport(500, &[653]));
        assert!((197..=199).contains(&upward_shift(&before, &after).unwrap()));
        assert_eq!(upward_shift(&before, &ink_mask(&viewport(0, &[]))), None);
    }

    #[test]
    fn ignores_native_scrollbar_without_ignoring_nearby_writing() {
        let mut img = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
        for (left, right, top, bottom) in [(736, 740, 120, 908), (710, 716, 200, 220)] {
            for y in top..bottom { for x in left..right { img.put_pixel(x, y, image::Rgb([0; 3])); } }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let mask = ink_mask(&Screenshot::from_png_data(bytes.into_inner()));
        assert_eq!(bottom(&mask), Some(219));
    }

    fn screen(dotted: bool, writing_bottom: u32) -> Screenshot {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in (75..1024).step_by(25) {
            for x in 0..768 {
                if !dotted || x % 4 == 0 {
                    image.put_pixel(x, y, image::Rgb([80, 80, 80]));
                }
            }
        }
        // Question at the top, earlier reply lower down (including colored ink).
        for (top, bottom, rgb) in [(100, 130, [0, 0, 0]), (writing_bottom - 20, writing_bottom, [0, 0, 255])] {
            for y in top..=bottom {
                for x in 120..125 {
                    image.put_pixel(x, y, image::Rgb(rgb));
                }
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Screenshot::from_png_data(bytes.into_inner())
    }

    #[test]
    fn appends_after_earlier_answers_not_after_the_selected_question() {
        let selection = Rect { x: 110, y: 90, w: 240, h: 50 };
        for dotted in [false, true] {
            let rect = append_rect(&screen(dotted, 712), selection).unwrap();
            assert_eq!(rect.y, 728);
            assert_eq!(rect.h, 242);
        }
    }

    #[test]
    fn refuses_a_full_page_instead_of_overwriting_the_middle_or_top() {
        let selection = Rect { x: 110, y: 90, w: 240, h: 50 };
        assert!(append_rect(&screen(false, 950), selection).is_err());
        let low_selection = Rect { y: 940, ..selection };
        assert!(append_rect(&screen(true, 200), low_selection).is_err());
    }

    #[test]
    fn ignores_both_axes_of_grid_but_keeps_faint_handwriting() {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in 0..1024 {
            for x in 0..768 {
                if x % 25 == 0 || y % 25 == 0 {
                    image.put_pixel(x, y, image::Rgb([190, 190, 190]));
                }
            }
        }
        for y in 630..=645 {
            for x in 138..143 {
                image.put_pixel(x, y, image::Rgb([210, 210, 210]));
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let rect = append_rect(&Screenshot::from_png_data(bytes.into_inner()), Rect { x: 100, y: 90, w: 100, h: 100 }).unwrap();
        assert_eq!(rect.y, 661);
    }

    #[test]
    fn ignores_three_pixel_template_dots_and_clipboard_footer() {
        let mut image = image::RgbImage::from_pixel(768, 1024, image::Rgb([255, 255, 255]));
        for y in (75..960).step_by(25) {
            for x in (70..750).step_by(8) {
                for dy in 0..3 {
                    for dx in 0..3 {
                        image.put_pixel(x + dx, y + dy, image::Rgb([50, 50, 50]));
                    }
                }
            }
        }
        for y in 989..1024 {
            for x in 55..764 {
                image.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        for y in 690..710 {
            for x in 140..145 {
                image.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let rect = append_rect(&Screenshot::from_png_data(bytes.into_inner()), Rect { x: 100, y: 90, w: 100, h: 100 }).unwrap();
        assert_eq!(rect.y, 725);
    }
}
