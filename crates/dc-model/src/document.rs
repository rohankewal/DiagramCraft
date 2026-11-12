//! The document, its pages, and page settings.

use crate::color::Color;
use crate::geom::BoundingBox;
use crate::line::Line;
use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The document unit. The format defaults to inches when unspecified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Cm,
    #[default]
    In,
    Pt,
    Px,
}

impl Units {
    /// How many typographic points one of these units spans. Pixels are taken
    /// at the CSS reference density of 96 per inch.
    pub fn points_per_unit(self) -> f64 {
        match self {
            Units::In => 72.0,
            Units::Cm => 72.0 / 2.54,
            Units::Pt => 1.0,
            Units::Px => 0.75,
        }
    }

    pub fn to_points(self, value: f64) -> f64 {
        value * self.points_per_unit()
    }

    pub fn from_points(self, points: f64) -> f64 {
        points / self.points_per_unit()
    }

    pub fn label(self) -> &'static str {
        match self {
            Units::Cm => "cm",
            Units::In => "in",
            Units::Pt => "pt",
            Units::Px => "px",
        }
    }
}

/// Document-wide settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A link from the document to a CSV data source in the package.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    /// A file in the package's `data` directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Initialization data for a Lucid extension package. This build does not run
/// extensions; the block is carried through so documents are not degraded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionBootstrapData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A named page size, or explicit dimensions.
///
/// The type string is kept open because the format's list of named sizes is not
/// exhaustively published; an unrecognized name falls back to US Letter for
/// display while retaining its original spelling on save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSize {
    #[serde(rename = "type")]
    pub size_type: String,
    /// `portrait` or `landscape`, for named sizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}

impl Default for PageSize {
    fn default() -> Self {
        PageSize { size_type: "letter".into(), format: None, w: None, h: None }
    }
}

impl PageSize {
    pub fn custom(w: f64, h: f64) -> Self {
        PageSize { size_type: "custom".into(), format: None, w: Some(w), h: Some(h) }
    }

    pub fn is_landscape(&self) -> bool {
        self.format.as_deref() == Some("landscape")
    }

    /// Portrait dimensions in points for the named sizes, before orientation.
    fn named_portrait_points(name: &str) -> (f64, f64) {
        match name {
            "letter" => (612.0, 792.0),
            "legal" => (612.0, 1008.0),
            "tabloid" => (792.0, 1224.0),
            "executive" => (522.0, 756.0),
            "folio" => (612.0, 936.0),
            "a3" => (841.89, 1190.55),
            "a4" => (595.28, 841.89),
            "a5" => (419.53, 595.28),
            "b4" => (708.66, 1000.63),
            "b5" => (498.9, 708.66),
            // Unknown named sizes render at Letter; the original name is kept.
            _ => (612.0, 792.0),
        }
    }

    /// Width and height in points, applying `format` orientation. Custom sizes
    /// are interpreted in `units`.
    pub fn dimensions_points(&self, units: Units) -> (f64, f64) {
        if self.size_type == "custom" {
            let w = units.to_points(self.w.unwrap_or(0.0));
            let h = units.to_points(self.h.unwrap_or(0.0));
            return (w, h);
        }
        let (w, h) = Self::named_portrait_points(&self.size_type);
        if self.is_landscape() { (h, w) } else { (w, h) }
    }
}

/// Settings for a single page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infinite_canvas: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_tiling: Option<bool>,
    /// Ignored by the format when `infinite_canvas` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<PageSize>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl PageSettings {
    pub fn is_infinite(&self) -> bool {
        self.infinite_canvas.unwrap_or(false)
    }

    pub fn resolved_fill(&self) -> Color {
        self.fill_color.unwrap_or(Color::WHITE)
    }

    /// The page bounds in points, or `None` on an infinite canvas.
    pub fn bounds_points(&self, units: Units) -> Option<(f64, f64)> {
        if self.is_infinite() {
            return None;
        }
        Some(self.size.clone().unwrap_or_default().dimensions_points(units))
    }
}

/// A collection of items that move and transform together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    /// IDs of the member shapes, lines, and nested groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i64>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Group {
    pub fn new(id: impl Into<String>, items: Vec<String>) -> Self {
        Group { id: id.into(), items: Some(items), note: None, z_index: None, extra: Map::new() }
    }

    pub fn item_ids(&self) -> &[String] {
        self.items.as_deref().unwrap_or(&[])
    }
}

/// A layer grouping items for visibility and locking.
///
/// The published specification does not detail this object's fields, so the
/// model stays permissive: known conventional fields are typed and everything
/// else is preserved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Layer {
    pub fn is_visible(&self) -> bool {
        self.visible.unwrap_or(true)
    }

    pub fn is_locked(&self) -> bool {
        self.locked.unwrap_or(false)
    }

    pub fn items(&self) -> &[String] {
        self.items.as_deref().unwrap_or(&[])
    }
}

/// A single page of a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub title: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<PageSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shapes: Option<Vec<Shape>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<Line>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Group>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<Layer>>,
    /// Shapes bound to a data collection. Not interpreted by this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_backed_shapes: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<Value>,
    /// Guides the author placed on this page, under a namespaced key.
    ///
    /// An extension: the published specification has no guides. The prefix
    /// keeps it from colliding with a field the format may add later, and a
    /// reader that does not know it carries it through untouched.
    #[serde(rename = "dcGuides", default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Vec<Guide>>,

    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A remembered binding between shapes and a table of data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLink {
    /// The entry name within the package's `data` directory.
    pub source: String,
    /// The column whose values are matched against shape labels.
    pub key_column: String,
    /// Where the source came from on disk, when it came from outside the
    /// document. Refreshing re-reads this; without it, refreshing re-reads
    /// the copy the package carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A straight line an author placed to align things against.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    /// Where it sits, in document units: an x for a vertical guide and a y
    /// for a horizontal one.
    pub at: f64,
    /// Whether the line runs down the page rather than across it.
    pub vertical: bool,
}

impl Page {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Page {
            id: id.into(),
            title: title.into(),
            settings: None,
            shapes: None,
            lines: None,
            groups: None,
            layers: None,
            data_backed_shapes: None,
            guides: None,
            custom_data: None,
            extra: Map::new(),
        }
    }

    pub fn shapes(&self) -> &[Shape] {
        self.shapes.as_deref().unwrap_or(&[])
    }

    pub fn lines(&self) -> &[Line] {
        self.lines.as_deref().unwrap_or(&[])
    }

    /// The guides placed on this page.
    pub fn guides(&self) -> &[Guide] {
        self.guides.as_deref().unwrap_or(&[])
    }

    pub fn groups(&self) -> &[Group] {
        self.groups.as_deref().unwrap_or(&[])
    }

    pub fn layers(&self) -> &[Layer] {
        self.layers.as_deref().unwrap_or(&[])
    }

    pub fn shapes_mut(&mut self) -> &mut Vec<Shape> {
        self.shapes.get_or_insert_with(Vec::new)
    }

    pub fn lines_mut(&mut self) -> &mut Vec<Line> {
        self.lines.get_or_insert_with(Vec::new)
    }

    pub fn groups_mut(&mut self) -> &mut Vec<Group> {
        self.groups.get_or_insert_with(Vec::new)
    }

    pub fn resolved_settings(&self) -> PageSettings {
        self.settings.clone().unwrap_or_default()
    }

    pub fn shape(&self, id: &str) -> Option<&Shape> {
        self.shapes().iter().find(|s| s.id == id)
    }

    pub fn shape_mut(&mut self, id: &str) -> Option<&mut Shape> {
        self.shapes_mut().iter_mut().find(|s| s.id == id)
    }

    pub fn line(&self, id: &str) -> Option<&Line> {
        self.lines().iter().find(|l| l.id == id)
    }

    pub fn line_mut(&mut self, id: &str) -> Option<&mut Line> {
        self.lines_mut().iter_mut().find(|l| l.id == id)
    }

    pub fn add_shape(&mut self, shape: Shape) {
        self.shapes_mut().push(shape);
    }

    pub fn add_line(&mut self, line: Line) {
        self.lines_mut().push(line);
    }

    /// Removes a shape and every line attached to it, returning whether the
    /// shape was present.
    pub fn remove_shape(&mut self, id: &str) -> bool {
        let before = self.shapes().len();
        self.shapes_mut().retain(|s| s.id != id);
        let removed = self.shapes().len() != before;
        if removed {
            self.lines_mut().retain(|l| !l.touches_shape(id));
        }
        removed
    }

    pub fn remove_line(&mut self, id: &str) -> bool {
        let before = self.lines().len();
        self.lines_mut().retain(|l| l.id != id);
        self.lines().len() != before
    }

    /// Whether any item on the page already uses this ID.
    pub fn contains_id(&self, id: &str) -> bool {
        self.shapes().iter().any(|s| s.id == id)
            || self.lines().iter().any(|l| l.id == id)
            || self.groups().iter().any(|g| g.id == id)
            || self.layers().iter().any(|l| l.id == id)
    }

    /// The box enclosing every shape on the page, or `None` when empty. Lines
    /// are excluded because their geometry depends on resolved routing.
    pub fn content_bounds(&self) -> Option<BoundingBox> {
        self.shapes()
            .iter()
            .map(|s| s.bounding_box.aabb())
            .reduce(|a, b| a.union(&b))
    }

    /// Shapes ordered back to front, which is paint order.
    pub fn shapes_in_paint_order(&self) -> Vec<&Shape> {
        let mut v: Vec<&Shape> = self.shapes().iter().collect();
        v.sort_by_key(|s| s.resolved_z_index());
        v
    }
}

/// A diagram document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collections: Option<Vec<Collection>>,
    pub pages: Vec<Page>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_settings: Option<DocumentSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_bootstrap_data: Option<ExtensionBootstrapData>,
    /// Data links this document remembers, under a namespaced key.
    ///
    /// An extension, like the formatting rules beside it. Keeping the
    /// binding is what makes a link refreshable rather than a one-off: the
    /// document records what it was matched against and can be told to do it
    /// again when the data has moved on.
    #[serde(
        rename = "dcDataLinks",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub data_links: Option<Vec<DataLink>>,
    /// Conditional-formatting rules, under a namespaced key.
    ///
    /// An extension: the published specification has none. The name is
    /// prefixed so it cannot collide with a field the format may add later.
    #[serde(
        rename = "dcFormatRules",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub format_rules: Option<Vec<crate::rules::FormatRule>>,

    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// The format version this build writes.
pub const FORMAT_VERSION: u32 = 1;

impl Document {
    /// A new document with one empty page.
    pub fn new() -> Self {
        Document {
            version: FORMAT_VERSION,
            collections: None,
            pages: vec![Page::new("page1", "Page 1")],
            document_settings: None,
            extension_bootstrap_data: None,
            data_links: None,
            format_rules: None,
            extra: Map::new(),
        }
    }

    pub fn units(&self) -> Units {
        self.document_settings
            .as_ref()
            .and_then(|s| s.units)
            .unwrap_or_default()
    }

    pub fn set_units(&mut self, units: Units) {
        self.document_settings.get_or_insert_with(Default::default).units = Some(units);
    }

    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: &str) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == id)
    }

    /// Whether any page already uses this ID.
    pub fn contains_id(&self, id: &str) -> bool {
        self.pages.iter().any(|p| p.id == id || p.contains_id(id))
    }

    /// An unused ID with the given prefix, suitable for a new item.
    pub fn fresh_id(&self, prefix: &str) -> String {
        for n in 1..u32::MAX {
            let candidate = format!("{prefix}{n}");
            if !self.contains_id(&candidate) {
                return candidate;
            }
        }
        unreachable!("exhausted the id space")
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Shape, ShapeType};

    #[test]
    fn published_document_example_parses() {
        // Structure taken from the published overview.
        let json = r#"{
            "version": 1,
            "collections": [{"id": "collection-id", "dataSource": "filename.csv"}],
            "pages": [{"id": "page-id", "title": "Page Title",
                       "shapes": [], "lines": [], "groups": [], "layers": []}],
            "documentSettings": {"units": "in"},
            "extensionBootstrapData": {
                "packageId": "110808fd-4553-4316-bccf-4f25ff59a532",
                "extensionName": "extension-name",
                "minimumVersion": "1.0.0",
                "data": {}
            }
        }"#;
        let d: Document = serde_json::from_str(json).unwrap();
        assert_eq!(d.version, 1);
        assert_eq!(d.units(), Units::In);
        assert_eq!(d.pages.len(), 1);
        assert_eq!(d.collections.as_ref().unwrap()[0].data_source.as_deref(), Some("filename.csv"));
    }

    #[test]
    fn units_default_to_inches_when_absent() {
        let d: Document =
            serde_json::from_str(r#"{"version":1,"pages":[{"id":"p","title":"P"}]}"#).unwrap();
        assert_eq!(d.units(), Units::In);
    }

    #[test]
    fn unit_conversion_round_trips() {
        for u in [Units::Cm, Units::In, Units::Pt, Units::Px] {
            let v = 12.5;
            assert!((u.from_points(u.to_points(v)) - v).abs() < 1e-9, "{u:?}");
        }
        assert_eq!(Units::In.to_points(1.0), 72.0);
        assert_eq!(Units::Pt.to_points(10.0), 10.0);
        assert!((Units::Px.to_points(96.0) - 72.0).abs() < 1e-9);
    }

    #[test]
    fn published_page_settings_example_parses() {
        let s: PageSettings = serde_json::from_str(
            r##"{"fillColor":"#333","infiniteCanvas":true,"autoTiling":false,
                "size":{"type":"custom","w":1000,"h":1500}}"##,
        )
        .unwrap();
        assert_eq!(s.resolved_fill(), Color::rgb(0x33, 0x33, 0x33));
        assert!(s.is_infinite());
        assert_eq!(s.bounds_points(Units::In), None, "infinite canvas has no bounds");
    }

    #[test]
    fn named_page_size_applies_orientation() {
        let portrait = PageSize { size_type: "letter".into(), format: None, w: None, h: None };
        assert_eq!(portrait.dimensions_points(Units::In), (612.0, 792.0));

        let landscape =
            PageSize { size_type: "letter".into(), format: Some("landscape".into()), w: None, h: None };
        assert_eq!(landscape.dimensions_points(Units::In), (792.0, 612.0));

        let exec =
            PageSize { size_type: "executive".into(), format: Some("landscape".into()), w: None, h: None };
        assert_eq!(exec.dimensions_points(Units::In), (756.0, 522.0));
    }

    #[test]
    fn custom_page_size_uses_document_units() {
        let size = PageSize::custom(10.0, 20.0);
        assert_eq!(size.dimensions_points(Units::In), (720.0, 1440.0));
        assert_eq!(size.dimensions_points(Units::Pt), (10.0, 20.0));
    }

    #[test]
    fn unknown_page_size_keeps_its_name() {
        let s = PageSize { size_type: "superTabloid".into(), format: None, w: None, h: None };
        assert_eq!(s.dimensions_points(Units::In), (612.0, 792.0), "falls back to letter");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["type"], "superTabloid", "original name is retained");
    }

    #[test]
    fn removing_a_shape_removes_its_connectors() {
        use crate::line::{Endpoint, EndpointStyle, Line};
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0)));
        page.add_shape(Shape::new("b", ShapeType::Rectangle, BoundingBox::new(50.0, 0.0, 10.0, 10.0)));
        page.add_line(Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("b", EndpointStyle::Arrow),
        ));

        assert!(page.remove_shape("a"));
        assert_eq!(page.shapes().len(), 1);
        assert_eq!(page.lines().len(), 0, "the dangling connector is removed too");
        assert!(!page.remove_shape("a"), "removing twice reports absent");
    }

    #[test]
    fn fresh_id_skips_taken_ids() {
        let mut d = Document::new();
        let page = &mut d.pages[0];
        page.add_shape(Shape::new("shape1", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0)));
        page.add_shape(Shape::new("shape2", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0)));
        assert_eq!(d.fresh_id("shape"), "shape3");
    }

    #[test]
    fn content_bounds_covers_all_shapes() {
        let mut page = Page::new("p", "P");
        page.add_shape(Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0)));
        page.add_shape(Shape::new("b", ShapeType::Rectangle, BoundingBox::new(90.0, 40.0, 10.0, 10.0)));
        let b = page.content_bounds().unwrap();
        assert_eq!((b.x, b.y, b.w, b.h), (0.0, 0.0, 100.0, 50.0));
        assert!(Page::new("e", "Empty").content_bounds().is_none());
    }

    #[test]
    fn paint_order_follows_z_index() {
        let mut page = Page::new("p", "P");
        let mut top = Shape::new("top", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        top.z_index = Some(5);
        let mut bottom = Shape::new("bottom", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        bottom.z_index = Some(-2);
        page.add_shape(top);
        page.add_shape(bottom);

        let order: Vec<&str> = page.shapes_in_paint_order().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, vec!["bottom", "top"]);
    }

    #[test]
    fn unmodeled_document_fields_survive() {
        let json = r#"{"version":1,"pages":[{"id":"p","title":"P"}],"futureThing":{"a":1}}"#;
        let d: Document = serde_json::from_str(json).unwrap();
        let back = serde_json::to_value(&d).unwrap();
        assert_eq!(back["futureThing"]["a"], 1);
    }
}
