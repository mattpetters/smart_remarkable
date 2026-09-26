//! Notebook-ink attribution and request status. Status is additive: pending
//! becomes checked or crossed without erasing any of the user's ink.
use anyhow::{ensure, Result};
use std::sync::{Arc, Mutex};

use crate::pen::Pen;
use crate::touch::{PenTool, Rect, Touch, TriggerCorner};
use crate::util::fit_answer_lines;

#[derive(Clone, Copy)]
pub enum AnswerStatus {
    Pending,
    Complete,
    Failed,
}

fn validate_rect(rect: Rect) -> Result<()> {
    ensure!(rect.x >= 0 && rect.y >= 0 && rect.w >= 60 && rect.h >= 64, "Answer box is too small");
    ensure!(rect.x + rect.w <= 768 && rect.y + rect.h <= 1024, "Answer box is outside the page");
    Ok(())
}

pub fn status_svg(rect: Rect, status: AnswerStatus) -> Result<String> {
    validate_rect(rect)?;
    let x = rect.x + 6;
    let y = rect.y + 4;
    let content = match status {
        AnswerStatus::Pending => format!(
            r#"<rect x="{x}" y="{y}" width="12" height="12" fill="none" stroke="black" stroke-width="1.6"/><text x="{}" y="{}" font-family="IBM Plex Mono" font-size="14" fill="black">AI</text>"#,
            x + 20,
            y + 13
        ),
        AnswerStatus::Complete => format!(
            r#"<path d="M {} {} L {} {} L {} {}" fill="none" stroke="black" stroke-width="1.8"/>"#,
            x + 2,
            y + 6,
            x + 5,
            y + 9,
            x + 10,
            y + 2
        ),
        AnswerStatus::Failed => format!(
            r#"<path d="M {} {} L {} {} M {} {} L {} {}" fill="none" stroke="black" stroke-width="1.8"/>"#,
            x + 3,
            y + 3,
            x + 9,
            y + 9,
            x + 9,
            y + 3,
            x + 3,
            y + 9
        ),
    };
    Ok(format!(r#"<svg width="768" height="1024" xmlns="http://www.w3.org/2000/svg">{content}</svg>"#))
}

/// Reserve a header and left margin, keep short answers at a consistent size,
/// and return the margin marker followed by individual answer-line fragments.
pub fn answer_svgs(lines: &[String], rect: Rect) -> Result<Vec<String>> {
    validate_rect(rect)?;
    ensure!(!lines.is_empty() && lines.iter().any(|line| !line.trim().is_empty()), "Answer is empty");
    let body = Rect {
        x: rect.x + 22,
        y: rect.y + 28,
        w: rect.w - 28,
        h: rect.h - 34,
    };
    let (mut fragments, height) = fit_answer_lines(lines, body, 1.0, false)?;
    ensure!(!fragments.is_empty(), "Answer has no drawable text");
    let x = rect.x + 6;
    let bottom = body.y as f32 + height + 3.0;
    let rule = format!(
        r#"<svg width="768" height="1024" xmlns="http://www.w3.org/2000/svg"><path d="M {} {} H {x} V {bottom} H {}" fill="none" stroke="black" stroke-width="1.6"/></svg>"#,
        x + 7,
        body.y,
        x + 7
    );
    fragments.insert(0, rule);
    Ok(fragments)
}

pub async fn draw_status(pen: Arc<Mutex<Pen>>, rect: Rect, status: AnswerStatus) -> Result<()> {
    let svg = status_svg(rect, status)?;
    // The trigger task holds the shared Touch lock while waiting; use a separate
    // input writer, as the normal answer renderer does.
    let mut touch = Touch::new(false, TriggerCorner::UpperRight);
    let previous = touch.switch_to_tool(PenTool::Ballpoint).await?;
    let result = tokio::task::block_in_place(|| pen.lock().map_err(|_| anyhow::anyhow!("Pen lock unavailable"))?.draw_svg_centerline(&svg));
    if previous != PenTool::Unknown {
        touch.restore_tool(previous).await?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::svg_to_bitmap;

    fn bounds(svg: &str) -> (i32, i32, i32, i32) {
        let bitmap = svg_to_bitmap(svg, 768, 1024).unwrap();
        let mut bbox = (768, 1024, 0, 0);
        for (y, row) in bitmap.iter().enumerate() {
            for (x, ink) in row.iter().enumerate() {
                if *ink {
                    bbox.0 = bbox.0.min(x as i32);
                    bbox.1 = bbox.1.min(y as i32);
                    bbox.2 = bbox.2.max(x as i32);
                    bbox.3 = bbox.3.max(y as i32);
                }
            }
        }
        assert!(bbox.0 <= bbox.2);
        bbox
    }

    #[test]
    fn marker_and_answer_stay_inside_box_without_touching_each_other() {
        let rect = Rect { x: 60, y: 330, w: 380, h: 520 };
        for count in [1, 8, 16] {
            let lines = vec!["A useful short answer".to_string(); count];
            let answer = answer_svgs(&lines, rect).unwrap();
            assert_eq!(answer.len(), count + 1);
            for svg in &answer {
                let (x1, y1, x2, y2) = bounds(svg);
                assert!(x1 >= rect.x && x2 <= rect.x + rect.w);
                assert!(y1 >= rect.y + 27 && y2 <= rect.y + rect.h);
            }
            let rule = bounds(&answer[0]);
            let first_line = bounds(&answer[1]);
            assert!(rule.2 < first_line.0);
            assert!(bounds(&status_svg(rect, AnswerStatus::Pending).unwrap()).3 < first_line.1);
        }
    }

    #[test]
    fn short_answer_does_not_fill_page_and_status_only_updates_checkbox() {
        let rect = Rect { x: 60, y: 300, w: 600, h: 680 };
        let answer = answer_svgs(&["Yes.".to_string()], rect).unwrap();
        assert!(bounds(&answer[0]).3 < rect.y + 65);
        for state in [AnswerStatus::Complete, AnswerStatus::Failed] {
            let (x1, y1, x2, y2) = bounds(&status_svg(rect, state).unwrap());
            assert!(x1 >= rect.x + 6 && x2 <= rect.x + 18);
            assert!(y1 >= rect.y + 4 && y2 <= rect.y + 16);
        }
    }

    #[test]
    fn rejects_empty_answers_and_invalid_placement() {
        let rect = Rect { x: 10, y: 10, w: 400, h: 400 };
        assert!(answer_svgs(&[], rect).is_err());
        assert!(answer_svgs(&["  ".to_string()], rect).is_err());
        for rect in [Rect { w: 40, ..rect }, Rect { y: 900, ..rect }] {
            assert!(status_svg(rect, AnswerStatus::Pending).is_err());
        }
    }
}
