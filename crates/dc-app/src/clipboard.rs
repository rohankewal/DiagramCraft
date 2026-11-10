//! System clipboard access.
//!
//! Shapes travel as a JSON fragment on the text clipboard rather than through
//! a private in-process buffer. That costs a little encoding work and buys
//! three things: copy and paste work between two running copies of the editor,
//! the payload survives a crash, and anyone curious can inspect what was
//! copied.
//!
//! Every operation degrades to a no-op if the platform clipboard is
//! unavailable — a headless session, a locked X11 selection owner — rather
//! than failing the user's edit.

use dc_model::{Line, Shape};
use serde::{Deserialize, Serialize};

/// Marks a clipboard payload as ours, so pasting arbitrary text from another
/// application is not mistaken for a diagram fragment.
const MAGIC: &str = "application/vnd.diagramcraft.fragment+json";

/// A copied selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fragment {
    /// Identifies the payload as a diagram fragment.
    pub kind: String,
    pub shapes: Vec<Shape>,
    /// Connectors whose endpoints are both inside `shapes`.
    pub lines: Vec<Line>,
}

impl Fragment {
    pub fn new(shapes: Vec<Shape>, lines: Vec<Line>) -> Self {
        Fragment { kind: MAGIC.to_string(), shapes, lines }
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.lines.is_empty()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a clipboard payload, rejecting anything that is not ours.
    pub fn from_json(text: &str) -> Option<Self> {
        let trimmed = text.trim_start();
        if !trimmed.starts_with('{') {
            return None;
        }
        let fragment: Fragment = serde_json::from_str(trimmed).ok()?;
        (fragment.kind == MAGIC).then_some(fragment)
    }
}

/// A handle to the platform clipboard.
///
/// Held open for the process lifetime because on X11 the clipboard contents
/// belong to the owning connection: dropping the handle after a copy would
/// discard what was just copied.
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard {
    pub fn new() -> Self {
        let inner = match arboard::Clipboard::new() {
            Ok(c) => Some(c),
            Err(e) => {
                log::warn!("clipboard unavailable: {e}");
                None
            }
        };
        Clipboard { inner }
    }

    /// Whether the platform clipboard could be opened.
    #[cfg(test)]
    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }

    pub fn set_text(&mut self, text: &str) {
        let Some(clipboard) = self.inner.as_mut() else {
            return;
        };
        if let Err(e) = clipboard.set_text(text.to_string()) {
            log::warn!("could not write to the clipboard: {e}");
        }
    }

    pub fn text(&mut self) -> Option<String> {
        let clipboard = self.inner.as_mut()?;
        match clipboard.get_text() {
            Ok(t) => Some(t),
            Err(e) => {
                log::debug!("clipboard read failed: {e}");
                None
            }
        }
    }

    pub fn set_fragment(&mut self, fragment: &Fragment) {
        self.set_text(&fragment.to_json());
    }

    /// The clipboard contents as a diagram fragment, if they are one.
    pub fn fragment(&mut self) -> Option<Fragment> {
        Fragment::from_json(&self.text()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Endpoint, EndpointStyle, ShapeType};

    fn shape(id: &str) -> Shape {
        Shape::new(id, ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0))
            .with_text("hello")
    }

    #[test]
    fn a_fragment_round_trips_through_json() {
        let fragment = Fragment::new(
            vec![shape("a"), shape("b")],
            vec![Line::new(
                "l1",
                Endpoint::at_shape("a", EndpointStyle::None),
                Endpoint::at_shape("b", EndpointStyle::Arrow),
            )],
        );

        let json = fragment.to_json();
        let back = Fragment::from_json(&json).expect("should parse");
        assert_eq!(back, fragment);
        assert_eq!(back.shapes.len(), 2);
        assert_eq!(back.lines.len(), 1);
    }

    #[test]
    fn ordinary_text_is_not_mistaken_for_a_fragment() {
        assert!(Fragment::from_json("just some copied prose").is_none());
        assert!(Fragment::from_json("").is_none());
        assert!(Fragment::from_json("{}").is_none(), "an object without our marker");
        assert!(
            Fragment::from_json(r#"{"kind":"something-else","shapes":[],"lines":[]}"#).is_none()
        );
    }

    #[test]
    fn malformed_json_is_rejected_without_panicking() {
        assert!(Fragment::from_json("{ not json").is_none());
        assert!(Fragment::from_json("{\"kind\":").is_none());
    }

    #[test]
    fn leading_whitespace_is_tolerated() {
        let json = format!("\n  {}", Fragment::new(vec![shape("a")], vec![]).to_json());
        assert!(Fragment::from_json(&json).is_some());
    }

    #[test]
    fn an_empty_fragment_is_reported_as_empty() {
        assert!(Fragment::default().is_empty());
        assert!(!Fragment::new(vec![shape("a")], vec![]).is_empty());
    }

    #[test]
    fn shape_styling_survives_the_clipboard() {
        let mut s = shape("a");
        s.style = Some(dc_model::Style {
            fill: Some(dc_model::Fill::solid(dc_model::Color::rgb(1, 2, 3))),
            ..Default::default()
        });
        let back = Fragment::from_json(&Fragment::new(vec![s.clone()], vec![]).to_json()).unwrap();
        assert_eq!(back.shapes[0], s);
    }

    #[test]
    fn unknown_shape_fields_survive_the_clipboard() {
        let json = r#"{"id":"s1","type":"awsLambda",
            "boundingBox":{"x":0,"y":0,"w":1,"h":1},"vendorThing":{"a":1}}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        let back = Fragment::from_json(&Fragment::new(vec![s], vec![]).to_json()).unwrap();
        assert_eq!(back.shapes[0].extra["vendorThing"]["a"], 1);
    }

    /// Touching the real clipboard is environment dependent, so this only
    /// checks that the wrapper never panics when it is missing.
    #[test]
    fn a_missing_clipboard_degrades_quietly() {
        let mut clipboard = Clipboard { inner: None };
        assert!(!clipboard.is_available());
        clipboard.set_text("ignored");
        assert_eq!(clipboard.text(), None);
        assert_eq!(clipboard.fragment(), None);
    }
}
