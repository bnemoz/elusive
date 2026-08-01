//! Turning a captured framebuffer into a PNG of the chromatogram pane.
//!
//! Everything here is pure: given an image size, a rect and a scale factor it
//! answers with numbers and bytes, never with a file. That matters because the
//! capture itself cannot be exercised headlessly — the interesting arithmetic
//! (points to physical pixels, clamping to the framebuffer) has to be testable on
//! its own or it is only ever checked by eye on one machine's display scale.

use std::path::PathBuf;

/// The pixel window of a logical-point rect inside a captured framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropBounds {
    /// Top-left corner, in physical pixels.
    pub origin: [usize; 2],
    /// Width and height, in physical pixels. Always non-zero.
    pub size: [usize; 2],
}

/// Default file name for a run's chromatogram image.
pub fn default_file_name(stem: &str) -> String {
    format!("{stem}-chromatogram.png")
}

/// Make sure a path chosen in the save dialog ends in `.png`.
///
/// A save dialog hands back exactly what was typed, and the GTK backend does not
/// append the filter's extension, so `myrun` would be written extension-less and
/// no image viewer would open it on a double-click.
///
/// The suffix is *appended* rather than substituted. Replacing the extension
/// would turn `run.v2` into `run.png` and silently drop part of the name the user
/// chose; `run.v2.png` keeps it and is still unambiguously a PNG.
pub fn with_png_extension(path: PathBuf) -> PathBuf {
    let already_png = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if already_png {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".png");
    PathBuf::from(name)
}

/// Where a pane's rect lands in a screenshot of the whole viewport.
///
/// `rect` is in logical points and the framebuffer is in physical pixels, so the
/// scale factor has to be applied here; getting it wrong yields a crop of the
/// wrong region at any display scale other than 1.0. The result is clamped to the
/// image, because a pane can be partly scrolled or dragged off screen and an
/// out-of-range slice would panic rather than produce a smaller picture.
///
/// Returns `None` when nothing usable is left — a zero-area rect, a rect entirely
/// off screen, or a nonsensical scale factor. Callers report that rather than
/// writing a zero-byte image.
pub fn crop_bounds(
    image_size: [usize; 2],
    rect: egui::Rect,
    pixels_per_point: f32,
) -> Option<CropBounds> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }
    let [width, height] = image_size;

    // Non-finite coordinates are normal here: `Rect::NOTHING` is the "no rect"
    // sentinel and is built from infinities.
    let to_px = |v: f32, limit: usize| -> usize {
        let scaled = v * pixels_per_point;
        if !scaled.is_finite() || scaled <= 0.0 {
            0
        } else {
            (scaled as usize).min(limit)
        }
    };

    let x0 = to_px(rect.min.x, width);
    let y0 = to_px(rect.min.y, height);
    // The far edge rounds up so a pane ending on a fractional pixel keeps its last
    // column instead of losing it to truncation.
    let x1 = to_px(rect.max.x.ceil(), width);
    let y1 = to_px(rect.max.y.ceil(), height);

    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(CropBounds {
        origin: [x0, y0],
        size: [x1 - x0, y1 - y0],
    })
}

/// Flatten a captured image to straight (non-premultiplied) RGBA8.
///
/// `Color32` stores premultiplied alpha; PNG stores straight alpha. A screenshot
/// is opaque so the two agree in practice, but converting explicitly means the
/// function stays correct if it is ever handed something translucent.
pub fn to_rgba8(image: &egui::ColorImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in &image.pixels {
        out.extend_from_slice(&pixel.to_srgba_unmultiplied());
    }
    out
}

/// Encode straight RGBA8 pixels as a PNG.
pub fn encode_png(size: [usize; 2], rgba: &[u8]) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder as _;

    let [width, height] = size;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(
        rgba,
        width as u32,
        height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min: [f32; 2], max: [f32; 2]) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(min[0], min[1]), egui::pos2(max[0], max[1]))
    }

    #[test]
    fn a_rect_maps_straight_through_at_scale_one() {
        let bounds = crop_bounds([800, 600], rect([10.0, 20.0], [110.0, 220.0]), 1.0)
            .expect("the rect is fully inside the image");
        assert_eq!(bounds.origin, [10, 20]);
        assert_eq!(bounds.size, [100, 200]);
    }

    #[test]
    fn a_rect_scales_with_the_display() {
        // The bug this guards against is a quarter-sized crop of the wrong corner
        // on a HiDPI screen, which is invisible to anyone testing at 1.0.
        let logical = rect([10.0, 20.0], [110.0, 220.0]);

        let at_1_5 = crop_bounds([1200, 900], logical, 1.5).expect("inside");
        assert_eq!(at_1_5.origin, [15, 30]);
        assert_eq!(at_1_5.size, [150, 300]);

        let at_2 = crop_bounds([1600, 1200], logical, 2.0).expect("inside");
        assert_eq!(at_2.origin, [20, 40]);
        assert_eq!(at_2.size, [200, 400]);
    }

    #[test]
    fn a_fractional_edge_keeps_its_last_pixel() {
        let bounds = crop_bounds([800, 600], rect([0.0, 0.0], [100.5, 50.5]), 1.0).expect("inside");
        assert_eq!(bounds.size, [101, 51]);
    }

    #[test]
    fn a_rect_hanging_off_the_image_is_clamped_not_refused() {
        let bounds = crop_bounds([800, 600], rect([700.0, 500.0], [900.0, 700.0]), 1.0)
            .expect("part of the rect is still on screen");
        assert_eq!(bounds.origin, [700, 500]);
        assert_eq!(bounds.size, [100, 100]);
        // The whole point of clamping: the slice stays inside the framebuffer.
        assert!(bounds.origin[0] + bounds.size[0] <= 800);
        assert!(bounds.origin[1] + bounds.size[1] <= 600);
    }

    #[test]
    fn a_rect_scaled_past_the_image_is_clamped_too() {
        // 500 pt at 2x is 1000 px, well past a 800 px-wide framebuffer.
        let bounds =
            crop_bounds([800, 600], rect([0.0, 0.0], [500.0, 400.0]), 2.0).expect("inside");
        assert_eq!(bounds.size, [800, 600]);
    }

    #[test]
    fn degenerate_rects_yield_nothing_to_crop() {
        assert_eq!(crop_bounds([800, 600], egui::Rect::ZERO, 1.0), None);
        assert_eq!(crop_bounds([800, 600], egui::Rect::NOTHING, 1.0), None);
        assert_eq!(
            crop_bounds([800, 600], rect([10.0, 10.0], [10.0, 200.0]), 1.0),
            None,
            "a zero-width rect has no pixels"
        );
        assert_eq!(
            crop_bounds([800, 600], rect([900.0, 10.0], [1000.0, 200.0]), 1.0),
            None,
            "a rect entirely off the right edge has nothing to show"
        );
    }

    #[test]
    fn an_empty_image_yields_nothing_to_crop() {
        assert_eq!(
            crop_bounds([0, 0], rect([0.0, 0.0], [10.0, 10.0]), 1.0),
            None
        );
    }

    #[test]
    fn a_silly_scale_factor_is_refused() {
        let r = rect([0.0, 0.0], [10.0, 10.0]);
        assert_eq!(crop_bounds([800, 600], r, 0.0), None);
        assert_eq!(crop_bounds([800, 600], r, -1.0), None);
        assert_eq!(crop_bounds([800, 600], r, f32::NAN), None);
    }

    #[test]
    fn a_bare_name_gains_the_png_suffix() {
        assert_eq!(
            with_png_extension(PathBuf::from("/tmp/myrun")),
            PathBuf::from("/tmp/myrun.png")
        );
    }

    #[test]
    fn an_existing_png_suffix_is_left_alone() {
        for name in ["/tmp/myrun.png", "/tmp/myrun.PNG"] {
            assert_eq!(with_png_extension(PathBuf::from(name)), PathBuf::from(name));
        }
    }

    #[test]
    fn a_dotted_name_keeps_every_part_of_itself() {
        // `set_extension` would answer `run.png` and lose the version marker.
        assert_eq!(
            with_png_extension(PathBuf::from("/tmp/run.v2")),
            PathBuf::from("/tmp/run.v2.png")
        );
    }

    #[test]
    fn the_default_name_is_derived_from_the_run() {
        assert_eq!(default_file_name("myrun"), "myrun-chromatogram.png");
    }

    #[test]
    fn a_synthetic_image_round_trips_through_the_encoder() {
        let mut image = egui::ColorImage::filled([4, 3], egui::Color32::TRANSPARENT);
        image.pixels[0] = egui::Color32::from_rgb(0xB4, 0x45, 0x55);
        image.pixels[5] = egui::Color32::from_rgb(0x4C, 0x8F, 0xD8);
        image.pixels[11] = egui::Color32::WHITE;

        let rgba = to_rgba8(&image);
        assert_eq!(rgba.len(), 4 * 3 * 4);

        let png = encode_png(image.size, &rgba).expect("a 4x3 RGBA buffer is encodable");
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("what we just wrote must decode")
            .to_rgba8();

        assert_eq!(decoded.dimensions(), (4, 3));
        assert_eq!(decoded.as_raw().as_slice(), rgba.as_slice());
    }

    #[test]
    fn cropping_a_synthetic_image_takes_the_asked_for_pixels() {
        // Exercises `crop_bounds` against the slicing it feeds, at 2x, so an
        // origin/size mix-up shows up as the wrong colour rather than a panic.
        let mut image = egui::ColorImage::filled([8, 8], egui::Color32::BLACK);
        for y in 4..8 {
            for x in 2..6 {
                image.pixels[y * 8 + x] = egui::Color32::WHITE;
            }
        }

        let bounds = crop_bounds(image.size, rect([1.0, 2.0], [3.0, 4.0]), 2.0).expect("inside");
        assert_eq!(bounds.origin, [2, 4]);
        assert_eq!(bounds.size, [4, 4]);

        let cropped = image.region_by_pixels(bounds.origin, bounds.size);
        assert_eq!(cropped.size, [4, 4]);
        assert!(cropped.pixels.iter().all(|p| *p == egui::Color32::WHITE));
    }
}
