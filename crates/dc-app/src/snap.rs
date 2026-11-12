//! Alignment snapping.
//!
//! While a shape is dragged, its left, center, and right edges are compared
//! against the same three positions on every other shape. The closest match
//! within the threshold wins on each axis independently, so a shape can snap
//! horizontally to one neighbor and vertically to another.

use dc_model::BoundingBox;
use dc_render::SnapGuide;

/// The adjustment to apply to a drag, plus the guides to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapResult {
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<SnapGuide>,
}

impl SnapResult {
    /// Whether anything aligned. Callers use this to decide between an
    /// alignment snap and a grid snap.
    pub fn is_snapped(&self) -> bool {
        !self.guides.is_empty()
    }
}

/// The three horizontal reference positions on a box.
fn x_edges(b: &BoundingBox) -> [f64; 3] {
    [b.x, b.x + b.w / 2.0, b.right()]
}

/// The three vertical reference positions on a box.
fn y_edges(b: &BoundingBox) -> [f64; 3] {
    [b.y, b.y + b.h / 2.0, b.bottom()]
}

/// Finds the nearest alignment for `moving` among `others`.
///
/// `threshold` is in the same units as the boxes, so callers working in screen
/// pixels should convert it through the camera first.
pub fn snap_box(moving: BoundingBox, others: &[BoundingBox], threshold: f64) -> SnapResult {
    let mut result = SnapResult::default();
    if threshold <= 0.0 || others.is_empty() {
        return result;
    }

    let mut best_x: Option<(f64, f64)> = None; // (distance, target)
    let mut best_y: Option<(f64, f64)> = None;

    for other in others {
        for mine in x_edges(&moving) {
            for theirs in x_edges(other) {
                let d = theirs - mine;
                if d.abs() <= threshold && best_x.as_ref().is_none_or(|(b, _)| d.abs() < *b) {
                    best_x = Some((d.abs(), theirs));
                    result.dx = d;
                }
            }
        }
        for mine in y_edges(&moving) {
            for theirs in y_edges(other) {
                let d = theirs - mine;
                if d.abs() <= threshold && best_y.as_ref().is_none_or(|(b, _)| d.abs() < *b) {
                    best_y = Some((d.abs(), theirs));
                    result.dy = d;
                }
            }
        }
    }

    if let Some((_, at)) = best_x {
        result.guides.push(SnapGuide { at, vertical: true });
    } else {
        result.dx = 0.0;
    }
    if let Some((_, at)) = best_y {
        result.guides.push(SnapGuide { at, vertical: false });
    } else {
        result.dy = 0.0;
    }

    result
}

/// Finds the nearest placed guide for `moving`.
///
/// Guides take priority over alignment with other shapes: an author who put
/// a guide down said where they want things, which is more specific than the
/// incidental fact that two shapes happen to line up.
pub fn snap_box_to_guides(
    moving: BoundingBox,
    guides: &[dc_model::Guide],
    threshold: f64,
) -> SnapResult {
    let mut result = SnapResult::default();
    if threshold <= 0.0 || guides.is_empty() {
        return result;
    }

    let mut best_x: Option<f64> = None;
    let mut best_y: Option<f64> = None;

    for guide in guides {
        let (mine, best, delta) = if guide.vertical {
            (x_edges(&moving), &mut best_x, &mut result.dx)
        } else {
            (y_edges(&moving), &mut best_y, &mut result.dy)
        };
        for edge in mine {
            let d = guide.at - edge;
            if d.abs() <= threshold && best.is_none_or(|b: f64| d.abs() < b) {
                *best = Some(d.abs());
                *delta = d;
            }
        }
    }

    for guide in guides {
        let hit = if guide.vertical { best_x } else { best_y };
        let delta = if guide.vertical { result.dx } else { result.dy };
        if hit.is_some()
            && x_edges(&moving)
                .iter()
                .chain(y_edges(&moving).iter())
                .any(|e| ((e + delta) - guide.at).abs() < 1e-6)
        {
            result.guides.push(SnapGuide { at: guide.at, vertical: guide.vertical });
        }
    }
    result.guides.dedup();

    if best_x.is_none() {
        result.dx = 0.0;
    }
    if best_y.is_none() {
        result.dy = 0.0;
    }
    result
}

/// Rounds a value to the nearest multiple of `pitch`, for grid snapping.
pub fn snap_to_grid(value: f64, pitch: f64) -> f64 {
    if pitch <= 0.0 {
        return value;
    }
    (value / pitch).round() * pitch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f64, y: f64, w: f64, h: f64) -> BoundingBox {
        BoundingBox::new(x, y, w, h)
    }

    #[test]
    fn no_neighbors_means_no_snap() {
        let r = snap_box(b(0.0, 0.0, 10.0, 10.0), &[], 5.0);
        assert!(!r.is_snapped());
        assert_eq!((r.dx, r.dy), (0.0, 0.0));
    }

    #[test]
    fn left_edges_align() {
        // Moving box's left edge is 3 units from the neighbor's left edge.
        let moving = b(103.0, 500.0, 50.0, 50.0);
        let other = b(100.0, 0.0, 50.0, 50.0);
        let r = snap_box(moving, &[other], 5.0);

        assert_eq!(r.dx, -3.0);
        assert!(r.guides.contains(&SnapGuide { at: 100.0, vertical: true }));
    }

    #[test]
    fn centers_align() {
        // Centers at 125 and 128; 3 units apart.
        let moving = b(103.0, 500.0, 50.0, 50.0);
        let other = b(100.0, 0.0, 50.0, 50.0);
        let r = snap_box(moving, &[other], 5.0);
        // Left-edge alignment is equally close, so some x snap must occur.
        assert_ne!(r.dx, 0.0);
    }

    #[test]
    fn right_edge_snaps_to_a_neighbors_left_edge() {
        // Moving right edge at 98, neighbor left edge at 100.
        let moving = b(48.0, 0.0, 50.0, 20.0);
        let other = b(100.0, 0.0, 50.0, 20.0);
        let r = snap_box(moving, &[other], 5.0);
        assert_eq!(r.dx, 2.0, "should close the 2-unit gap");
    }

    #[test]
    fn beyond_the_threshold_nothing_snaps() {
        // Offset on both axes; sharing a y coordinate would legitimately snap.
        let moving = b(140.0, 140.0, 50.0, 50.0);
        let other = b(0.0, 0.0, 50.0, 50.0);
        let r = snap_box(moving, &[other], 5.0);
        assert!(!r.is_snapped(), "got {r:?}");
    }

    #[test]
    fn already_aligned_edges_still_report_a_guide() {
        // A zero-distance match is worth showing: it tells the user why the
        // shape is not moving freely.
        let moving = b(400.0, 0.0, 50.0, 50.0);
        let other = b(0.0, 0.0, 50.0, 50.0);
        let r = snap_box(moving, &[other], 5.0);
        assert_eq!(r.dy, 0.0);
        assert!(r.guides.contains(&SnapGuide { at: 0.0, vertical: false }));
    }

    #[test]
    fn axes_snap_independently() {
        let moving = b(102.0, 302.0, 50.0, 50.0);
        let left_neighbor = b(100.0, 0.0, 50.0, 50.0);
        let top_neighbor = b(900.0, 300.0, 50.0, 50.0);

        let r = snap_box(moving, &[left_neighbor, top_neighbor], 5.0);
        assert_eq!(r.dx, -2.0);
        assert_eq!(r.dy, -2.0);
        assert_eq!(r.guides.len(), 2, "one guide per axis");
    }

    #[test]
    fn the_nearest_candidate_wins() {
        let moving = b(104.0, 0.0, 50.0, 20.0);
        let far = b(100.0, 0.0, 50.0, 20.0); // 4 units away
        let near = b(105.0, 0.0, 50.0, 20.0); // 1 unit away

        let r = snap_box(moving, &[far, near], 6.0);
        assert_eq!(r.dx, 1.0, "should prefer the closer edge");
    }

    #[test]
    fn a_zero_threshold_disables_snapping() {
        let r = snap_box(b(100.0, 0.0, 10.0, 10.0), &[b(100.0, 0.0, 10.0, 10.0)], 0.0);
        assert!(!r.is_snapped());
    }

    #[test]
    fn one_axis_snapping_leaves_the_other_untouched() {
        let moving = b(102.0, 777.0, 50.0, 50.0);
        let other = b(100.0, 0.0, 50.0, 50.0);
        let r = snap_box(moving, &[other], 5.0);

        assert_eq!(r.dx, -2.0);
        assert_eq!(r.dy, 0.0, "no vertical candidate was in range");
        assert_eq!(r.guides.len(), 1);
    }

    #[test]
    fn grid_snapping_rounds_to_the_pitch() {
        assert_eq!(snap_to_grid(13.0, 9.0), 9.0);
        assert_eq!(snap_to_grid(14.0, 9.0), 18.0);
        assert_eq!(snap_to_grid(-13.0, 9.0), -9.0);
        assert_eq!(snap_to_grid(7.5, 0.0), 7.5, "a zero pitch is a no-op");
    }
}
