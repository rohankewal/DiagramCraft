//! Lines, the connectors between shapes.

use crate::geom::{Point, RelPoint};
use crate::string_enum::string_enum;
use crate::style::Stroke;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How a line is routed between its endpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineType {
    /// A direct segment between endpoints.
    Straight,
    /// Right-angle segments. This is the conventional flowchart routing and the
    /// editor's default.
    #[default]
    Elbow,
    /// A smooth curve between endpoints.
    Curved,
}

string_enum! {
    /// A terminator drawn at a line end.
    ///
    /// These are the styles named in the published specification. The spec
    /// states twenty-four exist but enumerates the following; any value it does
    /// not name is preserved through `Other`.
    EndpointStyle {
        None => "none",
        Arrow => "arrow",
        HollowArrow => "hollowArrow",
        OpenArrow => "openArrow",
        Aggregation => "aggregation",
        Composition => "composition",
        Generalization => "generalization",
        Async1 => "async1",
        Async2 => "async2",
        ClosedSquare => "closedSquare",
        OpenSquare => "openSquare",
        ClosedCircle => "closedCircle",
        OpenCircle => "openCircle",
        BpmnConditional => "bpmnConditional",
        BpmnDefault => "bpmnDefault",
        ExactlyOne => "exactlyOne",
        Many => "many",
        Nesting => "nesting",
        One => "one",
        OneOrMore => "oneOrMore",
        ZeroOrMore => "zeroOrMore",
        ZeroOrOne => "zeroOrOne",
    }
}

impl Default for EndpointStyle {
    fn default() -> Self {
        EndpointStyle::None
    }
}

impl EndpointStyle {
    /// Whether the terminator's outline encloses an area that is filled.
    pub fn is_filled(&self) -> bool {
        matches!(
            self,
            EndpointStyle::Arrow
                | EndpointStyle::Composition
                | EndpointStyle::ClosedSquare
                | EndpointStyle::ClosedCircle
        )
    }

    /// How far the line should stop short of its anchor so the terminator does
    /// not overlap its own stroke, as a multiple of stroke width.
    pub fn inset_factor(&self) -> f64 {
        match self {
            EndpointStyle::None => 0.0,
            EndpointStyle::OpenArrow | EndpointStyle::Generalization => 0.0,
            EndpointStyle::Aggregation | EndpointStyle::Composition => 8.0,
            EndpointStyle::ClosedCircle | EndpointStyle::OpenCircle => 4.0,
            EndpointStyle::ClosedSquare | EndpointStyle::OpenSquare => 4.0,
            _ => 4.0,
        }
    }
}

/// Where a line end attaches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Endpoint {
    /// Attached to another line at a fraction along it.
    // `rename_all` on the enum renames variants; fields need their own.
    #[serde(rename_all = "camelCase")]
    LineEndpoint {
        #[serde(default, skip_serializing_if = "is_default_style")]
        style: EndpointStyle,
        line_id: String,
        /// A fraction from 0.0 to 1.0 along the target line.
        position: f64,
    },
    /// Attached to a shape. An absent `position` lets the editor pick the
    /// attachment point dynamically, which is the smart-line behavior.
    #[serde(rename_all = "camelCase")]
    ShapeEndpoint {
        #[serde(default, skip_serializing_if = "is_default_style")]
        style: EndpointStyle,
        shape_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<RelPoint>,
    },
    /// Pinned to a fixed point on the canvas.
    #[serde(rename_all = "camelCase")]
    PositionEndpoint {
        #[serde(default, skip_serializing_if = "is_default_style")]
        style: EndpointStyle,
        position: Point,
    },
}

fn is_default_style(s: &EndpointStyle) -> bool {
    *s == EndpointStyle::None
}

impl Endpoint {
    pub fn style(&self) -> &EndpointStyle {
        match self {
            Endpoint::LineEndpoint { style, .. }
            | Endpoint::ShapeEndpoint { style, .. }
            | Endpoint::PositionEndpoint { style, .. } => style,
        }
    }

    pub fn set_style(&mut self, new: EndpointStyle) {
        match self {
            Endpoint::LineEndpoint { style, .. }
            | Endpoint::ShapeEndpoint { style, .. }
            | Endpoint::PositionEndpoint { style, .. } => *style = new,
        }
    }

    /// The shape this end attaches to, if any.
    pub fn shape_id(&self) -> Option<&str> {
        match self {
            Endpoint::ShapeEndpoint { shape_id, .. } => Some(shape_id),
            _ => None,
        }
    }

    /// The line this end attaches to, if any.
    pub fn line_id(&self) -> Option<&str> {
        match self {
            Endpoint::LineEndpoint { line_id, .. } => Some(line_id),
            _ => None,
        }
    }

    pub fn at_shape(shape_id: impl Into<String>, style: EndpointStyle) -> Self {
        Endpoint::ShapeEndpoint { style, shape_id: shape_id.into(), position: None }
    }

    /// Attached to one fixed point on a shape rather than to whichever edge
    /// happens to face the other end.
    ///
    /// The routing layer honours the position and infers only the direction,
    /// so a pinned connector keeps its port as either shape moves.
    pub fn at_shape_point(
        shape_id: impl Into<String>,
        style: EndpointStyle,
        at: RelPoint,
    ) -> Self {
        Endpoint::ShapeEndpoint { style, shape_id: shape_id.into(), position: Some(at) }
    }

    /// The fixed attachment point, when this end is pinned to one.
    pub fn pinned_at(&self) -> Option<RelPoint> {
        match self {
            Endpoint::ShapeEndpoint { position, .. } => *position,
            _ => None,
        }
    }

    pub fn at_point(p: Point, style: EndpointStyle) -> Self {
        Endpoint::PositionEndpoint { style, position: p }
    }
}

/// Which side of the line a label sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelSide {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// A text label positioned along a line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineLabel {
    pub text: String,
    /// A fraction from 0.0 to 1.0 along the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<LabelSide>,
}

impl LineLabel {
    pub fn new(text: impl Into<String>) -> Self {
        LineLabel { text: text.into(), position: None, side: None }
    }

    pub fn resolved_position(&self) -> f64 {
        self.position.unwrap_or(0.5).clamp(0.0, 1.0)
    }

    pub fn resolved_side(&self) -> LabelSide {
        self.side.unwrap_or_default()
    }
}

/// A connector on a page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub id: String,
    pub line_type: LineType,
    pub endpoint1: Endpoint,
    pub endpoint2: Endpoint,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Vec<LineLabel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i64>,

    /// User-placed waypoints the route must pass through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joints: Option<Vec<Point>>,
    /// Control points for elbow routing. The published spec does not fix a
    /// shape for these, so they are carried through untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elbow_control_points: Option<Value>,

    /// Properties not modeled above, preserved for round-tripping.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Line {
    pub fn new(id: impl Into<String>, from: Endpoint, to: Endpoint) -> Self {
        Line {
            id: id.into(),
            line_type: LineType::default(),
            endpoint1: from,
            endpoint2: to,
            stroke: None,
            text: None,
            z_index: None,
            joints: None,
            elbow_control_points: None,
            extra: Map::new(),
        }
    }

    pub fn with_line_type(mut self, t: LineType) -> Self {
        self.line_type = t;
        self
    }

    pub fn with_label(mut self, text: impl Into<String>) -> Self {
        self.text.get_or_insert_with(Vec::new).push(LineLabel::new(text));
        self
    }

    pub fn resolved_stroke(&self) -> Stroke {
        self.stroke.unwrap_or_default()
    }

    pub fn resolved_z_index(&self) -> i64 {
        self.z_index.unwrap_or(0)
    }

    pub fn labels(&self) -> &[LineLabel] {
        self.text.as_deref().unwrap_or(&[])
    }

    /// Whether either end attaches to the given shape.
    pub fn touches_shape(&self, shape_id: &str) -> bool {
        self.endpoint1.shape_id() == Some(shape_id)
            || self.endpoint2.shape_id() == Some(shape_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example published in the lines reference, verbatim.
    const PUBLISHED_EXAMPLE: &str = r##"{
        "id": "line1",
        "lineType": "straight",
        "zIndex": 0,
        "endpoint1": {
            "type": "shapeEndpoint",
            "style": "none",
            "shapeId": "block3",
            "position": { "x": 1, "y": 1 }
        },
        "endpoint2": {
            "type": "lineEndpoint",
            "style": "arrow",
            "lineId": "line2",
            "position": 0.5
        },
        "stroke": {
            "color": "#000000",
            "width": 1,
            "style": "solid"
        },
        "text": [{
            "text": "test",
            "position": 0.5,
            "side": "middle"
        }]
    }"##;

    #[test]
    fn published_example_parses() {
        let l: Line = serde_json::from_str(PUBLISHED_EXAMPLE).unwrap();
        assert_eq!(l.id, "line1");
        assert_eq!(l.line_type, LineType::Straight);

        match &l.endpoint1 {
            Endpoint::ShapeEndpoint { shape_id, position, style } => {
                assert_eq!(shape_id, "block3");
                assert_eq!(*position, Some(RelPoint::new(1.0, 1.0)));
                assert_eq!(*style, EndpointStyle::None);
            }
            other => panic!("expected shapeEndpoint, got {other:?}"),
        }

        match &l.endpoint2 {
            Endpoint::LineEndpoint { line_id, position, style } => {
                assert_eq!(line_id, "line2");
                assert_eq!(*position, 0.5);
                assert_eq!(*style, EndpointStyle::Arrow);
            }
            other => panic!("expected lineEndpoint, got {other:?}"),
        }

        let labels = l.labels();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].text, "test");
        assert_eq!(labels[0].resolved_side(), LabelSide::Middle);
    }

    #[test]
    fn published_example_round_trips_semantically() {
        let l: Line = serde_json::from_str(PUBLISHED_EXAMPLE).unwrap();
        let again: Line = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l, again);
    }

    #[test]
    fn endpoint_tag_is_emitted() {
        let e = Endpoint::at_point(Point::new(3.0, 4.0), EndpointStyle::Arrow);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "positionEndpoint");
        assert_eq!(v["style"], "arrow");
        assert_eq!(v["position"]["x"], 3.0);
    }

    #[test]
    fn shape_endpoint_without_position_means_smart_routing() {
        let json = r#"{"type":"shapeEndpoint","shapeId":"b1"}"#;
        let e: Endpoint = serde_json::from_str(json).unwrap();
        match e {
            Endpoint::ShapeEndpoint { position, style, .. } => {
                assert!(position.is_none());
                assert_eq!(style, EndpointStyle::None, "style defaults to none");
            }
            _ => panic!("expected shapeEndpoint"),
        }
    }

    #[test]
    fn all_three_line_types_parse() {
        for (text, expected) in [
            ("straight", LineType::Straight),
            ("elbow", LineType::Elbow),
            ("curved", LineType::Curved),
        ] {
            let t: LineType = serde_json::from_str(&format!("\"{text}\"")).unwrap();
            assert_eq!(t, expected);
        }
    }

    #[test]
    fn unknown_endpoint_style_is_preserved() {
        let e: Endpoint =
            serde_json::from_str(r#"{"type":"shapeEndpoint","style":"futureGlyph","shapeId":"b"}"#)
                .unwrap();
        assert_eq!(*e.style(), EndpointStyle::Other("futureGlyph".into()));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["style"], "futureGlyph");
    }

    #[test]
    fn touches_shape_checks_both_ends() {
        let l = Line::new(
            "l",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("b", EndpointStyle::Arrow),
        );
        assert!(l.touches_shape("a"));
        assert!(l.touches_shape("b"));
        assert!(!l.touches_shape("c"));
    }

    #[test]
    fn published_filled_terminators_are_classified() {
        assert!(EndpointStyle::Arrow.is_filled());
        assert!(EndpointStyle::Composition.is_filled());
        assert!(!EndpointStyle::HollowArrow.is_filled());
        assert!(!EndpointStyle::OpenCircle.is_filled());
    }
}
