//! A Visio shape sheet: the cell table a shape is described by, and the
//! conversion from Visio's conventions to the document model's.
//!
//! Every geometric and visual property of a Visio shape is a named cell with a
//! string value. A shape that leaves a cell unset inherits it from the master
//! it was stamped from, so lookup checks the shape first and the master second.

use crate::xml::Node;
use dc_model::{
    BoundingBox, Color, Fill, LineType, Point, Stroke, StrokeStyle, Style,
};
use std::collections::BTreeMap;

/// Points per inch, matching the crate-level constant.
const POINTS_PER_INCH: f64 = 72.0;

/// A shape's cells, already resolved against its master, plus everything the
/// importer derives from them.
#[derive(Clone, Debug)]
pub struct Sheet {
    pub cells: BTreeMap<String, String>,
    /// The shape's box in document units, with Visio's axis already flipped.
    pub bounds: BoundingBox,
    pub text: String,
    pub master_name: String,
}

impl Sheet {
    /// Reads a shape element, filling in anything it inherits from its master.
    pub fn read(
        node: &Node,
        master_cells: &BTreeMap<String, String>,
        master_name: &str,
        page_height_in: f64,
    ) -> Self {
        let mut cells = master_cells.clone();
        // The instance wins wherever it says anything.
        cells.extend(cells_of(node));

        let text = node
            .child("Text")
            .map(|t| t.text.clone())
            .unwrap_or_default();

        let bounds = bounds_from(&cells, page_height_in);

        Sheet {
            cells,
            bounds,
            text,
            master_name: master_name.to_string(),
        }
    }

    pub fn number(&self, name: &str) -> Option<f64> {
        self.cells.get(name)?.trim().parse::<f64>().ok()
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.cells.get(name).map(String::as_str)
    }

    /// The style the shape's fill and line cells describe.
    pub fn style(&self) -> Style {
        let fill = self
            .color("FillForegnd")
            // A shape whose fill pattern is zero is not filled at all.
            .filter(|_| self.number("FillPattern").unwrap_or(1.0) != 0.0)
            .unwrap_or(Color::WHITE);

        let rounding = self
            .number("Rounding")
            // The model stores a radius at double the pixel value.
            .map(|inches| inches * POINTS_PER_INCH * 2.0)
            .filter(|r| *r > 0.0);

        Style {
            fill: Some(Fill::solid(fill)),
            stroke: Some(self.stroke()),
            rounding,
            ..Default::default()
        }
    }

    /// The stroke the shape's line cells describe.
    pub fn stroke(&self) -> Stroke {
        // Line pattern zero means no line, which the model expresses as a
        // zero width rather than a separate flag.
        let invisible = self.number("LinePattern").is_some_and(|p| p == 0.0);
        let width = if invisible {
            0.0
        } else {
            self.number("LineWeight")
                .map(|inches| (inches * POINTS_PER_INCH).max(0.25))
                .unwrap_or(1.0)
        };

        Stroke {
            color: Some(self.color("LineColor").unwrap_or(Color::BLACK)),
            width: Some(width),
            style: Some(self.dash()),
        }
    }

    /// The dash pattern, read from Visio's numbered line patterns: 1 is solid,
    /// 2 and 3 are dotted, and the rest are dashes of various rhythms.
    pub fn dash(&self) -> StrokeStyle {
        match self.number("LinePattern").unwrap_or(1.0) as i64 {
            0 | 1 => StrokeStyle::Solid,
            2 | 3 => StrokeStyle::Dotted,
            _ => StrokeStyle::Dashed,
        }
    }

    /// How the connector routes, from its line-routing cell.
    pub fn line_type(&self) -> LineType {
        // `ShapeRouteStyle` 1 and 2 are straight centre-to-centre lines and 16
        // is a curve; every other value, and the absence of the cell, is one
        // of Visio's right-angle routings.
        match self.number("ShapeRouteStyle").unwrap_or(0.0) as i64 {
            1 | 2 => LineType::Straight,
            16 => LineType::Curved,
            _ => LineType::Elbow,
        }
    }

    /// Where the connector's tail sits, in document units.
    pub fn begin_point(&self, page_height_in: f64) -> Point {
        self.point("BeginX", "BeginY", page_height_in)
            .unwrap_or_else(|| self.bounds.center())
    }

    /// Where the connector's head sits, in document units.
    pub fn end_point(&self, page_height_in: f64) -> Point {
        self.point("EndX", "EndY", page_height_in)
            .unwrap_or_else(|| self.bounds.center())
    }

    fn point(&self, x_cell: &str, y_cell: &str, page_height_in: f64) -> Option<Point> {
        let x = self.number(x_cell)?;
        let y = self.number(y_cell)?;
        Some(Point::new(
            x * POINTS_PER_INCH,
            flip_y(y, page_height_in),
        ))
    }

    fn color(&self, name: &str) -> Option<Color> {
        parse_color(self.value(name)?)
    }
}

/// Every `Cell` directly inside an element, as a name to value map.
pub fn cells_of(node: &Node) -> BTreeMap<String, String> {
    let mut cells = BTreeMap::new();
    for cell in node.children_named("Cell") {
        let (Some(name), Some(value)) = (cell.attr("N"), cell.attr("V")) else {
            continue;
        };
        cells.insert(name.to_string(), value.to_string());
    }
    // A `Section` holds the cells of sub-shapes and rows; those are not shape
    // level properties, so they are deliberately not merged in here.
    cells
}

/// Converts a Visio y coordinate, measured up from the bottom of the page, to
/// document units measured down from the top.
pub fn flip_y(y_inches: f64, page_height_in: f64) -> f64 {
    (page_height_in - y_inches) * POINTS_PER_INCH
}

/// The shape's box, from the cells that place and size it.
///
/// Visio positions a shape by its pin rather than by a corner: `PinX`/`PinY`
/// is where the pin sits on the page, and `LocPinX`/`LocPinY` is where the pin
/// sits inside the shape. The top-left corner is therefore the pin less the
/// local pin, and the box is then flipped onto a downward y axis.
pub fn bounds_from(cells: &BTreeMap<String, String>, page_height_in: f64) -> BoundingBox {
    let get = |name: &str| cells.get(name).and_then(|v| v.trim().parse::<f64>().ok());

    let width = get("Width").unwrap_or(1.0).abs();
    let height = get("Height").unwrap_or(1.0).abs();
    let pin_x = get("PinX").unwrap_or(0.0);
    let pin_y = get("PinY").unwrap_or(0.0);
    // A shape with no local pin rotates about its centre, which is also what
    // Visio's own default is.
    let loc_x = get("LocPinX").unwrap_or(width / 2.0);
    let loc_y = get("LocPinY").unwrap_or(height / 2.0);

    let left = pin_x - loc_x;
    // The bottom edge in Visio's frame is the top edge in ours once flipped.
    let top_in_visio = pin_y - loc_y + height;

    let bounds = BoundingBox::new(
        left * POINTS_PER_INCH,
        flip_y(top_in_visio, page_height_in),
        width * POINTS_PER_INCH,
        height * POINTS_PER_INCH,
    );

    // Visio measures rotation counter-clockwise in radians; the document
    // model measures it clockwise in degrees.
    match get("Angle") {
        Some(radians) if radians != 0.0 => {
            bounds.with_rotation((-radians.to_degrees()).rem_euclid(360.0))
        }
        _ => bounds,
    }
}

/// Parses a Visio color, which is a hex triplet or an index into the drawing's
/// palette. An index has no meaning without the palette, so it is declined
/// rather than guessed at.
pub fn parse_color(value: &str) -> Option<Color> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return match hex.len() {
            3 | 4 | 6 | 8 => Color::parse(v).ok(),
            _ => None,
        };
    }
    // Some writers emit `RGB(r, g, b)`.
    if let Some(inner) = v
        .strip_prefix("RGB(")
        .or_else(|| v.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<u8> = inner
            .split(',')
            .filter_map(|p| p.trim().parse::<f64>().ok())
            .map(|n| n.clamp(0.0, 255.0) as u8)
            .collect();
        if parts.len() == 3 {
            return Some(Color::rgb(parts[0], parts[1], parts[2]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_y_axis_is_flipped_about_the_page() {
        // The top of an 11 inch page is y = 11 in Visio and 0 here.
        assert_eq!(flip_y(11.0, 11.0), 0.0);
        assert_eq!(flip_y(0.0, 11.0), 792.0, "the bottom edge in points");
        assert_eq!(flip_y(5.5, 11.0), 396.0, "the middle stays in the middle");
    }

    #[test]
    fn a_pin_centred_shape_lands_on_its_corner() {
        // A 2 x 0.75 inch shape pinned at its centre, 4.25 across and 9.5 up
        // an 11 inch page.
        let bb = bounds_from(
            &cells(&[
                ("PinX", "4.25"),
                ("PinY", "9.5"),
                ("Width", "2"),
                ("Height", "0.75"),
                ("LocPinX", "1"),
                ("LocPinY", "0.375"),
            ]),
            11.0,
        );
        assert_eq!(bb.x, 3.25 * 72.0);
        assert_eq!(bb.w, 144.0);
        assert_eq!(bb.h, 54.0);
        // Its top edge is 9.875 inches up, so 1.125 inches down from the top.
        assert!((bb.y - 1.125 * 72.0).abs() < 1e-9, "y was {}", bb.y);
    }

    #[test]
    fn an_off_centre_pin_is_respected() {
        // Pinned at its own top-left corner.
        let bb = bounds_from(
            &cells(&[
                ("PinX", "1"),
                ("PinY", "10"),
                ("Width", "2"),
                ("Height", "1"),
                ("LocPinX", "0"),
                ("LocPinY", "1"),
            ]),
            11.0,
        );
        assert_eq!(bb.x, 72.0);
        assert_eq!(bb.y, 72.0, "one inch down from the top of the page");
    }

    #[test]
    fn a_missing_local_pin_defaults_to_the_centre() {
        let bb = bounds_from(
            &cells(&[("PinX", "2"), ("PinY", "9"), ("Width", "2"), ("Height", "2")]),
            11.0,
        );
        assert_eq!(bb.x, 72.0, "centred horizontally on 2 inches");
        assert_eq!(bb.y, 72.0, "its top edge is 10 inches up");
    }

    #[test]
    fn rotation_is_converted_to_clockwise_degrees() {
        // A quarter turn counter-clockwise in Visio is 270 degrees clockwise.
        let bb = bounds_from(
            &cells(&[
                ("PinX", "1"),
                ("PinY", "1"),
                ("Width", "1"),
                ("Height", "1"),
                ("Angle", &std::f64::consts::FRAC_PI_2.to_string()),
            ]),
            11.0,
        );
        assert!((bb.rotation_degrees() - 270.0).abs() < 1e-6, "{}", bb.rotation_degrees());
    }

    #[test]
    fn a_shape_with_no_cells_still_gets_a_box() {
        let bb = bounds_from(&cells(&[]), 11.0);
        assert!(bb.w > 0.0 && bb.h > 0.0);
    }

    #[test]
    fn negative_sizes_are_normalized() {
        let bb = bounds_from(&cells(&[("Width", "-2"), ("Height", "-1")]), 11.0);
        assert_eq!((bb.w, bb.h), (144.0, 72.0));
    }

    #[test]
    fn a_shape_inherits_cells_it_does_not_set() {
        let node = crate::xml::parse(r#"<Shape ID="1"><Cell N="PinX" V="3"/></Shape>"#)
            .unwrap()
            .children
            .remove(0);
        let master = cells(&[("PinX", "1"), ("PinY", "9"), ("Width", "2"), ("Height", "1")]);

        let sheet = Sheet::read(&node, &master, "Process", 11.0);
        assert_eq!(sheet.number("PinX"), Some(3.0), "the instance wins");
        assert_eq!(sheet.number("Width"), Some(2.0), "and inherits the rest");
    }

    #[test]
    fn colors_parse_in_the_forms_visio_writes() {
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("RGB(0, 128, 255)"), Some(Color::rgb(0, 128, 255)));
        assert_eq!(parse_color("3"), None, "a palette index means nothing alone");
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn line_weight_converts_from_inches_to_points() {
        let node = crate::xml::parse(
            r##"<Shape><Cell N="LineWeight" V="0.0138888888888889"/><Cell N="LineColor" V="#123456"/></Shape>"##,
        )
        .unwrap()
        .children
        .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "Process", 11.0);
        let stroke = sheet.stroke();
        assert!((stroke.resolved_width() - 1.0).abs() < 0.01, "{}", stroke.resolved_width());
        assert_eq!(stroke.resolved_color(), Color::rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn an_unpatterned_line_is_invisible() {
        let node = crate::xml::parse(r#"<Shape><Cell N="LinePattern" V="0"/></Shape>"#)
            .unwrap()
            .children
            .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "Process", 11.0);
        assert!(sheet.stroke().is_invisible());
    }

    #[test]
    fn line_patterns_become_dash_styles() {
        let dash_for = |pattern: &str| {
            let xml = format!(r#"<Shape><Cell N="LinePattern" V="{pattern}"/></Shape>"#);
            let node = crate::xml::parse(&xml).unwrap().children.remove(0);
            Sheet::read(&node, &BTreeMap::new(), "", 11.0).dash()
        };
        assert_eq!(dash_for("1"), StrokeStyle::Solid);
        assert_eq!(dash_for("2"), StrokeStyle::Dotted);
        assert_eq!(dash_for("4"), StrokeStyle::Dashed);
    }

    #[test]
    fn an_unfilled_shape_keeps_a_white_fill_rather_than_a_missing_one() {
        let node = crate::xml::parse(
            r##"<Shape><Cell N="FillPattern" V="0"/><Cell N="FillForegnd" V="#ff0000"/></Shape>"##,
        )
        .unwrap()
        .children
        .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "", 11.0);
        assert_eq!(
            sheet.style().resolved_fill().color(),
            Some(Color::WHITE),
            "pattern zero means the red was never painted"
        );
    }

    #[test]
    fn routing_styles_become_line_types() {
        let route = |style: &str| {
            let xml = format!(r#"<Shape><Cell N="ShapeRouteStyle" V="{style}"/></Shape>"#);
            let node = crate::xml::parse(&xml).unwrap().children.remove(0);
            Sheet::read(&node, &BTreeMap::new(), "Dynamic connector", 11.0).line_type()
        };
        assert_eq!(route("1"), LineType::Straight);
        assert_eq!(route("16"), LineType::Curved);
        assert_eq!(route("5"), LineType::Elbow);
    }

    #[test]
    fn connector_endpoints_are_flipped_like_everything_else() {
        let node = crate::xml::parse(
            r#"<Shape><Cell N="BeginX" V="1"/><Cell N="BeginY" V="10"/>
               <Cell N="EndX" V="4"/><Cell N="EndY" V="6"/></Shape>"#,
        )
        .unwrap()
        .children
        .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "Dynamic connector", 11.0);
        assert_eq!(sheet.begin_point(11.0), Point::new(72.0, 72.0));
        assert_eq!(sheet.end_point(11.0), Point::new(288.0, 360.0));
    }

    #[test]
    fn a_connector_with_no_endpoints_falls_back_to_its_box() {
        let node = crate::xml::parse(r#"<Shape ID="1"/>"#).unwrap().children.remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "Dynamic connector", 11.0);
        assert_eq!(sheet.begin_point(11.0), sheet.bounds.center());
    }

    #[test]
    fn rounding_is_stored_at_the_models_doubled_scale() {
        let node = crate::xml::parse(r#"<Shape><Cell N="Rounding" V="0.125"/></Shape>"#)
            .unwrap()
            .children
            .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "Rectangle", 11.0);
        // An eighth of an inch is 9 points, and the model stores double.
        assert_eq!(sheet.style().corner_radius_px(), 9.0);
    }

    #[test]
    fn section_cells_are_not_mistaken_for_shape_cells() {
        let node = crate::xml::parse(
            r#"<Shape ID="1"><Cell N="Width" V="2"/>
               <Section N="Geometry"><Row><Cell N="X" V="99"/></Row></Section></Shape>"#,
        )
        .unwrap()
        .children
        .remove(0);
        let sheet = Sheet::read(&node, &BTreeMap::new(), "", 11.0);
        assert_eq!(sheet.number("Width"), Some(2.0));
        assert_eq!(sheet.number("X"), None, "geometry rows are not shape cells");
    }
}
