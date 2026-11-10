//! Decoded image assets.
//!
//! A document references images by file name into its package. Decoding is
//! done once per document load and cached, because a shape's fill is re-encoded
//! into the scene on every frame and decoding a PNG at sixty hertz is not a
//! thing anyone should do.

use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use std::collections::HashMap;
use std::sync::Arc;

/// Images available to the current document, keyed by the name a shape uses to
/// reference them.
#[derive(Clone, Default)]
pub struct ImageCache {
    decoded: HashMap<String, ImageData>,
    /// Names that failed to decode, so a broken asset is not retried each load.
    failed: Vec<String>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.decoded.is_empty()
    }

    pub fn len(&self) -> usize {
        self.decoded.len()
    }

    /// Replaces the cache with the assets from a package.
    ///
    /// Undecodable entries are skipped and reported rather than failing the
    /// load: a document with one corrupt image should still open.
    pub fn load(&mut self, assets: impl IntoIterator<Item = (String, Vec<u8>)>) {
        self.decoded.clear();
        self.failed.clear();

        for (name, bytes) in assets {
            match decode(&bytes) {
                Some(image) => {
                    self.decoded.insert(name, image);
                }
                None => {
                    log::warn!("could not decode image {name:?}");
                    self.failed.push(name);
                }
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<&ImageData> {
        self.decoded.get(name)
    }

    /// Whether an asset was present but could not be decoded, which is worth
    /// distinguishing from an asset that is simply absent.
    pub fn failed(&self, name: &str) -> bool {
        self.failed.iter().any(|n| n == name)
    }
}

/// Decodes any image format the `image` crate recognizes into premultiplied
/// RGBA, which is what the renderer samples.
fn decode(bytes: &[u8]) -> Option<ImageData> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return None;
    }

    Some(ImageData {
        data: Blob::new(Arc::new(rgba.into_raw())),
        format: ImageFormat::Rgba8,
        // The `image` crate produces straight alpha, not premultiplied.
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

/// The transform that places an image of the given size into `area` under a
/// fit mode.
pub fn placement(
    mode: dc_model::ImageScale,
    area: kurbo::Rect,
    image_width: u32,
    image_height: u32,
) -> kurbo::Affine {
    use dc_model::ImageScale;

    let (iw, ih) = (image_width.max(1) as f64, image_height.max(1) as f64);
    let (aw, ah) = (area.width().max(0.0), area.height().max(0.0));
    if aw <= 0.0 || ah <= 0.0 {
        return kurbo::Affine::translate((area.x0, area.y0));
    }

    let (sx, sy) = match mode {
        // Whole image visible, aspect preserved.
        ImageScale::Fit => {
            let s = (aw / iw).min(ah / ih);
            (s, s)
        }
        // Area covered, aspect preserved, overflow cropped by the shape.
        ImageScale::Fill | ImageScale::Tile => {
            let s = (aw / iw).max(ah / ih);
            (s, s)
        }
        // Aspect ignored.
        ImageScale::Stretch => (aw / iw, ah / ih),
        ImageScale::Original => (1.0, 1.0),
    };

    // Centered in the area, which is what every fit mode here expects.
    let dx = area.x0 + (aw - iw * sx) / 2.0;
    let dy = area.y0 + (ah - ih * sy) / 2.0;
    kurbo::Affine::translate((dx, dy)) * kurbo::Affine::scale_non_uniform(sx, sy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::ImageScale;
    use kurbo::Rect;

    /// A tiny valid PNG, encoded here so the test needs no fixture file.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut buffer, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels = vec![0xA0u8; (width * height * 4) as usize];
            writer.write_image_data(&pixels).unwrap();
        }
        buffer
    }

    #[test]
    fn a_png_decodes_to_rgba() {
        let image = decode(&png_bytes(4, 3)).expect("should decode");
        assert_eq!((image.width, image.height), (4, 3));
        assert_eq!(image.format, ImageFormat::Rgba8);
        assert_eq!(image.data.len(), 4 * 3 * 4);
    }

    #[test]
    fn garbage_does_not_decode_and_does_not_panic() {
        assert!(decode(b"not an image at all").is_none());
        assert!(decode(&[]).is_none());
        // A truncated PNG header.
        assert!(decode(&png_bytes(2, 2)[..8]).is_none());
    }

    #[test]
    fn the_cache_holds_what_decoded_and_remembers_what_did_not() {
        let mut cache = ImageCache::new();
        cache.load([
            ("logo.png".to_string(), png_bytes(2, 2)),
            ("broken.png".to_string(), b"junk".to_vec()),
        ]);

        assert_eq!(cache.len(), 1);
        assert!(cache.get("logo.png").is_some());
        assert!(cache.get("broken.png").is_none());
        assert!(cache.failed("broken.png"), "a corrupt asset is distinguishable");
        assert!(!cache.failed("absent.png"));
    }

    #[test]
    fn loading_replaces_the_previous_document() {
        let mut cache = ImageCache::new();
        cache.load([("a.png".to_string(), png_bytes(2, 2))]);
        cache.load([("b.png".to_string(), png_bytes(2, 2))]);

        assert!(cache.get("a.png").is_none(), "stale assets must be dropped");
        assert!(cache.get("b.png").is_some());
    }

    fn area() -> Rect {
        Rect::new(10.0, 20.0, 210.0, 120.0) // 200 x 100
    }

    /// Where the image's corners land after placement.
    fn placed(mode: ImageScale, w: u32, h: u32) -> Rect {
        let t = placement(mode, area(), w, h);
        let tl = t * kurbo::Point::new(0.0, 0.0);
        let br = t * kurbo::Point::new(w as f64, h as f64);
        Rect::new(tl.x, tl.y, br.x, br.y)
    }

    #[test]
    fn fit_shows_the_whole_image_inside_the_area() {
        // A 100x100 image in a 200x100 area fits at 1x, centered.
        let r = placed(ImageScale::Fit, 100, 100);
        assert!((r.width() - 100.0).abs() < 1e-9, "{r:?}");
        assert!(r.x0 >= area().x0 - 1e-9 && r.x1 <= area().x1 + 1e-9);
        assert!(r.y0 >= area().y0 - 1e-9 && r.y1 <= area().y1 + 1e-9);
    }

    #[test]
    fn fit_preserves_aspect_ratio() {
        let r = placed(ImageScale::Fit, 400, 100);
        assert!((r.width() / r.height() - 4.0).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn fill_covers_the_area() {
        let r = placed(ImageScale::Fill, 100, 100);
        assert!(r.width() >= area().width() - 1e-9, "{r:?}");
        assert!(r.height() >= area().height() - 1e-9);
        // And keeps its proportions.
        assert!((r.width() / r.height() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn stretch_matches_the_area_exactly() {
        let r = placed(ImageScale::Stretch, 37, 91);
        assert!((r.width() - area().width()).abs() < 1e-9, "{r:?}");
        assert!((r.height() - area().height()).abs() < 1e-9);
    }

    #[test]
    fn original_keeps_pixel_size_and_centers() {
        let r = placed(ImageScale::Original, 50, 40);
        assert!((r.width() - 50.0).abs() < 1e-9);
        assert!((r.height() - 40.0).abs() < 1e-9);
        assert!((r.center().x - area().center().x).abs() < 1e-9, "{r:?}");
        assert!((r.center().y - area().center().y).abs() < 1e-9);
    }

    #[test]
    fn every_mode_centers_on_the_area() {
        for mode in [
            ImageScale::Fit,
            ImageScale::Fill,
            ImageScale::Stretch,
            ImageScale::Original,
            ImageScale::Tile,
        ] {
            let r = placed(mode, 64, 48);
            assert!(
                (r.center().x - area().center().x).abs() < 1e-6
                    && (r.center().y - area().center().y).abs() < 1e-6,
                "{mode:?} was off center: {r:?}"
            );
        }
    }

    #[test]
    fn a_degenerate_area_does_not_divide_by_zero() {
        let t = placement(ImageScale::Fit, Rect::new(5.0, 5.0, 5.0, 5.0), 10, 10);
        let p = t * kurbo::Point::new(0.0, 0.0);
        assert!(p.x.is_finite() && p.y.is_finite());
    }

    #[test]
    fn a_zero_sized_image_does_not_divide_by_zero() {
        let t = placement(ImageScale::Fit, area(), 0, 0);
        let p = t * kurbo::Point::new(1.0, 1.0);
        assert!(p.x.is_finite() && p.y.is_finite());
    }
}
