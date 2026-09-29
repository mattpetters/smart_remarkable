//! Small, bounded vector illustrations. No model-supplied SVG or resources.
use crate::touch::Rect;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const HEIGHT: i32 = 440;
pub const MIN_WIDTH: i32 = 628;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Illustration {
    pub title: String,
    pub strokes: Vec<Vec<Point>>,
    pub labels: Vec<Label>,
}

fn valid_text(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.chars().count() <= limit && text.chars().all(|c| c >= ' ' && c < '\u{2e80}' && c != '\u{7f}')
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
pub fn validate(drawings: &[Illustration]) -> Result<()> {
    ensure!(drawings.len() <= 2, "Too many illustrations");
    for d in drawings {
        ensure!(valid_text(&d.title, 44), "Invalid illustration title");
        ensure!(
            !d.strokes.is_empty() && d.strokes.len() <= 64 && d.labels.len() <= 24,
            "Illustration exceeds limits"
        );
        ensure!(d.strokes.iter().map(Vec::len).sum::<usize>() <= 1024, "Too many illustration points");
        for path in &d.strokes {
            ensure!((2..=128).contains(&path.len()), "Invalid illustration path");
            ensure!(
                path.iter().all(|p| (8.0..=592.0).contains(&p.x) && (8.0..=352.0).contains(&p.y)),
                "Illustration outside canvas"
            );
        }
        for l in &d.labels {
            ensure!(
                valid_text(&l.text, 40) && (8.0..=592.0).contains(&l.x) && (18.0..=352.0).contains(&l.y) && l.x + 10.8 * l.text.chars().count() as f32 <= 592.0,
                "Illustration label outside canvas"
            );
        }
    }
    Ok(())
}

pub fn svg(d: &Illustration, rect: Rect) -> Result<String> {
    validate(std::slice::from_ref(d))?;
    ensure!(
        rect.w >= MIN_WIDTH && rect.h >= HEIGHT && rect.x >= 0 && rect.y >= 0 && rect.x + rect.w <= 768 && rect.y + HEIGHT <= 1024,
        "Insufficient illustration space"
    );
    let mut body = format!(
        r#"<path d="M {} {} h -7 v 402 h 7" fill="none" stroke="black" stroke-width="1.6"/><text x="{}" y="{}" font-family="IBM Plex Mono" font-size="20">{}</text><g transform="translate({} {})" fill="none" stroke="black" stroke-width="1.5">"#,
        rect.x + 13,
        rect.y + 28,
        rect.x + 22,
        rect.y + 47,
        escape(&d.title),
        rect.x + 22,
        rect.y + 64
    );
    for path in &d.strokes {
        let points = path.iter().map(|p| format!("{},{}", p.x, p.y)).collect::<Vec<_>>().join(" ");
        body.push_str(&format!(r#"<polyline points="{points}"/>"#));
    }
    for label in &d.labels {
        body.push_str(&format!(
            r#"<text x="{}" y="{}" font-family="IBM Plex Mono" font-size="18" fill="black" stroke="none">{}</text>"#,
            label.x,
            label.y,
            escape(&label.text)
        ));
    }
    Ok(format!(r#"<svg width="768" height="1024" xmlns="http://www.w3.org/2000/svg">{body}</g></svg>"#))
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn example() -> Illustration {
        Illustration {
            title: "Illustrative response".into(),
            strokes: vec![vec![Point { x: 50., y: 30. }, Point { x: 50., y: 300. }, Point { x: 570., y: 300. }]],
            labels: vec![Label {
                x: 200.,
                y: 338.,
                text: "Time (s)".into(),
            }],
        }
    }
    #[test]
    fn bounds_and_complexity_are_checked_before_rendering() {
        let mut d = example();
        assert!(validate(&[d.clone()]).is_ok());
        d.strokes[0][0].x = f32::NAN;
        assert!(validate(&[d]).is_err());
        let mut d = example();
        d.labels[0].x = 580.;
        assert!(validate(&[d]).is_err());
        let mut d = example();
        d.strokes = vec![vec![Point { x: 10., y: 10. }; 128]; 9];
        assert!(validate(&[d]).is_err());
    }
    #[test]
    fn drawing_is_confined_to_its_reserved_area_and_text_is_escaped() {
        let mut d = example();
        d.title = "<test> & chart".into();
        let rect = Rect { x: 64, y: 300, w: 694, h: 440 };
        let s = svg(&d, rect).unwrap();
        assert!(s.contains("&lt;test&gt; &amp; chart"));
        let bitmap = crate::util::svg_to_bitmap(&s, 768, 1024).unwrap();
        assert!(bitmap.iter().flatten().any(|v| *v));
        for (y, row) in bitmap.iter().enumerate() {
            for (x, pixel) in row.iter().enumerate() {
                if *pixel {
                    assert!((64..758).contains(&x) && (300..740).contains(&y));
                }
            }
        }
        assert!(svg(&d, Rect { x: 64, y: 700, w: 694, h: 280 }).is_err());
    }
}
