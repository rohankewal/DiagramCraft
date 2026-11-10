//! Selection handles.
//!
//! Handle geometry is shared between painting and hit testing so the two can
//! never disagree about where a handle is.

use dc_model::{BoundingBox, Point};

/// A grab point on a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleKind {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleRight,
    BottomRight,
    BottomCenter,
    BottomLeft,
    MiddleLeft,
    /// The arm above the selection that spins it.
    Rotate,
}

impl HandleKind {
    /// The eight resize handles, clockwise from the top-left.
    pub const RESIZE: [HandleKind; 8] = [
        HandleKind::TopLeft,
        HandleKind::TopCenter,
        HandleKind::TopRight,
        HandleKind::MiddleRight,
        HandleKind::BottomRight,
        HandleKind::BottomCenter,
        HandleKind::BottomLeft,
        HandleKind::MiddleLeft,
    ];

    /// Which edges this handle moves, as `(left, top, right, bottom)`.
    pub fn edges(self) -> (bool, bool, bool, bool) {
        use HandleKind::*;
        match self {
            TopLeft => (true, true, false, false),
            TopCenter => (false, true, false, false),
            TopRight => (false, true, true, false),
            MiddleRight => (false, false, true, false),
            BottomRight => (false, false, true, true),
            BottomCenter => (false, false, false, true),
            BottomLeft => (true, false, false, true),
            MiddleLeft => (true, false, false, false),
            Rotate => (false, false, false, false),
        }
    }

    /// Whether dragging this handle changes both dimensions, which is when
    /// proportional resizing applies.
    pub fn is_corner(self) -> bool {
        use HandleKind::*;
        matches!(self, TopLeft | TopRight | BottomRight | BottomLeft)
    }

    /// A name for the resize cursor this handle should show.
    pub fn cursor(self) -> &'static str {
        use HandleKind::*;
        match self {
            TopLeft | BottomRight => "nwse-resize",
            TopRight | BottomLeft => "nesw-resize",
            TopCenter | BottomCenter => "ns-resize",
            MiddleLeft | MiddleRight => "ew-resize",
            Rotate => "grab",
        }
    }
}

/// How far above the selection the rotation arm sits, in screen pixels.
pub const ROTATE_ARM_LENGTH: f64 = 22.0;

/// The position of one handle on a screen-space selection box.
pub fn handle_position(bounds: BoundingBox, kind: HandleKind) -> Point {
    use HandleKind::*;
    let (x0, y0) = (bounds.x, bounds.y);
    let (x1, y1) = (bounds.right(), bounds.bottom());
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;

    match kind {
        TopLeft => Point::new(x0, y0),
        TopCenter => Point::new(cx, y0),
        TopRight => Point::new(x1, y0),
        MiddleRight => Point::new(x1, cy),
        BottomRight => Point::new(x1, y1),
        BottomCenter => Point::new(cx, y1),
        BottomLeft => Point::new(x0, y1),
        MiddleLeft => Point::new(x0, cy),
        Rotate => Point::new(cx, y0 - ROTATE_ARM_LENGTH),
    }
}

/// Every handle on a selection box, rotation arm last.
pub fn all_handles(bounds: BoundingBox) -> Vec<(HandleKind, Point)> {
    HandleKind::RESIZE
        .iter()
        .copied()
        .chain(std::iter::once(HandleKind::Rotate))
        .map(|k| (k, handle_position(bounds, k)))
        .collect()
}

/// Finds the handle under a screen-space point.
///
/// The rotation arm is tested first so it wins where it overlaps the top-center
/// handle on a very short selection.
pub fn hit_test(bounds: BoundingBox, point: Point, handle_size: f64, slop: f64) -> Option<HandleKind> {
    let reach = handle_size / 2.0 + slop;
    let candidates = std::iter::once(HandleKind::Rotate).chain(HandleKind::RESIZE);

    for kind in candidates {
        let p = handle_position(bounds, kind);
        if (point.x - p.x).abs() <= reach && (point.y - p.y).abs() <= reach {
            return Some(kind);
        }
    }
    None
}

/// Applies a handle drag to a box, returning the new box.
///
/// `proportional` locks the aspect ratio, which corner handles do when shift is
/// held. The result is normalized, so dragging an edge past its opposite flips
/// the box rather than producing negative dimensions.
pub fn resize(
    original: BoundingBox,
    kind: HandleKind,
    dx: f64,
    dy: f64,
    proportional: bool,
) -> BoundingBox {
    let (left, top, right, bottom) = kind.edges();
    let mut x0 = original.x;
    let mut y0 = original.y;
    let mut x1 = original.right();
    let mut y1 = original.bottom();

    let (mut dx, mut dy) = (dx, dy);
    if proportional && kind.is_corner() && original.w > 0.0 && original.h > 0.0 {
        // Follow whichever axis the pointer moved further along, so the drag
        // feels like it tracks the cursor.
        let aspect = original.w / original.h;
        if dx.abs() * original.h >= dy.abs() * original.w {
            dy = dx / aspect * if left == top { 1.0 } else { -1.0 };
        } else {
            dx = dy * aspect * if left == top { 1.0 } else { -1.0 };
        }
    }

    if left {
        x0 += dx;
    }
    if right {
        x1 += dx;
    }
    if top {
        y0 += dy;
    }
    if bottom {
        y1 += dy;
    }

    BoundingBox {
        x: x0.min(x1),
        y: y0.min(y1),
        w: (x1 - x0).abs(),
        h: (y1 - y0).abs(),
        rotation: original.rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> BoundingBox {
        BoundingBox::new(100.0, 200.0, 400.0, 300.0)
    }

    #[test]
    fn handles_sit_on_the_box_corners_and_edges() {
        let b = bounds();
        assert_eq!(handle_position(b, HandleKind::TopLeft), Point::new(100.0, 200.0));
        assert_eq!(handle_position(b, HandleKind::BottomRight), Point::new(500.0, 500.0));
        assert_eq!(handle_position(b, HandleKind::TopCenter), Point::new(300.0, 200.0));
        assert_eq!(handle_position(b, HandleKind::MiddleLeft), Point::new(100.0, 350.0));
    }

    #[test]
    fn rotate_handle_sits_above_the_box() {
        let p = handle_position(bounds(), HandleKind::Rotate);
        assert_eq!(p, Point::new(300.0, 200.0 - ROTATE_ARM_LENGTH));
    }

    #[test]
    fn there_are_nine_handles() {
        assert_eq!(all_handles(bounds()).len(), 9);
    }

    #[test]
    fn hit_test_finds_each_handle_at_its_own_position() {
        let b = bounds();
        for (kind, p) in all_handles(b) {
            assert_eq!(hit_test(b, p, 8.0, 4.0), Some(kind), "missed {kind:?}");
        }
    }

    #[test]
    fn hit_test_respects_the_slop_radius() {
        let b = bounds();
        // 8px handle plus 4px slop reaches 8 units from the center.
        assert_eq!(
            hit_test(b, Point::new(107.0, 200.0), 8.0, 4.0),
            Some(HandleKind::TopLeft)
        );
        assert_eq!(hit_test(b, Point::new(120.0, 200.0), 8.0, 4.0), None);
    }

    #[test]
    fn hit_test_misses_the_box_interior() {
        assert_eq!(hit_test(bounds(), Point::new(300.0, 350.0), 8.0, 4.0), None);
    }

    #[test]
    fn rotate_wins_over_top_center_when_they_overlap() {
        // A zero-height selection puts both near the same place; rotation is
        // tested first so it stays reachable.
        let flat = BoundingBox::new(0.0, 0.0, 100.0, 0.0);
        let p = handle_position(flat, HandleKind::Rotate);
        assert_eq!(hit_test(flat, p, 8.0, 4.0), Some(HandleKind::Rotate));
    }

    #[test]
    fn dragging_a_corner_moves_two_edges() {
        let r = resize(bounds(), HandleKind::BottomRight, 50.0, 25.0, false);
        assert_eq!((r.x, r.y, r.w, r.h), (100.0, 200.0, 450.0, 325.0));
    }

    #[test]
    fn dragging_an_edge_moves_only_that_edge() {
        let r = resize(bounds(), HandleKind::MiddleLeft, 40.0, 999.0, false);
        assert_eq!((r.x, r.y, r.w, r.h), (140.0, 200.0, 360.0, 300.0));
    }

    #[test]
    fn dragging_past_the_opposite_edge_flips_the_box() {
        // Pull the left edge 500 units right, past the right edge at 500.
        let r = resize(bounds(), HandleKind::MiddleLeft, 500.0, 0.0, false);
        assert_eq!(r.x, 500.0);
        assert_eq!(r.w, 100.0, "width stays positive");
    }

    #[test]
    fn proportional_resize_preserves_aspect_ratio() {
        let b = BoundingBox::new(0.0, 0.0, 400.0, 200.0);
        let r = resize(b, HandleKind::BottomRight, 100.0, 5.0, true);
        let before = b.w / b.h;
        let after = r.w / r.h;
        assert!((before - after).abs() < 1e-9, "{before} became {after}");
    }

    #[test]
    fn proportional_resize_is_ignored_on_edge_handles() {
        let b = BoundingBox::new(0.0, 0.0, 400.0, 200.0);
        let r = resize(b, HandleKind::MiddleRight, 100.0, 0.0, true);
        assert_eq!(r.h, 200.0, "an edge handle changes one dimension only");
        assert_eq!(r.w, 500.0);
    }

    #[test]
    fn resize_preserves_rotation() {
        let b = BoundingBox::new(0.0, 0.0, 100.0, 100.0).with_rotation(30.0);
        let r = resize(b, HandleKind::BottomRight, 10.0, 10.0, false);
        assert_eq!(r.rotation, Some(30.0));
    }

    #[test]
    fn corner_handles_are_classified() {
        assert!(HandleKind::TopLeft.is_corner());
        assert!(!HandleKind::TopCenter.is_corner());
        assert!(!HandleKind::Rotate.is_corner());
    }

    #[test]
    fn opposite_corners_share_a_cursor() {
        assert_eq!(HandleKind::TopLeft.cursor(), HandleKind::BottomRight.cursor());
        assert_eq!(HandleKind::TopRight.cursor(), HandleKind::BottomLeft.cursor());
    }
}
