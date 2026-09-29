//! Portrait controls verified on Paper Pro Move firmware 3.29.
//! Coordinates use the same 768x1024 page space as the renderer.
use crate::screenshot::Screenshot;

pub const TOGGLE: (i32, i32) = (45, 34);
pub const PEN: (i32, i32) = (134, 34);
pub const MORE: (i32, i32) = (628, 34);
pub const TYPES: [(i32, i32); 9] = [
    (160, 150),
    (250, 150),
    (339, 150),
    (160, 217),
    (250, 217),
    (339, 217),
    (160, 285),
    (250, 285),
    (339, 285),
];
pub const SIZES: [(i32, i32); 3] = [(160, 421), (250, 421), (339, 421)];
pub const COLORS: [(i32, i32); 9] = [
    (160, 529),
    (250, 529),
    (339, 529),
    (160, 596),
    (250, 596),
    (339, 596),
    (160, 664),
    (250, 664),
    (339, 664),
];

pub fn dark(ss: &Screenshot, x: i32, y: i32) -> bool {
    ss.get_pixel(x as u32, y as u32).is_some_and(|(r, g, b)| r.max(g).max(b) < 100)
}

pub fn control_matches(ss: &Screenshot, png: &[u8], x: u32, y: u32) -> bool {
    let reference = image::load_from_memory(png).expect("embedded UI control").to_luma8();
    let (mut actual, mut expected, mut shared) = (0, 0, 0);
    for (dx, dy, pixel) in reference.enumerate_pixels() {
        let a = dark(ss, (x + dx) as i32, (y + dy) as i32);
        let b = pixel.0[0] < 100;
        actual += usize::from(a);
        expected += usize::from(b);
        shared += usize::from(a && b);
    }
    shared * 200 >= (actual + expected).max(1) * 86
}

/// Detect the small vertical displacement introduced by native toolbar layouts.
/// A strict match still rejects landscape, lock screens, and relocated toolbars.
pub fn portrait_offset(ss: &Screenshot) -> Option<i32> {
    let reference = image::load_from_memory(include_bytes!("../assets/ui/move-toolbar-toggle.png"))
        .unwrap()
        .to_luma8();
    let mut best = (0, 0);
    for offset in 0..=16 {
        let (mut actual, mut expected, mut shared) = (0, 0, 0);
        for (x, y, pixel) in reference.enumerate_pixels() {
            // The dot inside the toggle moves when the toolbar is hidden.
            if (7..28).contains(&x) && (7..23).contains(&y) {
                continue;
            }
            let a = dark(ss, x as i32 + 28, y as i32 + 19 + offset);
            let b = pixel.0[0] < 100;
            actual += usize::from(a);
            expected += usize::from(b);
            shared += usize::from(a && b);
        }
        let score = shared * 2000 / (actual + expected).max(1);
        if score > best.1 {
            best = (offset, score);
        }
    }
    (best.1 >= 900).then_some(best.0)
}

pub fn portrait_canvas(ss: &Screenshot) -> bool {
    portrait_offset(ss).is_some()
}

pub fn selected_tool(ss: &Screenshot) -> Option<i32> {
    // Logical pen ID 80 is shared with the restoration state machine. Other
    // tool IDs are their horizontal positions in the Move's fixed toolbar.
    let offset = portrait_offset(ss)?;
    let mut tools = [(134, 80), (224, 224), (313, 313)]
        .into_iter()
        .filter_map(|(x, id)| [x - 30, x, x + 30].iter().all(|&xx| dark(ss, xx, 60 + offset)).then_some(id));
    let first = tools.next()?;
    tools.next().is_none().then_some(first)
}

pub fn toolbar_open(ss: &Screenshot) -> bool {
    portrait_canvas(ss) && selected_tool(ss).is_some()
}

pub fn popover(ss: &Screenshot) -> bool {
    let Some(offset) = portrait_offset(ss) else {
        return false;
    };
    [90, 409].iter().all(|&x| [100, 200, 300, 450, 600].iter().all(|&y| dark(ss, x, y + offset)))
}

pub fn selected_cell(ss: &Screenshot, cells: &[(i32, i32)]) -> Option<usize> {
    let offset = portrait_offset(ss)?;
    let mut matches = cells.iter().enumerate().filter_map(|(index, &(x, y))| {
        [-30, 30]
            .iter()
            .all(|dx| [-24, 24].iter().all(|dy| dark(ss, x + dx, y + dy + offset)))
            .then_some(index)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

pub fn add_page_control(ss: &Screenshot) -> Option<(i32, i32)> {
    if !toolbar_open(ss) {
        return None;
    }
    // The More menu can omit document-specific actions. Match the native Add
    // page label rather than assuming its vertical position on every document.
    let reference = image::load_from_memory(include_bytes!("../assets/ui/move-add-page.png")).unwrap().to_luma8();
    let points: Vec<_> = reference.enumerate_pixels().filter(|(_, _, p)| p.0[0] < 100).map(|(x, y, _)| (x, y)).collect();
    let probes: Vec<_> = points.iter().step_by((points.len() / 32).max(1)).copied().collect();
    let mut candidates = Vec::new();
    for y in 70..950 - reference.height() {
        if probes.iter().filter(|&&(dx, dy)| dark(ss, (373 + dx) as i32, (y + dy) as i32)).count() * 10 < probes.len() * 9 {
            continue;
        }
        if control_matches(ss, include_bytes!("../assets/ui/move-add-page.png"), 373, y) {
            candidates.push(y);
        }
    }
    let first = *candidates.first()?;
    if candidates.iter().any(|y| y.abs_diff(first) > 4) {
        return None;
    }
    Some((512, first as i32 + 20))
}

pub fn add_page_menu(ss: &Screenshot) -> bool {
    add_page_control(ss).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screen(mut draw: impl FnMut(&mut image::RgbImage)) -> Screenshot {
        let mut im = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
        let toggle = image::load_from_memory(include_bytes!("../assets/ui/move-toolbar-toggle.png"))
            .unwrap()
            .to_rgb8();
        image::imageops::overlay(&mut im, &toggle, 28, 19);
        draw(&mut im);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(im).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Screenshot::from_png_data(bytes.into_inner())
    }
    fn cell(im: &mut image::RgbImage, x: i32, y: i32) {
        for yy in y - 30..y + 31 {
            for xx in x - 40..x + 41 {
                im.put_pixel(xx as u32, yy as u32, image::Rgb([0; 3]));
            }
        }
    }
    #[test]
    fn recognizes_all_fixed_tool_slots_and_hidden_toolbar() {
        assert!(portrait_canvas(&screen(|_| {})));
        assert!(!toolbar_open(&screen(|_| {})));
        for (x, id) in [(134, 80), (224, 224), (313, 313)] {
            let ss = screen(|im| {
                for yy in 0..68 {
                    for xx in x - 44..x + 45 {
                        im.put_pixel(xx, yy, image::Rgb([0; 3]));
                    }
                }
            });
            assert_eq!(selected_tool(&ss), Some(id));
            assert!(toolbar_open(&ss));
        }
    }
    #[test]
    fn identifies_palette_cells_and_rejects_ambiguous_selection() {
        for cells in [&TYPES[..], &SIZES[..], &COLORS[..]] {
            for (i, &(x, y)) in cells.iter().enumerate() {
                assert_eq!(selected_cell(&screen(|im| cell(im, x, y)), cells), Some(i));
            }
            assert_eq!(
                selected_cell(
                    &screen(|im| {
                        cell(im, cells[0].0, cells[0].1);
                        cell(im, cells[1].0, cells[1].1);
                    }),
                    cells
                ),
                None
            );
        }
    }
    #[test]
    fn finds_page_control_when_document_actions_shift_it() {
        for y in [622, 690] {
            let ss = screen(|im| {
                for yy in 0..68 {
                    for xx in 90..179 {
                        im.put_pixel(xx, yy, image::Rgb([0; 3]));
                    }
                }
                let add = image::load_from_memory(include_bytes!("../assets/ui/move-add-page.png")).unwrap().to_rgb8();
                image::imageops::overlay(im, &add, 373, y);
            });
            assert_eq!(add_page_control(&ss), Some((512, y as i32 + 20)));
        }
    }
    #[test]
    fn follows_toolbar_offset_after_extension_and_visibility_changes() {
        let source = screen(|im| {
            for yy in 0..68 {
                for xx in 269..359 {
                    im.put_pixel(xx, yy, image::Rgb([0; 3]));
                }
            }
            cell(im, COLORS[3].0, COLORS[3].1);
        });
        let mut decoded = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
        for y in 0..1024 {
            for x in 0..768 {
                let (r, g, b) = source.get_pixel(x, y).unwrap();
                decoded.put_pixel(x, y, image::Rgb([r, g, b]));
            }
        }
        for offset in [0, 6, 10, 16] {
            let mut shifted = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
            image::imageops::overlay(&mut shifted, &decoded, 0, offset);
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(shifted).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            let ss = Screenshot::from_png_data(bytes.into_inner());
            assert_eq!(portrait_offset(&ss), Some(offset as i32));
            assert_eq!(selected_tool(&ss), Some(313));
            assert_eq!(selected_cell(&ss, &COLORS), Some(3));
        }
    }
    #[test]
    fn rejects_nonportrait_or_noncanvas_captures() {
        let blank = screen(|im| {
            for y in 0..70 {
                for x in 0..768 {
                    im.put_pixel(x, y, image::Rgb([255; 3]));
                }
            }
        });
        assert!(!portrait_canvas(&blank));
        assert!(!toolbar_open(&blank));
        assert_eq!(add_page_control(&blank), None);
    }
}
