//! Positions and bounding boxes.
//!
//! The document coordinate system has +x to the right and +y downward, with
//! rotation measured clockwise in degrees.

use serde::{Deserialize, Serialize};

/// An absolute point in document units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ORIGIN: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance_to(self, other: Point) -> f64 {
        let (dx, dy) = (other.x - self.x, other.y - self.y);
        (dx * dx + dy * dy).sqrt()
    }

    pub fn lerp(self, other: Point, t: f64) -> Point {
        Point::new(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }

    /// Rotates clockwise around `origin` by `degrees`, matching the document's
    /// rotation convention.
    pub fn rotate_about(self, origin: Point, degrees: f64) -> Point {
        let r = degrees.to_radians();
        let (sin, cos) = r.sin_cos();
        let (dx, dy) = (self.x - origin.x, self.y - origin.y);
        Point::new(
            origin.x + dx * cos - dy * sin,
            origin.y + dx * sin + dy * cos,
        )
    }
}

/// A fraction along a shape edge or line, nominally in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelPoint {
    pub x: f64,
    pub y: f64,
}

impl RelPoint {
    /// The shape center, which is the attachment default for a connector that
    /// does not pin itself to a specific edge position.
    pub const CENTER: RelPoint = RelPoint { x: 0.5, y: 0.5 };

    pub const fn new(x: f64, y: f64) -> Self {
        RelPoint { x, y }
    }
}

/// An axis-aligned box plus an optional clockwise rotation about its center.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
}

impl BoundingBox {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        BoundingBox { x, y, w, h, rotation: None }
    }

    pub fn with_rotation(mut self, degrees: f64) -> Self {
        self.rotation = if degrees == 0.0 { None } else { Some(degrees) };
        self
    }

    pub fn rotation_degrees(&self) -> f64 {
        self.rotation.unwrap_or(0.0)
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// Maps a relative position to document space, honoring rotation.
    pub fn point_at(&self, rel: RelPoint) -> Point {
        let p = Point::new(self.x + self.w * rel.x, self.y + self.h * rel.y);
        match self.rotation {
            Some(deg) if deg != 0.0 => p.rotate_about(self.center(), deg),
            _ => p,
        }
    }

    /// The four corners in document space, clockwise from the top-left.
    pub fn corners(&self) -> [Point; 4] {
        [
            self.point_at(RelPoint::new(0.0, 0.0)),
            self.point_at(RelPoint::new(1.0, 0.0)),
            self.point_at(RelPoint::new(1.0, 1.0)),
            self.point_at(RelPoint::new(0.0, 1.0)),
        ]
    }

    /// Unrotated hit test. Callers handling rotated shapes should transform the
    /// point into shape space first.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    /// Hit test that accounts for rotation.
    pub fn contains_rotated(&self, p: Point) -> bool {
        match self.rotation {
            Some(deg) if deg != 0.0 => {
                self.contains(p.rotate_about(self.center(), -deg))
            }
            _ => self.contains(p),
        }
    }

    /// The axis-aligned box that encloses this box after rotation. This is what
    /// culling and multi-select marquees need.
    pub fn aabb(&self) -> BoundingBox {
        match self.rotation {
            Some(deg) if deg != 0.0 => {
                let c = self.corners();
                let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
                let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
                for p in c {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                BoundingBox::new(min_x, min_y, max_x - min_x, max_y - min_y)
            }
            _ => BoundingBox::new(self.x, self.y, self.w, self.h),
        }
    }

    pub fn intersects(&self, other: &BoundingBox) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    pub fn translated(&self, dx: f64, dy: f64) -> BoundingBox {
        BoundingBox { x: self.x + dx, y: self.y + dy, ..*self }
    }

    pub fn inflated(&self, by: f64) -> BoundingBox {
        BoundingBox {
            x: self.x - by,
            y: self.y - by,
            w: self.w + by * 2.0,
            h: self.h + by * 2.0,
            ..*self
        }
    }

    /// The smallest box containing both inputs, ignoring rotation.
    pub fn union(&self, other: &BoundingBox) -> BoundingBox {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        BoundingBox::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    /// Normalizes a box dragged out right-to-left or bottom-to-top.
    pub fn from_corners(a: Point, b: Point) -> BoundingBox {
        BoundingBox::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (b.x - a.x).abs(),
            (b.y - a.y).abs(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn documented_example_parses() {
        // Verbatim from the published reference.
        let bb: BoundingBox =
            serde_json::from_str(r#"{"x":5,"y":3,"w":10,"h":15,"rotation":180}"#).unwrap();
        assert_eq!(bb.x, 5.0);
        assert_eq!(bb.w, 10.0);
        assert_eq!(bb.rotation, Some(180.0));
    }

    #[test]
    fn omits_absent_rotation_on_write() {
        let json = serde_json::to_string(&BoundingBox::new(1.0, 2.0, 3.0, 4.0)).unwrap();
        assert_eq!(json, r#"{"x":1.0,"y":2.0,"w":3.0,"h":4.0}"#);
    }

    #[test]
    fn point_at_maps_relative_to_absolute() {
        let bb = BoundingBox::new(10.0, 20.0, 100.0, 50.0);
        let p = bb.point_at(RelPoint::new(0.5, 1.0));
        assert!(approx(p.x, 60.0) && approx(p.y, 70.0));
    }

    #[test]
    fn rotation_is_clockwise() {
        // Rotating the right-middle edge by 90 degrees clockwise should land it
        // at the bottom-middle, given +y is down.
        let bb = BoundingBox::new(0.0, 0.0, 10.0, 10.0).with_rotation(90.0);
        let p = bb.point_at(RelPoint::new(1.0, 0.5));
        assert!(approx(p.x, 5.0), "x was {}", p.x);
        assert!(approx(p.y, 10.0), "y was {}", p.y);
    }

    #[test]
    fn aabb_grows_for_rotated_box() {
        let bb = BoundingBox::new(0.0, 0.0, 10.0, 10.0).with_rotation(45.0);
        let aabb = bb.aabb();
        let expected = 10.0 * 2f64.sqrt();
        assert!(approx(aabb.w, expected), "w was {}", aabb.w);
        assert!(aabb.x < 0.0);
    }

    #[test]
    fn rotated_hit_test_rejects_unrotated_corner() {
        let bb = BoundingBox::new(0.0, 0.0, 10.0, 10.0).with_rotation(45.0);
        // The original top-left corner rotates away from this point.
        assert!(!bb.contains_rotated(Point::new(0.2, 0.2)));
        assert!(bb.contains_rotated(bb.center()));
    }

    #[test]
    fn from_corners_normalizes_reverse_drag() {
        let bb = BoundingBox::from_corners(Point::new(10.0, 10.0), Point::new(0.0, 4.0));
        assert_eq!((bb.x, bb.y, bb.w, bb.h), (0.0, 4.0, 10.0, 6.0));
    }
}
