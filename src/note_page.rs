//! Verified native note-page insertion on Paper Pro, English firmware 3.27.
//! Never writes document files or uses the floating end-of-document Add button.
use crate::{device::DeviceModel, screenshot::Screenshot, touch::Touch};
use anyhow::{ensure, Result};
use tokio::time::{sleep, Duration};

const OVERVIEW: &[u8] = include_bytes!("../assets/ui/page-overview.png");
const ADD_AFTER: &[u8] = include_bytes!("../assets/ui/add-page-after.png");
const OVERVIEW_REGISTERED: &[u8] = include_bytes!("../assets/ui/page-overview-registered.png");
const ADD_AFTER_REGISTERED: &[u8] = include_bytes!("../assets/ui/add-page-after-registered.png");

fn dark(ss: &Screenshot, x: u32, y: u32) -> bool {
    ss.get_pixel(x, y).map(|(r, g, b)| r.max(g).max(b) < 128).unwrap_or(false)
}

fn control_matches(ss: &Screenshot, png: &[u8], x: u32, y: u32) -> bool {
    let reference = image::load_from_memory(png).expect("embedded UI control").to_luma8();
    let (mut actual, mut expected, mut shared) = (0, 0, 0);
    for (dx, dy, pixel) in reference.enumerate_pixels() {
        let a = dark(ss, x + dx, y + dy);
        let b = pixel.0[0] < 128;
        actual += usize::from(a);
        expected += usize::from(b);
        shared += usize::from(a && b);
    }
    shared * 200 >= (actual + expected).max(1) * 90
}

fn overview(ss: &Screenshot) -> bool {
    control_matches(ss, OVERVIEW, 17, 70) || control_matches(ss, OVERVIEW_REGISTERED, 16, 70)
}

fn add_after(ss: &Screenshot) -> bool {
    control_matches(ss, ADD_AFTER, 620, 171) || control_matches(ss, ADD_AFTER_REGISTERED, 620, 171)
}

fn selecting(ss: &Screenshot) -> bool {
    overview(ss) && [100, 250, 400, 510].iter().all(|&x| dark(ss, x, 20))
}

fn insertion_menu(ss: &Screenshot) -> bool {
    // The selected thumbnail was verified before opening More. Its popover
    // covers the top-right thumbnail's border, so verify the selection toolbar
    // and the actual insertion control here instead of the obscured thumbnail.
    selecting(ss) && add_after(ss)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Tile {
    x: u32,
    y: u32,
    w: u32,
}

impl Tile {
    fn point(self) -> (i32, i32) {
        ((self.x + self.w / 2) as i32, self.y as i32 - 35)
    }
}

/// The current thumbnail has a solid underline, unlike the one-pixel borders
/// on other thumbnails. Reject ambiguous frames and partially visible tiles.
fn current_tile(ss: &Screenshot) -> Result<Tile> {
    ensure!(overview(ss) && !selecting(ss), "Page overview was not recognized");
    let mut tiles: Vec<Tile> = Vec::new();
    for y in 150..960 {
        let mut x = 18;
        while x < 750 {
            if !dark(ss, x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < 750 && dark(ss, x, y) {
                x += 1;
            }
            let w = x - start;
            if (80..=360).contains(&w)
                && (y..y + 3).all(|yy| (start + 2..x - 2).all(|xx| dark(ss, xx, yy)))
                && !tiles.iter().any(|tile| tile.x == start && y - tile.y <= 4)
            {
                tiles.push(Tile { x: start, y, w });
            }
        }
    }
    ensure!(tiles.len() == 1, "Could not uniquely identify the current page thumbnail");
    Ok(tiles[0])
}

fn selected_tile(ss: &Screenshot, tile: Tile) -> bool {
    selecting(ss)
        && tile.x >= 8
        && [10, 30, 60]
            .iter()
            .all(|&dy| dark(ss, tile.x - 5, tile.y - dy) && dark(ss, tile.x + tile.w + 5, tile.y - dy))
}

fn inserted_tile(ss: &Screenshot, original: Tile) -> Result<Tile> {
    let inserted = current_tile(ss)?;
    ensure!(inserted != original, "Current-page indicator did not advance after insertion");
    Ok(inserted)
}

struct Ui<'a> {
    touch: &'a mut Touch,
    rotated: bool,
}

impl Ui<'_> {
    fn capture(&self) -> Result<Screenshot> {
        let mut ss = Screenshot::new()?;
        ss.take_screenshot_oriented(self.rotated)?;
        Ok(ss)
    }

    async fn tap(&mut self, point: (i32, i32)) -> Result<()> {
        self.touch.tap(point).await?;
        // Thumbnail generation and e-ink transitions outlast a normal pen tap.
        sleep(Duration::from_millis(1000)).await;
        Ok(())
    }

    async fn wait_for(&self, stage: &str, check: impl Fn(&Screenshot) -> bool) -> Result<Screenshot> {
        for attempt in 0..4 {
            if let Ok(screen) = self.capture() {
                if check(&screen) {
                    return Ok(screen);
                }
            }
            if attempt < 3 {
                sleep(Duration::from_millis(400)).await;
            }
        }
        anyhow::bail!("Native page controls did not settle at {stage}; no action was repeated")
    }

    async fn insert(&mut self) -> Result<Screenshot> {
        self.tap((28, 894)).await?;
        let original = current_tile(&self.wait_for("current thumbnail", |screen| current_tile(screen).is_ok()).await?)?;
        self.touch.touch_start(original.point()).await?;
        sleep(Duration::from_millis(1000)).await;
        self.touch.touch_stop().await?;
        sleep(Duration::from_millis(1000)).await;
        self.wait_for("selected thumbnail", |screen| selected_tile(screen, original)).await?;
        self.tap((738, 26)).await?;
        self.wait_for("Add page after menu", insertion_menu).await?;
        self.tap((688, 185)).await?; // exactly one creation attempt
        self.wait_for("inserted page selection", selecting).await?;
        self.tap((38, 27)).await?; // cancel selection, keeping the new current page
        let grid = self.wait_for("new current thumbnail", |screen| inserted_tile(screen, original).is_ok()).await?;
        let inserted = inserted_tile(&grid, original)?;
        self.tap(inserted.point()).await?;
        let screen = self.wait_for("new note canvas", |screen| !overview(screen) && Touch::screenshot_palette_open(screen)).await?;
        ensure!(
            !overview(&screen) && Touch::screenshot_palette_open(&screen),
            "New note-page canvas did not open"
        );
        Ok(screen)
    }

    async fn dismiss_overview(&mut self) -> Result<()> {
        // A first outside tap may dismiss only the More popover; then cancel
        // page selection, then close the overview. Re-read after each step.
        for _ in 0..3 {
            let screen = self.capture()?;
            if !overview(&screen) {
                return Ok(());
            }
            self.tap(if selecting(&screen) { (38, 27) } else { (28, 27) }).await?;
        }
        ensure!(!overview(&self.capture()?), "Could not close page overview after insertion failure");
        Ok(())
    }
}

pub async fn insert_after_current(touch: &mut Touch) -> Result<Screenshot> {
    ensure!(
        DeviceModel::detect() == DeviceModel::RemarkablePaperPro,
        "Note-page insertion currently supports Paper Pro only"
    );
    let mut ui = Ui {
        touch,
        rotated: crate::util::ui_rotated_180(),
    };
    let hidden = !Touch::screenshot_palette_open(&ui.capture()?);
    if hidden {
        ui.tap((28, 28)).await?;
    }
    let result = ui.insert().await;
    if result.is_err() {
        ui.dismiss_overview().await?;
    }
    if hidden && Touch::screenshot_palette_open(&ui.capture()?) {
        ui.tap((28, 28)).await?;
    }
    result?;
    ui.capture()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(two_current: bool, selection: bool) -> Screenshot {
        let mut img = image::RgbImage::from_pixel(768, 1024, image::Rgb([255; 3]));
        let control = image::load_from_memory(OVERVIEW).unwrap().to_rgb8();
        image::imageops::overlay(&mut img, &control, 17, 70);
        if selection {
            for y in 0..53 {
                for x in 2..764 {
                    img.put_pixel(x, y, image::Rgb([0; 3]));
                }
            }
        }
        for x in if two_current { vec![140, 391] } else { vec![391] } {
            for y in 622..626 {
                for xx in x..x + 109 {
                    img.put_pixel(xx, y, image::Rgb([0; 3]));
                }
            }
            // Other one-pixel thumbnail borders must not be seen as current.
            for xx in 18..127 {
                img.put_pixel(xx, 622, image::Rgb([0; 3]));
            }
            if selection {
                for yy in 550..630 {
                    for xx in [x - 5, x + 114] {
                        img.put_pixel(xx, yy, image::Rgb([0; 3]));
                    }
                }
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        Screenshot::from_png_data(bytes.into_inner())
    }

    #[test]
    fn identifies_current_page_and_requires_indicator_to_advance() {
        let ss = fixture(false, false);
        let tile = current_tile(&ss).unwrap();
        assert_eq!(tile, Tile { x: 391, y: 622, w: 109 });
        assert!(inserted_tile(&ss, tile).is_err());
        assert_eq!(inserted_tile(&ss, Tile { x: 266, ..tile }).unwrap(), tile);
        assert!(current_tile(&fixture(true, false)).is_err());
        assert!(current_tile(&fixture(false, true)).is_err());
        assert!(selected_tile(&fixture(false, true), tile));
        assert!(!control_matches(&ss, ADD_AFTER, 620, 171));
    }

    #[test]
    fn registered_capture_controls_keep_strict_matching_without_legacy_scaling() {
        let mut ss = fixture(false, false);
        let mut img = image::RgbImage::from_fn(768, 1024, |x, y| {
            let (r, g, b) = ss.get_pixel(x, y).unwrap();
            image::Rgb([r, g, b])
        });
        // Replace only public UI controls, never embed notebook thumbnails.
        for y in 70..94 {
            for x in 16..113 { img.put_pixel(x, y, image::Rgb([255; 3])); }
        }
        let control = image::load_from_memory(OVERVIEW_REGISTERED).unwrap().to_rgb8();
        image::imageops::overlay(&mut img, &control, 16, 70);
        let menu = image::load_from_memory(ADD_AFTER_REGISTERED).unwrap().to_rgb8();
        image::imageops::overlay(&mut img, &menu, 620, 171);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        ss = Screenshot::from_png_data(bytes.into_inner());
        assert!(overview(&ss));
        assert!(current_tile(&ss).is_ok());
        assert!(add_after(&ss));
        assert!(!control_matches(&ss, OVERVIEW, 17, 70));
        assert!(!control_matches(&ss, ADD_AFTER, 620, 171));
    }

    #[test]
    fn insertion_menu_accepts_an_occluded_top_right_thumbnail_only_in_selection_mode() {
        let tile = Tile { x: 640, y: 268, w: 109 };
        let ss = fixture(false, true);
        let mut img = image::RgbImage::from_fn(768, 1024, |x, y| {
            let (r, g, b) = ss.get_pixel(x, y).unwrap();
            image::Rgb([r, g, b])
        });
        for y in 118..276 {
            for x in [tile.x - 5, tile.x + tile.w + 5] {
                img.put_pixel(x, y, image::Rgb([0; 3]));
            }
        }
        let screen = |img: &image::RgbImage| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(img.clone()).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            Screenshot::from_png_data(bytes.into_inner())
        };
        assert!(selected_tile(&screen(&img), tile));
        // More's opaque popover hides every border sample for page six.
        for y in 53..265 {
            for x in 607..764 { img.put_pixel(x, y, image::Rgb([255; 3])); }
        }
        let control = image::load_from_memory(ADD_AFTER_REGISTERED).unwrap().to_rgb8();
        image::imageops::overlay(&mut img, &control, 620, 171);
        assert!(!selected_tile(&screen(&img), tile));
        assert!(insertion_menu(&screen(&img)));
        // A matching menu label alone must never authorize page insertion.
        img.put_pixel(250, 20, image::Rgb([255; 3]));
        assert!(!insertion_menu(&screen(&img)));
    }
}
