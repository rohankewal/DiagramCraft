//! Line terminators: the arrowheads, crow's feet, and UML glyphs drawn at
//! connector ends.
//!
//! Each marker is built in a local frame whose origin is the anchor point, with
//! `back` measuring distance away from the shape along the connector and `side`
//! measuring perpendicular offset. That keeps every glyph definition readable
//! as a set of 2D offsets regardless of the connector's direction.

use dc_model::{EndpointStyle, Point};
use kurbo::{BezPath, Vec2};

/// How a marker's interior is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerFill {
    /// Filled with the line's own color, like a plain arrowhead.
    Solid,
    /// Filled with the page background so the line does not show through,
    /// which is how UML draws inheritance and aggregation.
    Hollow,
    /// Stroked only, with nothing enclosed.
    Open,
}

/// A terminator ready to draw.
#[derive(Clone, Debug)]
pub struct Marker {
    pub path: BezPath,
    pub fill: MarkerFill,
}

/// Base glyph length at a hairline stroke, in points.
const BASE_LENGTH: f64 = 9.0;
/// Base half-width at a hairline stroke.
const BASE_HALF_WIDTH: f64 = 4.0;

/// The local frame of a terminator.
struct Frame {
    tip: Point,
    /// Unit vector pointing away from the shape, along the connector.
    back: Vec2,
    /// Unit vector perpendicular to `back`.
    side: Vec2,
}

impl Frame {
    fn new(tip: Point, outward: Vec2) -> Self {
        let back = if outward.hypot() < 1e-9 {
            Vec2::new(1.0, 0.0)
        } else {
            outward.normalize()
        };
        // Rotating 90 degrees clockwise keeps a right-handed frame under the
        // document's y-down convention.
        let side = Vec2::new(-back.y, back.x);
        Frame { tip, back, side }
    }

    /// Maps local offsets to a document-space point.
    fn at(&self, back: f64, side: f64) -> (f64, f64) {
        (
            self.tip.x + self.back.x * back + self.side.x * side,
            self.tip.y + self.back.y * back + self.side.y * side,
        )
    }
}

/// Builds the terminator for an endpoint, or `None` for a bare line end.
///
/// `outward` points away from the shape the endpoint attaches to, so the glyph
/// is drawn extending back along the connector with its tip at `tip`.
pub fn endpoint_marker(
    style: &EndpointStyle,
    tip: Point,
    outward: Vec2,
    stroke_width: f64,
) -> Option<Marker> {
    use EndpointStyle::*;

    let f = Frame::new(tip, outward);
    // Glyphs grow with stroke weight so a heavy line keeps proportion.
    let len = BASE_LENGTH.max(stroke_width * 4.0);
    let hw = BASE_HALF_WIDTH.max(stroke_width * 2.0);

    let marker = match style {
        None_ => return Option::None,
        Arrow => triangle(&f, len, hw, MarkerFill::Solid),
        HollowArrow | Generalization => triangle(&f, len, hw, MarkerFill::Hollow),
        OpenArrow => open_v(&f, len, hw),
        Composition => diamond(&f, len * 1.5, hw, MarkerFill::Solid),
        Aggregation => diamond(&f, len * 1.5, hw, MarkerFill::Hollow),
        BpmnConditional => diamond(&f, len * 1.3, hw * 0.75, MarkerFill::Hollow),
        ClosedSquare => square(&f, hw, MarkerFill::Solid),
        OpenSquare => square(&f, hw, MarkerFill::Hollow),
        ClosedCircle => circle(&f, hw, 0.0, MarkerFill::Solid),
        OpenCircle => circle(&f, hw, 0.0, MarkerFill::Hollow),
        Nesting => nesting(&f, hw),
        Async1 => single_barb(&f, len, hw),
        Async2 => open_v(&f, len, hw * 0.8),
        BpmnDefault => slash(&f, len, hw),

        // Crow's-foot cardinality notation.
        One => ticks(&f, hw, &[len * 0.7]),
        ExactlyOne => ticks(&f, hw, &[len * 0.55, len * 1.0]),
        Many => crows_foot(&f, len, hw, 0.0),
        OneOrMore => crows_foot_with(&f, len, hw, Decoration::Tick),
        ZeroOrMore => crows_foot_with(&f, len, hw, Decoration::Circle),
        ZeroOrOne => zero_or_one(&f, len, hw),

        // A style this build does not know. A plain arrow keeps the connector's
        // direction legible rather than silently dropping the terminator.
        Other(_) => triangle(&f, len, hw, MarkerFill::Solid),
    };

    Some(marker)
}

fn closed_polygon(f: &Frame, points: &[(f64, f64)], fill: MarkerFill) -> Marker {
    let mut p = BezPath::new();
    if let Some(&(b, s)) = points.first() {
        p.move_to(f.at(b, s));
        for &(b, s) in &points[1..] {
            p.line_to(f.at(b, s));
        }
        p.close_path();
    }
    Marker { path: p, fill }
}

fn triangle(f: &Frame, len: f64, hw: f64, fill: MarkerFill) -> Marker {
    closed_polygon(f, &[(0.0, 0.0), (len, hw), (len, -hw)], fill)
}

fn diamond(f: &Frame, len: f64, hw: f64, fill: MarkerFill) -> Marker {
    closed_polygon(
        f,
        &[(0.0, 0.0), (len / 2.0, hw), (len, 0.0), (len / 2.0, -hw)],
        fill,
    )
}

fn square(f: &Frame, hw: f64, fill: MarkerFill) -> Marker {
    let d = hw * 2.0;
    closed_polygon(f, &[(0.0, -hw), (0.0, hw), (d, hw), (d, -hw)], fill)
}

fn circle(f: &Frame, radius: f64, back_offset: f64, fill: MarkerFill) -> Marker {
    // A circle from four cubic arcs, centered one radius back from the tip so
    // it sits tangent to the anchor.
    const K: f64 = 0.552_284_749_830_793_4;
    let c = back_offset + radius;
    let mut p = BezPath::new();
    p.move_to(f.at(c - radius, 0.0));
    p.curve_to(
        f.at(c - radius, radius * K),
        f.at(c - radius * K, radius),
        f.at(c, radius),
    );
    p.curve_to(
        f.at(c + radius * K, radius),
        f.at(c + radius, radius * K),
        f.at(c + radius, 0.0),
    );
    p.curve_to(
        f.at(c + radius, -radius * K),
        f.at(c + radius * K, -radius),
        f.at(c, -radius),
    );
    p.curve_to(
        f.at(c - radius * K, -radius),
        f.at(c - radius, -radius * K),
        f.at(c - radius, 0.0),
    );
    p.close_path();
    Marker { path: p, fill }
}

/// An open arrowhead: two barbs meeting at the tip.
fn open_v(f: &Frame, len: f64, hw: f64) -> Marker {
    let mut p = BezPath::new();
    p.move_to(f.at(len, hw));
    p.line_to(f.at(0.0, 0.0));
    p.line_to(f.at(len, -hw));
    Marker { path: p, fill: MarkerFill::Open }
}

/// A single barb, used for an asynchronous message.
fn single_barb(f: &Frame, len: f64, hw: f64) -> Marker {
    let mut p = BezPath::new();
    p.move_to(f.at(len, hw));
    p.line_to(f.at(0.0, 0.0));
    Marker { path: p, fill: MarkerFill::Open }
}

/// A stroke across the line, as BPMN uses to mark a default flow.
fn slash(f: &Frame, len: f64, hw: f64) -> Marker {
    let mut p = BezPath::new();
    p.move_to(f.at(len * 0.4, -hw));
    p.line_to(f.at(len * 1.1, hw));
    Marker { path: p, fill: MarkerFill::Open }
}

/// Perpendicular ticks at the given distances back from the tip.
fn ticks(f: &Frame, hw: f64, positions: &[f64]) -> Marker {
    let mut p = BezPath::new();
    for &back in positions {
        p.move_to(f.at(back, -hw));
        p.line_to(f.at(back, hw));
    }
    Marker { path: p, fill: MarkerFill::Open }
}

/// The three splayed lines of a many-cardinality crow's foot.
fn crows_foot(f: &Frame, len: f64, hw: f64, origin_back: f64) -> Marker {
    let mut p = BezPath::new();
    let tip = origin_back;
    let spread = origin_back + len;
    for side in [-hw, 0.0, hw] {
        p.move_to(f.at(tip, 0.0));
        p.line_to(f.at(spread, side));
    }
    Marker { path: p, fill: MarkerFill::Open }
}

enum Decoration {
    Tick,
    Circle,
}

/// A crow's foot with a tick or circle behind it, for one-or-more and
/// zero-or-more.
fn crows_foot_with(f: &Frame, len: f64, hw: f64, decoration: Decoration) -> Marker {
    let mut marker = crows_foot(f, len, hw, 0.0);
    match decoration {
        Decoration::Tick => {
            let back = len * 1.45;
            marker.path.move_to(f.at(back, -hw));
            marker.path.line_to(f.at(back, hw));
        }
        Decoration::Circle => {
            let c = circle(f, hw * 0.65, len * 1.15, MarkerFill::Hollow);
            marker.path.extend(c.path.iter());
        }
    }
    marker
}

/// A circle and a tick, for zero-or-one.
fn zero_or_one(f: &Frame, len: f64, hw: f64) -> Marker {
    let mut marker = circle(f, hw * 0.65, len * 0.15, MarkerFill::Hollow);
    marker.fill = MarkerFill::Open;
    let back = len * 1.25;
    marker.path.move_to(f.at(back, -hw));
    marker.path.line_to(f.at(back, hw));
    marker
}

/// A circle crossed by a plus, the UML nesting glyph.
fn nesting(f: &Frame, hw: f64) -> Marker {
    let r = hw * 0.9;
    let mut marker = circle(f, r, 0.0, MarkerFill::Open);
    let c = r;
    marker.path.move_to(f.at(c - r, 0.0));
    marker.path.line_to(f.at(c + r, 0.0));
    marker.path.move_to(f.at(c, -r));
    marker.path.line_to(f.at(c, r));
    marker
}

// `None` is a variant name on EndpointStyle; alias it so the match above reads
// clearly without shadowing Option::None.
use EndpointStyle::None as None_;

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    const EAST: Vec2 = Vec2::new(1.0, 0.0);

    fn marker_for(style: &str) -> Option<Marker> {
        endpoint_marker(
            &EndpointStyle::from(style),
            Point::new(100.0, 50.0),
            EAST,
            1.0,
        )
    }

    #[test]
    fn none_draws_nothing() {
        assert!(marker_for("none").is_none());
    }

    #[test]
    fn every_other_published_style_draws_something() {
        for style in EndpointStyle::KNOWN {
            if *style == "none" {
                continue;
            }
            let m = marker_for(style).unwrap_or_else(|| panic!("{style} produced no marker"));
            assert!(!m.path.is_empty(), "{style} produced an empty path");
        }
    }

    #[test]
    fn unknown_style_falls_back_to_an_arrow() {
        let m = marker_for("futureGlyph").expect("fallback marker");
        assert_eq!(m.fill, MarkerFill::Solid);
    }

    #[test]
    fn markers_start_at_the_anchor_and_extend_backward() {
        // With `outward` pointing east, a glyph must occupy x >= the anchor.
        for style in EndpointStyle::KNOWN {
            if *style == "none" {
                continue;
            }
            let m = marker_for(style).unwrap();
            let b = m.path.bounding_box();
            assert!(
                b.x1 >= 100.0 - 0.01,
                "{style} drew behind the anchor: {b:?}"
            );
            assert!(
                b.x0 >= 100.0 - 0.01,
                "{style} crossed to the shape side of the anchor: {b:?}"
            );
        }
    }

    #[test]
    fn direction_rotates_the_glyph() {
        let east = endpoint_marker(&EndpointStyle::Arrow, Point::new(0.0, 0.0), EAST, 1.0)
            .unwrap()
            .path
            .bounding_box();
        let south = endpoint_marker(
            &EndpointStyle::Arrow,
            Point::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            1.0,
        )
        .unwrap()
        .path
        .bounding_box();

        assert!(east.width() > east.height(), "an east arrow is wider than tall");
        assert!(south.height() > south.width(), "a south arrow is taller than wide");
    }

    #[test]
    fn arrow_tip_sits_exactly_on_the_anchor() {
        let tip = Point::new(100.0, 50.0);
        let m = endpoint_marker(&EndpointStyle::Arrow, tip, EAST, 1.0).unwrap();
        let b = m.path.bounding_box();
        assert!((b.x0 - tip.x).abs() < 0.01, "tip drifted: {b:?}");
        // The barbs straddle the axis evenly.
        assert!((b.center().y - tip.y).abs() < 0.01);
    }

    #[test]
    fn glyphs_scale_with_stroke_width() {
        let thin = endpoint_marker(&EndpointStyle::Arrow, Point::ORIGIN, EAST, 1.0)
            .unwrap()
            .path
            .bounding_box();
        let thick = endpoint_marker(&EndpointStyle::Arrow, Point::ORIGIN, EAST, 6.0)
            .unwrap()
            .path
            .bounding_box();
        assert!(thick.width() > thin.width() * 2.0, "{thick:?} vs {thin:?}");
    }

    #[test]
    fn uml_fills_match_their_notation() {
        assert_eq!(marker_for("arrow").unwrap().fill, MarkerFill::Solid);
        assert_eq!(marker_for("composition").unwrap().fill, MarkerFill::Solid);
        assert_eq!(marker_for("hollowArrow").unwrap().fill, MarkerFill::Hollow);
        assert_eq!(marker_for("generalization").unwrap().fill, MarkerFill::Hollow);
        assert_eq!(marker_for("aggregation").unwrap().fill, MarkerFill::Hollow);
        assert_eq!(marker_for("openArrow").unwrap().fill, MarkerFill::Open);
    }

    #[test]
    fn crows_feet_are_open_strokes() {
        for style in ["many", "oneOrMore", "zeroOrMore", "one", "exactlyOne", "zeroOrOne"] {
            assert_eq!(
                marker_for(style).unwrap().fill,
                MarkerFill::Open,
                "{style} should be stroked, not filled"
            );
        }
    }

    #[test]
    fn zero_degenerate_direction_does_not_panic() {
        let m = endpoint_marker(
            &EndpointStyle::Arrow,
            Point::ORIGIN,
            Vec2::new(0.0, 0.0),
            1.0,
        );
        assert!(m.is_some(), "a degenerate direction should still produce a glyph");
    }
}
