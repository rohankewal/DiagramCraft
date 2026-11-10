//! The mapping between document space and the viewport.

use dc_model::{BoundingBox, Point};
use kurbo::{Affine, Rect, Vec2};

/// Zoom limits. Below the minimum a document is unreadable; above the maximum
/// floating-point precision in the renderer starts to show.
pub const MIN_ZOOM: f64 = 0.02;
pub const MAX_ZOOM: f64 = 64.0;

/// A 2D view onto the document.
///
/// `origin` is the document point shown at the viewport's top-left corner, and
/// `zoom` is logical pixels per document unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub origin: Point,
    pub zoom: f64,
    /// Viewport size in logical pixels.
    pub viewport: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            origin: Point::ORIGIN,
            zoom: 1.0,
            viewport: Vec2::new(1280.0, 800.0),
        }
    }
}

impl Camera {
    pub fn new(viewport: Vec2) -> Self {
        Camera { viewport, ..Default::default() }
    }

    /// The transform from document space to viewport space.
    pub fn transform(&self) -> Affine {
        Affine::translate((-self.origin.x * self.zoom, -self.origin.y * self.zoom))
            * Affine::scale(self.zoom)
    }

    pub fn doc_to_screen(&self, p: Point) -> Point {
        Point::new(
            (p.x - self.origin.x) * self.zoom,
            (p.y - self.origin.y) * self.zoom,
        )
    }

    pub fn screen_to_doc(&self, p: Point) -> Point {
        Point::new(p.x / self.zoom + self.origin.x, p.y / self.zoom + self.origin.y)
    }

    /// Converts a screen-space distance to document units.
    pub fn screen_to_doc_scalar(&self, v: f64) -> f64 {
        v / self.zoom
    }

    /// The document region currently visible.
    pub fn visible_rect(&self) -> Rect {
        let bottom_right = self.screen_to_doc(Point::new(self.viewport.x, self.viewport.y));
        Rect::new(self.origin.x, self.origin.y, bottom_right.x, bottom_right.y)
    }

    /// Scrolls the view by a screen-space delta.
    pub fn pan_by_screen(&mut self, dx: f64, dy: f64) {
        self.origin.x -= dx / self.zoom;
        self.origin.y -= dy / self.zoom;
    }

    /// Zooms by `factor`, keeping the document point under `anchor` fixed.
    /// This is what makes scroll-wheel zoom feel anchored to the cursor.
    pub fn zoom_at(&mut self, anchor: Point, factor: f64) {
        let before = self.screen_to_doc(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.screen_to_doc(anchor);
        self.origin.x += before.x - after.x;
        self.origin.y += before.y - after.y;
    }

    /// Sets an absolute zoom level, keeping `anchor` fixed.
    pub fn set_zoom_at(&mut self, anchor: Point, zoom: f64) {
        let target = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if self.zoom > 0.0 {
            self.zoom_at(anchor, target / self.zoom);
        }
    }

    /// Centers the view on a document point without changing zoom.
    pub fn center_on(&mut self, p: Point) {
        self.origin = Point::new(
            p.x - self.viewport.x / (2.0 * self.zoom),
            p.y - self.viewport.y / (2.0 * self.zoom),
        );
    }

    /// Frames a document region, leaving `padding` screen pixels around it.
    /// A degenerate region keeps the current zoom and just centers.
    pub fn fit(&mut self, region: BoundingBox, padding: f64) {
        let avail_w = (self.viewport.x - padding * 2.0).max(1.0);
        let avail_h = (self.viewport.y - padding * 2.0).max(1.0);

        if region.w > 0.0 && region.h > 0.0 {
            let zoom = (avail_w / region.w).min(avail_h / region.h);
            self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
        self.center_on(region.center());
    }

    /// Resets to a hundred-percent view at the document origin.
    pub fn reset(&mut self) {
        self.origin = Point::ORIGIN;
        self.zoom = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new(Vec2::new(800.0, 600.0))
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn identity_camera_maps_one_to_one() {
        let c = cam();
        let p = c.doc_to_screen(Point::new(10.0, 20.0));
        assert!(approx(p.x, 10.0) && approx(p.y, 20.0));
    }

    #[test]
    fn screen_and_doc_conversions_are_inverses() {
        let mut c = cam();
        c.origin = Point::new(-33.0, 71.5);
        c.zoom = 2.75;

        let doc = Point::new(123.4, -56.7);
        let back = c.screen_to_doc(c.doc_to_screen(doc));
        assert!(approx(back.x, doc.x) && approx(back.y, doc.y), "{back:?}");
    }

    #[test]
    fn transform_matches_manual_conversion() {
        let mut c = cam();
        c.origin = Point::new(5.0, 7.0);
        c.zoom = 1.5;

        let doc = Point::new(40.0, 60.0);
        let via_matrix = c.transform() * kurbo::Point::new(doc.x, doc.y);
        let direct = c.doc_to_screen(doc);
        assert!(approx(via_matrix.x, direct.x), "{via_matrix:?} vs {direct:?}");
        assert!(approx(via_matrix.y, direct.y));
    }

    #[test]
    fn zoom_keeps_the_anchor_point_fixed() {
        let mut c = cam();
        let anchor = Point::new(400.0, 300.0);
        let doc_before = c.screen_to_doc(anchor);

        c.zoom_at(anchor, 2.5);
        let doc_after = c.screen_to_doc(anchor);

        assert!(approx(doc_before.x, doc_after.x), "{doc_before:?} vs {doc_after:?}");
        assert!(approx(doc_before.y, doc_after.y));
        assert!(approx(c.zoom, 2.5));
    }

    #[test]
    fn zoom_is_clamped_to_usable_limits() {
        let mut c = cam();
        c.zoom_at(Point::ORIGIN, 1e9);
        assert_eq!(c.zoom, MAX_ZOOM);

        c.zoom_at(Point::ORIGIN, 1e-12);
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn panning_moves_the_view_opposite_the_drag() {
        let mut c = cam();
        c.zoom = 2.0;
        // Dragging the canvas right by 100px reveals content to the left.
        c.pan_by_screen(100.0, 0.0);
        assert!(approx(c.origin.x, -50.0), "origin was {:?}", c.origin);
    }

    #[test]
    fn fit_frames_the_region_with_padding() {
        let mut c = cam();
        // A 400x300 region in an 800x600 viewport with 50px padding fits at
        // 700/400 = 1.75 horizontally and 500/300 = 1.67 vertically.
        let region = BoundingBox::new(0.0, 0.0, 400.0, 300.0);
        c.fit(region, 50.0);

        assert!((c.zoom - 500.0 / 300.0).abs() < 1e-9, "zoom was {}", c.zoom);
        // And the region's center lands in the viewport's center.
        let center = c.doc_to_screen(region.center());
        assert!((center.x - 400.0).abs() < 1e-6, "{center:?}");
        assert!((center.y - 300.0).abs() < 1e-6);
    }

    #[test]
    fn fit_on_an_empty_region_keeps_zoom_and_centers() {
        let mut c = cam();
        c.zoom = 3.0;
        let region = BoundingBox::new(100.0, 100.0, 0.0, 0.0);
        c.fit(region, 20.0);

        assert_eq!(c.zoom, 3.0, "a zero-size region cannot determine a zoom");
        let center = c.doc_to_screen(Point::new(100.0, 100.0));
        assert!((center.x - 400.0).abs() < 1e-6);
    }

    #[test]
    fn visible_rect_tracks_origin_and_zoom() {
        let mut c = cam();
        c.origin = Point::new(10.0, 20.0);
        c.zoom = 2.0;
        let r = c.visible_rect();

        assert!(approx(r.x0, 10.0) && approx(r.y0, 20.0));
        // 800 screen pixels at 2x covers 400 document units.
        assert!(approx(r.width(), 400.0), "width was {}", r.width());
        assert!(approx(r.height(), 300.0));
    }

    #[test]
    fn center_on_places_the_point_mid_viewport() {
        let mut c = cam();
        c.zoom = 1.25;
        c.center_on(Point::new(500.0, 400.0));
        let p = c.doc_to_screen(Point::new(500.0, 400.0));
        assert!((p.x - 400.0).abs() < 1e-6, "{p:?}");
        assert!((p.y - 300.0).abs() < 1e-6);
    }
}
