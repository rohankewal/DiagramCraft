//! Writing a Visio `.vsdx` drawing.
//!
//! A `.vsdx` is an OPC package — a ZIP holding XML parts tied together by
//! relationship files — so the writer's job is three things: lay out the
//! parts Visio insists on, convert coordinates, and turn each shape's
//! outline into geometry rows.
//!
//! ## Coordinates
//!
//! Documents here are in points, measured from the top-left with y pointing
//! down. Visio works in inches, from the bottom-left with y pointing up.
//! Every coordinate therefore divides by 72 and flips, and each shape's
//! geometry is written relative to its own lower-left corner rather than to
//! the page.
//!
//! ## Curves
//!
//! Visio expresses curves as arcs or NURBS, neither of which a cubic Bézier
//! maps onto cleanly. Rather than approximate badly, curves are flattened to
//! short line segments at a tolerance fine enough that the result is
//! indistinguishable at any sensible zoom — and a straight edge stays a
//! single straight segment, so a rectangle exports as four lines, not forty.

use dc_model::{Color, Document, Line, Page, Shape, StrokeStyle};
use kurbo::{BezPath, PathEl, Point as KPoint};
use std::io::{Seek, Write};

/// Points per inch.
const POINTS_PER_INCH: f64 = 72.0;

/// How far a flattened curve may stray from the true one, in points. A
/// quarter point is finer than a printer resolves.
const FLATTEN_TOLERANCE: f64 = 0.25;

/// The sheet used when a page declares no size of its own.
const DEFAULT_PAGE: (f64, f64) = (612.0, 792.0);

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("document has no pages")]
    NoPages,
}

/// Writes a document as a Visio drawing.
///
/// Every page becomes a Visio page, in order.
pub fn write<W: Write + Seek>(document: &Document, writer: W) -> Result<(), WriteError> {
    if document.pages.is_empty() {
        return Err(WriteError::NoPages);
    }

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(writer);

    let put = |zip: &mut zip::ZipWriter<W>, name: &str, body: &str| -> Result<(), WriteError> {
        zip.start_file(name, options)?;
        zip.write_all(body.as_bytes())?;
        Ok(())
    };

    put(&mut zip, "[Content_Types].xml", &content_types(document.pages.len()))?;
    put(&mut zip, "_rels/.rels", &root_rels())?;
    put(&mut zip, "visio/document.xml", &document_part())?;
    put(&mut zip, "visio/_rels/document.xml.rels", &document_rels())?;
    put(&mut zip, "visio/pages/pages.xml", &pages_part(document))?;
    put(&mut zip, "visio/pages/_rels/pages.xml.rels", &pages_rels(document.pages.len()))?;

    for (i, page) in document.pages.iter().enumerate() {
        put(&mut zip, &format!("visio/pages/page{}.xml", i + 1), &page_part(page))?;
    }

    zip.finish()?;
    Ok(())
}

/// Writes a document to a path.
pub fn write_path(document: &Document, path: impl AsRef<std::path::Path>) -> Result<(), WriteError> {
    let file = std::fs::File::create(path)?;
    write(document, file)
}

// --------------------------------------------------------------- the parts

const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const MAIN_NS: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn content_types(pages: usize) -> String {
    let mut s = String::from(HEADER);
    s.push_str(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    );
    s.push_str(
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    );
    s.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    s.push_str(
        r#"<Override PartName="/visio/document.xml" ContentType="application/vnd.ms-visio.drawing.main+xml"/>"#,
    );
    s.push_str(
        r#"<Override PartName="/visio/pages/pages.xml" ContentType="application/vnd.ms-visio.pages+xml"/>"#,
    );
    for i in 1..=pages {
        s.push_str(&format!(
            r#"<Override PartName="/visio/pages/page{i}.xml" ContentType="application/vnd.ms-visio.page+xml"/>"#
        ));
    }
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    format!(
        r#"{HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/visio/2010/relationships/document" Target="visio/document.xml"/></Relationships>"#
    )
}

fn document_part() -> String {
    format!(
        r#"{HEADER}<VisioDocument xmlns="{MAIN_NS}" xml:space="preserve"><DocumentSettings TopPage="0" DefaultTextStyle="0" DefaultLineStyle="0" DefaultFillStyle="0" DefaultGuideStyle="0"/></VisioDocument>"#
    )
}

fn document_rels() -> String {
    format!(
        r#"{HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/visio/2010/relationships/pages" Target="pages/pages.xml"/></Relationships>"#
    )
}

fn pages_rels(pages: usize) -> String {
    let mut s = String::from(HEADER);
    s.push_str(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for i in 1..=pages {
        s.push_str(&format!(
            r#"<Relationship Id="rId{i}" Type="http://schemas.microsoft.com/visio/2010/relationships/page" Target="page{i}.xml"/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// The size a page is written at, in points.
///
/// A page's declared size when it has one, grown if the drawing runs past
/// it — Visio would otherwise clip what sits outside the sheet.
fn page_size(page: &Page) -> (f64, f64) {
    let declared = page
        .resolved_settings()
        .bounds_points(dc_model::Units::default())
        .unwrap_or(DEFAULT_PAGE);
    match page.content_bounds() {
        Some(b) => (
            declared.0.max(b.right() + 24.0),
            declared.1.max(b.bottom() + 24.0),
        ),
        None => declared,
    }
}

fn pages_part(document: &Document) -> String {
    let mut s = String::from(HEADER);
    s.push_str(&format!(
        r#"<Pages xmlns="{MAIN_NS}" xmlns:r="{REL_NS}" xml:space="preserve">"#
    ));
    for (i, page) in document.pages.iter().enumerate() {
        let (w, h) = page_size(page);
        let name = escape(&page.title);
        s.push_str(&format!(
            r#"<Page ID="{i}" NameU="{name}" Name="{name}" ViewScale="1" ViewCenterX="{}" ViewCenterY="{}">"#,
            num(inches(w) / 2.0),
            num(inches(h) / 2.0),
        ));
        s.push_str(r#"<PageSheet LineStyle="0" FillStyle="0" TextStyle="0">"#);
        s.push_str(&format!(r#"<Cell N="PageWidth" V="{}"/>"#, num(inches(w))));
        s.push_str(&format!(r#"<Cell N="PageHeight" V="{}"/>"#, num(inches(h))));
        s.push_str("</PageSheet>");
        s.push_str(&format!(r#"<Rel r:id="rId{}"/>"#, i + 1));
        s.push_str("</Page>");
    }
    s.push_str("</Pages>");
    s
}

fn page_part(page: &Page) -> String {
    let (_, page_h) = page_size(page);
    let mut s = String::from(HEADER);
    s.push_str(&format!(
        r#"<PageContents xmlns="{MAIN_NS}" xmlns:r="{REL_NS}" xml:space="preserve"><Shapes>"#
    ));

    let mut id = 1u32;

    // Connectors first, so they sit under the shapes they join — the order
    // the canvas paints in.
    for line in page.lines() {
        if let Some(body) = line_shape(line, page, page_h, id) {
            s.push_str(&body);
            id += 1;
        }
    }

    for shape in page.shapes_in_paint_order() {
        if let Some(body) = shape_shape(shape, page_h, id) {
            s.push_str(&body);
            id += 1;
        }
    }

    s.push_str("</Shapes></PageContents>");
    s
}

// -------------------------------------------------------------- the shapes

fn shape_shape(shape: &Shape, page_h: f64, id: u32) -> Option<String> {
    let alpha = shape.opacity_factor();
    if alpha <= 0.0 || shape.shape_type == dc_model::ShapeType::Hotspot {
        return None;
    }

    let geo = dc_geom::geometry(shape);
    let bb = shape.bounding_box.aabb();
    if bb.w <= 0.0 || bb.h <= 0.0 {
        return None;
    }

    let style = shape.resolved_style();
    let stroke = shape.effective_stroke();
    let fill = if geo.filled {
        match style.resolved_fill() {
            dc_model::Fill::Color { color } => Some(color),
            dc_model::Fill::Image { .. } => None,
        }
    } else {
        None
    };

    let mut s = String::new();
    s.push_str(&format!(
        r#"<Shape ID="{id}" NameU="Shape{id}" Type="Shape" LineStyle="0" FillStyle="0" TextStyle="0">"#
    ));
    s.push_str(&placement(bb, page_h));

    // Fill and line are shape-level; the geometry sections below say only
    // whether each one participates.
    match fill {
        Some(color) => {
            s.push_str(&format!(r#"<Cell N="FillForegnd" V="{}"/>"#, hex(color)));
            s.push_str(&format!(
                r#"<Cell N="FillForegndTrans" V="{}"/>"#,
                num(1.0 - alpha * (color.a as f64 / 255.0))
            ));
        }
        None => s.push_str(r#"<Cell N="FillPattern" V="0"/>"#),
    }

    if stroke.is_invisible() {
        s.push_str(r#"<Cell N="LinePattern" V="0"/>"#);
    } else {
        let color = stroke.resolved_color();
        s.push_str(&format!(r#"<Cell N="LineColor" V="{}"/>"#, hex(color)));
        s.push_str(&format!(
            r#"<Cell N="LineWeight" V="{}"/>"#,
            num(inches(stroke.resolved_width()))
        ));
        s.push_str(&format!(
            r#"<Cell N="LinePattern" V="{}"/>"#,
            line_pattern(stroke.resolved_style())
        ));
    }

    let origin = (bb.x, bb.y + bb.h);
    let mut section = 0;
    if geo.stroked || geo.filled {
        s.push_str(&geometry_section(
            &geo.outline,
            origin,
            section,
            fill.is_none(),
            !geo.stroked,
        ));
        section += 1;
    }
    // Interior detail is stroked but never filled.
    if let Some(decoration) = &geo.decoration {
        s.push_str(&geometry_section(decoration, origin, section, true, false));
    }

    let text = plain_text(shape.text_str());
    if !text.trim().is_empty() {
        s.push_str(&format!("<Text>{}</Text>", escape(&text)));
    }

    s.push_str("</Shape>");
    Some(s)
}

fn line_shape(line: &Line, page: &Page, page_h: f64, id: u32) -> Option<String> {
    let route = dc_geom::route_line(line, page)?;
    let stroke = line.resolved_stroke();
    if stroke.is_invisible() {
        return None;
    }

    let bounds = kurbo::Shape::bounding_box(&route.path);
    // A perfectly straight run has no extent on one axis; Visio wants a box
    // with some size, so it is given a hairline of one.
    let bb = dc_model::BoundingBox::new(
        bounds.x0,
        bounds.y0,
        (bounds.width()).max(0.01),
        (bounds.height()).max(0.01),
    );

    let mut s = String::new();
    s.push_str(&format!(
        r#"<Shape ID="{id}" NameU="Connector{id}" Type="Shape" LineStyle="0" FillStyle="0" TextStyle="0">"#
    ));
    s.push_str(&placement(bb, page_h));
    s.push_str(r#"<Cell N="FillPattern" V="0"/>"#);
    s.push_str(&format!(
        r#"<Cell N="LineColor" V="{}"/>"#,
        hex(stroke.resolved_color())
    ));
    s.push_str(&format!(
        r#"<Cell N="LineWeight" V="{}"/>"#,
        num(inches(stroke.resolved_width()))
    ));
    s.push_str(&format!(
        r#"<Cell N="LinePattern" V="{}"/>"#,
        line_pattern(stroke.resolved_style())
    ));

    let origin = (bb.x, bb.y + bb.h);
    s.push_str(&geometry_section(&route.path, origin, 0, true, false));

    // Endpoint markers travel as their own closed outlines rather than as
    // Visio arrow styles: the published endpoint set is wider than Visio's,
    // so drawing them keeps every one of them rather than the few that map.
    let width = stroke.resolved_width();
    let mut section = 1;
    for (endpoint, tip, dir) in [
        (&line.endpoint1, route.start, -route.start_dir),
        (&line.endpoint2, route.end, -route.end_dir),
    ] {
        let Some(marker) = dc_geom::endpoint_marker(endpoint.style(), tip, -dir, width) else {
            continue;
        };
        let hollow = matches!(marker.fill, dc_geom::MarkerFill::Open);
        s.push_str(&geometry_section(&marker.path, origin, section, hollow, false));
        section += 1;
    }

    let label = line
        .labels()
        .first()
        .map(|l| plain_text(&l.text))
        .unwrap_or_default();
    if !label.trim().is_empty() {
        s.push_str(&format!("<Text>{}</Text>", escape(&label)));
    }

    s.push_str("</Shape>");
    Some(s)
}

/// The cells that place a shape on the page.
fn placement(bb: dc_model::BoundingBox, page_h: f64) -> String {
    let centre = bb.center();
    format!(
        r#"<Cell N="PinX" V="{}"/><Cell N="PinY" V="{}"/><Cell N="Width" V="{}"/><Cell N="Height" V="{}"/><Cell N="LocPinX" V="{}"/><Cell N="LocPinY" V="{}"/>"#,
        num(inches(centre.x)),
        num(inches(page_h - centre.y)),
        num(inches(bb.w)),
        num(inches(bb.h)),
        num(inches(bb.w) / 2.0),
        num(inches(bb.h) / 2.0),
    )
}

/// One geometry section, in coordinates local to the shape.
///
/// `origin` is the shape's lower-left corner in document points, which is
/// where Visio measures a shape's own geometry from.
fn geometry_section(
    path: &BezPath,
    origin: (f64, f64),
    index: usize,
    no_fill: bool,
    no_line: bool,
) -> String {
    let mut s = format!(r#"<Section N="Geometry" IX="{index}">"#);
    s.push_str(&format!(
        r#"<Cell N="NoFill" V="{}"/><Cell N="NoLine" V="{}"/>"#,
        u8::from(no_fill),
        u8::from(no_line)
    ));

    let mut row = 1;
    let emit = |s: &mut String, kind: &str, p: KPoint, row: &mut i32| {
        s.push_str(&format!(
            r#"<Row T="{kind}" IX="{}"><Cell N="X" V="{}"/><Cell N="Y" V="{}"/></Row>"#,
            row,
            num(inches(p.x - origin.0)),
            // Local y runs up from the shape's bottom edge.
            num(inches(origin.1 - p.y)),
        ));
        *row += 1;
    };

    // Curves are flattened; straight segments pass through untouched, so a
    // rectangle stays four rows rather than becoming hundreds.
    let mut start: Option<KPoint> = None;
    let mut current: Option<KPoint> = None;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                emit(&mut s, "MoveTo", p, &mut row);
                start = Some(p);
                current = Some(p);
            }
            PathEl::LineTo(p) => {
                emit(&mut s, "LineTo", p, &mut row);
                current = Some(p);
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
                let from = current.unwrap_or_default();
                let mut segment = BezPath::new();
                segment.move_to(from);
                segment.push(*el);
                kurbo::flatten(segment.elements().iter().copied(), FLATTEN_TOLERANCE, |e| {
                    if let PathEl::LineTo(p) = e {
                        emit(&mut s, "LineTo", p, &mut row);
                        current = Some(p);
                    }
                });
            }
            PathEl::ClosePath => {
                if let Some(p) = start {
                    emit(&mut s, "LineTo", p, &mut row);
                    current = Some(p);
                }
            }
        }
    }

    s.push_str("</Section>");
    s
}

/// Visio's line-pattern index for a stroke style.
///
/// 1 is solid, 2 a dash and 3 a dot; the format's three styles land on them
/// directly.
fn line_pattern(style: StrokeStyle) -> u8 {
    match style {
        StrokeStyle::Solid => 1,
        StrokeStyle::Dashed => 2,
        StrokeStyle::Dotted => 3,
    }
}

fn inches(points: f64) -> f64 {
    points / POINTS_PER_INCH
}

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Formats a number for a Visio cell, without exponent notation.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let rounded = (v * 100_000.0).round() / 100_000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// The readable text of a label, with its markup resolved.
///
/// Labels carry a small markup subset — `<b>`, `<i>` and the rest — which
/// Visio has no counterpart for in a plain `<Text>` element. Writing the raw
/// string would put the tags themselves on the page, so the markup is parsed
/// and only what it renders is kept.
fn plain_text(raw: &str) -> String {
    dc_model::markup::parse(raw, dc_model::TextStyle::default()).plain()
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Page, Shape, ShapeType, Text};
    use std::io::Cursor;

    fn document_with_a_shape() -> Document {
        let mut page = Page::new("p1", "Page-1");
        let mut shape =
            Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(72.0, 144.0, 144.0, 72.0));
        shape.text = Some(Text::new("Hello"));
        page.add_shape(shape);
        let mut document = Document::default();
        document.pages = vec![page];
        document
    }

    fn parts(document: &Document) -> std::collections::BTreeMap<String, String> {
        let mut buf = Cursor::new(Vec::new());
        write(document, &mut buf).unwrap();
        let bytes = buf.into_inner();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut out = std::collections::BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let name = entry.name().unwrap().to_string();
            let mut text = String::new();
            use std::io::Read;
            entry.read_to_string(&mut text).unwrap();
            out.insert(name, text);
        }
        out
    }

    #[test]
    fn the_package_holds_every_part_visio_requires() {
        let parts = parts(&document_with_a_shape());
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "visio/document.xml",
            "visio/_rels/document.xml.rels",
            "visio/pages/pages.xml",
            "visio/pages/_rels/pages.xml.rels",
            "visio/pages/page1.xml",
        ] {
            assert!(parts.contains_key(required), "missing {required}");
        }
    }

    #[test]
    fn every_part_is_declared_in_the_content_types() {
        let parts = parts(&document_with_a_shape());
        let types = &parts["[Content_Types].xml"];
        assert!(types.contains("/visio/document.xml"));
        assert!(types.contains("/visio/pages/pages.xml"));
        assert!(types.contains("/visio/pages/page1.xml"));
    }

    #[test]
    fn a_page_is_sized_in_inches() {
        let parts = parts(&document_with_a_shape());
        let pages = &parts["visio/pages/pages.xml"];
        // A default page is US Letter: 612×792 points is 8.5×11 inches.
        assert!(pages.contains(r#"<Cell N="PageWidth" V="8.5"/>"#), "{pages}");
        assert!(pages.contains(r#"<Cell N="PageHeight" V="11"/>"#), "{pages}");
    }

    #[test]
    fn a_shape_is_placed_by_its_centre_with_y_flipped() {
        let parts = parts(&document_with_a_shape());
        let page = &parts["visio/pages/page1.xml"];
        // The shape spans x 72..216 and y 144..216 in points, so its centre
        // is (144, 180) — two inches across, and 792-180 = 612 points up.
        assert!(page.contains(r#"<Cell N="PinX" V="2"/>"#), "{page}");
        assert!(page.contains(r#"<Cell N="PinY" V="8.5"/>"#), "{page}");
        assert!(page.contains(r#"<Cell N="Width" V="2"/>"#));
        assert!(page.contains(r#"<Cell N="Height" V="1"/>"#));
    }

    #[test]
    fn a_rectangle_stays_four_segments_rather_than_being_flattened() {
        let parts = parts(&document_with_a_shape());
        let page = &parts["visio/pages/page1.xml"];
        let moves = page.matches(r#"T="MoveTo""#).count();
        let lines = page.matches(r#"T="LineTo""#).count();
        assert_eq!(moves, 1, "one subpath");
        assert!(
            (4..=5).contains(&lines),
            "a rectangle should be a handful of lines, got {lines}"
        );
    }

    #[test]
    fn a_curve_is_flattened_into_several_segments() {
        let mut page = Page::new("p1", "Page-1");
        page.add_shape(Shape::new(
            "s1",
            ShapeType::Circle,
            BoundingBox::new(0.0, 0.0, 144.0, 144.0),
        ));
        let mut document = Document::default();
        document.pages = vec![page];

        let parts = parts(&document);
        let body = &parts["visio/pages/page1.xml"];
        let lines = body.matches(r#"T="LineTo""#).count();
        assert!(lines > 20, "a circle should flatten to many segments, got {lines}");
    }

    #[test]
    fn a_label_reaches_the_shape_and_is_escaped() {
        let mut document = document_with_a_shape();
        document.pages[0].shapes.as_mut().unwrap()[0].text = Some(Text::new("a < b & c"));
        let parts = parts(&document);
        let page = &parts["visio/pages/page1.xml"];
        assert!(page.contains("<Text>a &lt; b &amp; c</Text>"), "{page}");
    }

    #[test]
    fn label_markup_is_resolved_rather_than_written_as_tags() {
        let mut document = document_with_a_shape();
        document.pages[0].shapes.as_mut().unwrap()[0].text =
            Some(Text::new("<b>SLA:</b> two days"));
        let parts = parts(&document);
        let page = &parts["visio/pages/page1.xml"];

        assert!(page.contains("<Text>SLA: two days</Text>"), "{page}");
        assert!(
            !page.contains("&lt;b&gt;"),
            "the markup tags must not reach the page as text"
        );
    }

    #[test]
    fn an_unfilled_shape_turns_its_fill_pattern_off() {
        let mut page = Page::new("p1", "Page-1");
        // A text block has no outline to fill.
        page.add_shape(Shape::new(
            "s1",
            ShapeType::TextBlock,
            BoundingBox::new(0.0, 0.0, 100.0, 40.0),
        ));
        let mut document = Document::default();
        document.pages = vec![page];
        let parts = parts(&document);
        assert!(parts["visio/pages/page1.xml"].contains(r#"<Cell N="FillPattern" V="0"/>"#));
    }

    #[test]
    fn every_page_becomes_a_visio_page() {
        let mut document = Document::default();
        document.pages = vec![
            Page::new("a", "First"),
            Page::new("b", "Second"),
            Page::new("c", "Third"),
        ];
        let parts = parts(&document);
        assert!(parts.contains_key("visio/pages/page3.xml"));
        let pages = &parts["visio/pages/pages.xml"];
        assert!(pages.contains(r#"NameU="Second""#), "{pages}");
        assert_eq!(pages.matches("<Page ").count(), 3);
    }

    #[test]
    fn what_is_written_can_be_read_back_by_our_own_importer() {
        // The strongest check available without Visio itself: the importer
        // was written from the format's own documentation, so if it can read
        // the package then the parts, relationships and units line up.
        let document = document_with_a_shape();
        let mut buf = Cursor::new(Vec::new());
        write(&document, &mut buf).unwrap();

        let imported = crate::read(Cursor::new(buf.into_inner()))
            .expect("our importer should accept our own output");

        assert_eq!(imported.document.pages.len(), 1);
        let shapes = imported.document.pages[0].shapes();
        assert_eq!(shapes.len(), 1, "the shape should survive the trip");

        // Back in points, within a rounding hair of where it started.
        let bb = shapes[0].bounding_box;
        assert!((bb.w - 144.0).abs() < 1.0, "width came back as {}", bb.w);
        assert!((bb.h - 72.0).abs() < 1.0, "height came back as {}", bb.h);
        assert!((bb.x - 72.0).abs() < 1.0, "x came back as {}", bb.x);
        assert!((bb.y - 144.0).abs() < 1.0, "y came back as {}", bb.y);
        assert_eq!(shapes[0].text_str(), "Hello");
    }

    #[test]
    fn a_whole_diagram_survives_the_round_trip() {
        // Several shapes and the connectors between them, which is what a
        // real drawing is made of.
        let mut page = Page::new("p1", "Flow");
        for (i, (x, y)) in [(72.0, 72.0), (72.0, 216.0), (288.0, 216.0)].iter().enumerate() {
            let mut shape = Shape::new(
                format!("s{i}"),
                ShapeType::Process,
                BoundingBox::new(*x, *y, 120.0, 54.0),
            );
            shape.text = Some(Text::new(format!("Step {i}")));
            page.add_shape(shape);
        }
        page.add_line(dc_model::Line::new(
            "l0",
            dc_model::Endpoint::at_shape("s0", dc_model::EndpointStyle::None),
            dc_model::Endpoint::at_shape("s1", dc_model::EndpointStyle::Arrow),
        ));
        let mut document = Document::default();
        document.pages = vec![page];

        let mut buf = Cursor::new(Vec::new());
        write(&document, &mut buf).unwrap();
        let imported = crate::read(Cursor::new(buf.into_inner())).unwrap();

        let shapes = imported.document.pages[0].shapes();
        assert!(
            shapes.len() >= 3,
            "all three shapes plus the connector should arrive, got {}",
            shapes.len()
        );
        let labels: Vec<&str> = shapes.iter().map(|s| s.text_str()).collect();
        for expected in ["Step 0", "Step 1", "Step 2"] {
            assert!(labels.contains(&expected), "lost {expected}: {labels:?}");
        }
    }

    #[test]
    fn a_document_with_no_pages_is_refused() {
        let mut document = Document::default();
        document.pages.clear();
        let mut buf = Cursor::new(Vec::new());
        assert!(matches!(
            write(&document, &mut buf),
            Err(WriteError::NoPages)
        ));
    }

    #[test]
    fn the_page_grows_to_hold_a_drawing_that_runs_off_the_sheet() {
        let mut page = Page::new("p1", "Page-1");
        // Well beyond the 612×792 default sheet.
        page.add_shape(Shape::new(
            "s1",
            ShapeType::Rectangle,
            BoundingBox::new(1000.0, 100.0, 200.0, 100.0),
        ));
        let mut document = Document::default();
        document.pages = vec![page];
        let parts = parts(&document);
        let pages = &parts["visio/pages/pages.xml"];
        assert!(
            !pages.contains(r#"<Cell N="PageWidth" V="8.5"/>"#),
            "the sheet should have been widened: {pages}"
        );
    }
}
