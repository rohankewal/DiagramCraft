//! Fills, strokes, and text styling.
//!
//! Defaults follow the published format: solid white fill, solid 1px black
//! stroke, 10pt Liberation Sans centered text, and a text color derived from
//! fill contrast when unspecified.

use crate::color::Color;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How an image fill is fitted into its shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageScale {
    #[default]
    Fit,
    Fill,
    Stretch,
    Original,
    Tile,
}

/// A shape background: either a flat color or an image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Fill {
    Color {
        color: Color,
    },
    // `rename_all` on the enum renames variants; fields need their own.
    #[serde(rename_all = "camelCase")]
    Image {
        /// A file in the package's `images` directory.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#ref: Option<String>,
        /// An external URL, used when `ref` is absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_scale: Option<ImageScale>,
    },
}

impl Fill {
    pub fn solid(color: Color) -> Self {
        Fill::Color { color }
    }

    /// The flat color of this fill, if it has one.
    pub fn color(&self) -> Option<Color> {
        match self {
            Fill::Color { color } => Some(*color),
            Fill::Image { .. } => None,
        }
    }
}

impl Default for Fill {
    fn default() -> Self {
        Fill::solid(Color::WHITE)
    }
}

/// Dash pattern for a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl StrokeStyle {
    /// Dash lengths in multiples of stroke width, or `None` when solid.
    pub fn dash_pattern(self, width: f64) -> Option<Vec<f64>> {
        let w = width.max(1.0);
        match self {
            StrokeStyle::Solid => None,
            StrokeStyle::Dashed => Some(vec![w * 4.0, w * 3.0]),
            StrokeStyle::Dotted => Some(vec![w, w * 2.0]),
        }
    }
}

/// A shape border or line stroke. Every field is optional in the format, so
/// this type carries the documented defaults in its accessors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StrokeStyle>,
}

impl Stroke {
    pub fn new(color: Color, width: f64) -> Self {
        Stroke { color: Some(color), width: Some(width), style: None }
    }

    /// Defaults to black per the format.
    pub fn resolved_color(&self) -> Color {
        self.color.unwrap_or(Color::BLACK)
    }

    /// Defaults to 1px per the format.
    pub fn resolved_width(&self) -> f64 {
        self.width.unwrap_or(1.0)
    }

    pub fn resolved_style(&self) -> StrokeStyle {
        self.style.unwrap_or_default()
    }

    pub fn is_invisible(&self) -> bool {
        self.resolved_width() <= 0.0 || self.resolved_color().a == 0
    }
}

/// The visual style of a shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Corner radius. The format specifies this as double the pixel value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<Color>,

    /// Style properties not modeled above, preserved for round-tripping.
    ///
    /// The Lucidspark freehand drawing carries its own `style` object of
    /// `{ width, color }`, which lands here rather than being discarded.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Style {
    pub fn resolved_fill(&self) -> Fill {
        self.fill.clone().unwrap_or_default()
    }

    pub fn resolved_stroke(&self) -> Stroke {
        self.stroke.unwrap_or_default()
    }

    /// Corner radius in pixels. The stored value is doubled, per the spec note.
    pub fn corner_radius_px(&self) -> f64 {
        self.rounding.unwrap_or(0.0) / 2.0
    }

    /// Explicit text color, otherwise the contrast-derived default against the
    /// fill, otherwise black.
    pub fn resolved_text_color(&self) -> Color {
        if let Some(c) = self.text_color {
            return c;
        }
        match self.resolved_fill().color() {
            Some(bg) if bg.a > 0 => bg.contrasting_text(),
            _ => Color::BLACK,
        }
    }
}

/// Shape text. The format accepts a bare string or an object, and allows a
/// small HTML subset inside the string; both input shapes are preserved so a
/// round-trip does not rewrite the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Text {
    Plain(String),
    Object { text: String },
}

impl Text {
    pub fn new(s: impl Into<String>) -> Self {
        Text::Object { text: s.into() }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Text::Plain(s) => s,
            Text::Object { text } => text,
        }
    }

    pub fn set(&mut self, s: impl Into<String>) {
        match self {
            Text::Plain(t) => *t = s.into(),
            Text::Object { text } => *text = s.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

/// Horizontal text alignment. Defaults to centered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
    Justify,
}

/// Vertical text alignment. Defaults to centered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerticalAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

/// Resolved text attributes after applying the format's documented defaults.
/// These mirror the CSS-ish attribute names the format permits inline.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub color: Color,
    pub font_family: String,
    pub font_size_pt: f64,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub align: TextAlign,
    pub vertical_align: VerticalAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            color: Color::BLACK,
            font_family: "Liberation Sans".to_string(),
            font_size_pt: 10.0,
            italic: false,
            bold: false,
            underline: false,
            strikethrough: false,
            align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_color_fill_parses() {
        let f: Fill = serde_json::from_str(r##"{"type":"color","color":"#800080"}"##).unwrap();
        assert_eq!(f.color(), Some(Color::rgb(0x80, 0, 0x80)));
    }

    #[test]
    fn documented_image_fills_parse_both_forms() {
        let by_ref: Fill =
            serde_json::from_str(r#"{"type":"image","ref":"image.png","imageScale":"fit"}"#)
                .unwrap();
        match by_ref {
            Fill::Image { r#ref, image_scale, .. } => {
                assert_eq!(r#ref.as_deref(), Some("image.png"));
                assert_eq!(image_scale, Some(ImageScale::Fit));
            }
            _ => panic!("expected image fill"),
        }

        let by_url: Fill = serde_json::from_str(
            r#"{"type":"image","url":"https://www.example.com/images/logo.png","imageScale":"stretch"}"#,
        )
        .unwrap();
        match by_url {
            Fill::Image { url, image_scale, .. } => {
                assert!(url.unwrap().ends_with("logo.png"));
                assert_eq!(image_scale, Some(ImageScale::Stretch));
            }
            _ => panic!("expected image fill"),
        }
    }

    #[test]
    fn documented_style_example_parses() {
        let s: Style = serde_json::from_str(
            r##"{"fill":{"type":"color","color":"#ffffff"},
                "stroke":{"color":"#6f2131","width":3,"style":"solid"},
                "rounding":10,"textColor":"#00ff00"}"##,
        )
        .unwrap();
        assert_eq!(s.resolved_stroke().resolved_width(), 3.0);
        assert_eq!(s.corner_radius_px(), 5.0, "rounding is double the pixel value");
        assert_eq!(s.resolved_text_color(), Color::rgb(0, 255, 0));
    }

    #[test]
    fn stroke_defaults_match_spec() {
        let s = Stroke::default();
        assert_eq!(s.resolved_color(), Color::BLACK);
        assert_eq!(s.resolved_width(), 1.0);
        assert_eq!(s.resolved_style(), StrokeStyle::Solid);
    }

    #[test]
    fn text_color_falls_back_to_fill_contrast() {
        let dark = Style { fill: Some(Fill::solid(Color::rgb(0x11, 0x11, 0x11))), ..Default::default() };
        assert_eq!(dark.resolved_text_color(), Color::WHITE);

        let light = Style { fill: Some(Fill::solid(Color::WHITE)), ..Default::default() };
        assert_eq!(light.resolved_text_color(), Color::BLACK);
    }

    #[test]
    fn text_accepts_both_documented_forms() {
        let obj: Text = serde_json::from_str(r#"{"text":"Shape title"}"#).unwrap();
        assert_eq!(obj.as_str(), "Shape title");

        let plain: Text = serde_json::from_str(r#""Shape title""#).unwrap();
        assert_eq!(plain.as_str(), "Shape title");

        // Each form re-serializes as it arrived.
        assert_eq!(serde_json::to_string(&plain).unwrap(), r#""Shape title""#);
        assert_eq!(serde_json::to_string(&obj).unwrap(), r#"{"text":"Shape title"}"#);
    }

    #[test]
    fn dash_patterns_scale_with_width() {
        assert!(StrokeStyle::Solid.dash_pattern(1.0).is_none());
        assert_eq!(StrokeStyle::Dashed.dash_pattern(2.0), Some(vec![8.0, 6.0]));
        assert_eq!(StrokeStyle::Dotted.dash_pattern(1.0), Some(vec![1.0, 2.0]));
    }

    #[test]
    fn text_style_defaults_match_spec_table() {
        let d = TextStyle::default();
        assert_eq!(d.font_family, "Liberation Sans");
        assert_eq!(d.font_size_pt, 10.0);
        assert_eq!(d.align, TextAlign::Center);
        assert_eq!(d.vertical_align, VerticalAlign::Center);
        assert_eq!(d.color, Color::BLACK);
    }
}
