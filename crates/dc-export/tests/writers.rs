//! What both writers must produce from a known document.
//!
//! These read the output rather than a screenshot, so they pin down the
//! things a picture cannot show: that text is real text, that an overflowing
//! label is confined, and that a PDF's structure is actually well formed.

use dc_export::Options;
use dc_model::{BoundingBox, Document, Page, Shape, ShapeType, Text};

/// A one-page document holding a single labelled rectangle.
fn document_with(text: &str, w: f64, h: f64) -> Document {
    let mut page = Page::new("p1", "Page 1");
    let mut shape = Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(40.0, 40.0, w, h));
    shape.text = Some(Text::new(text));
    page.add_shape(shape);

    let mut document = Document::default();
    document.pages = vec![page];
    document
}

fn svg_of(document: &Document) -> String {
    let drawing = dc_export::drawing_for(document, 0).unwrap();
    dc_export::svg::render(&drawing, Options::default())
}

fn pdf_of(document: &Document) -> String {
    let drawing = dc_export::drawing_for(document, 0).unwrap();
    String::from_utf8_lossy(&dc_export::pdf::render(&drawing, Options::default())).into_owned()
}

#[test]
fn svg_writes_the_label_as_real_text() {
    let svg = svg_of(&document_with("Validate fields", 160.0, 60.0));
    assert!(svg.contains("<text "), "there should be a text element");
    assert!(
        svg.contains(">Validate fields</text>"),
        "the label should be readable in the markup, not outlines"
    );
}

#[test]
fn svg_declares_its_size_and_frames_the_content() {
    let svg = svg_of(&document_with("x", 160.0, 60.0));
    assert!(svg.starts_with("<?xml"), "it should be a well formed document");
    assert!(svg.contains(r#"xmlns="http://www.w3.org/2000/svg""#));
    assert!(svg.contains("viewBox="), "a viewBox is what makes it scale");
    assert!(svg.trim_end().ends_with("</svg>"));
}

#[test]
fn svg_escapes_markup_characters_in_a_label() {
    let svg = svg_of(&document_with("a < b & c", 300.0, 60.0));
    assert!(svg.contains("a &lt; b &amp; c"), "got: {svg}");
    assert!(!svg.contains("a < b & c"), "the raw characters would break the file");
}

#[test]
fn a_label_that_fits_is_not_clipped() {
    let svg = svg_of(&document_with("Short", 200.0, 80.0));
    assert!(
        !svg.contains("<clipPath"),
        "text that fits should not pay for a clip path"
    );
}

#[test]
fn a_label_taller_than_its_shape_is_confined_to_it() {
    // Far more text than a 60×30 box can hold.
    let svg = svg_of(&document_with(&"word ".repeat(40), 60.0, 30.0));
    assert!(
        svg.contains("<clipPath"),
        "an overflowing label must be clipped, as it is on the canvas"
    );
    assert!(svg.contains("clip-path=\"url(#tc"), "and the clip must be referenced");
}

#[test]
fn the_transparent_option_leaves_the_sheet_unpainted() {
    let document = document_with("x", 160.0, 60.0);
    let drawing = dc_export::drawing_for(&document, 0).unwrap();

    let opaque = dc_export::svg::render(&drawing, Options { transparent: false });
    let clear = dc_export::svg::render(&drawing, Options { transparent: true });
    assert!(opaque.contains("<rect"), "an opaque export paints the sheet");
    assert!(
        clear.matches("<rect").count() < opaque.matches("<rect").count(),
        "a transparent export paints one fewer rectangle"
    );
}

#[test]
fn pdf_is_structurally_complete() {
    let pdf = pdf_of(&document_with("Validate fields", 160.0, 60.0));
    assert!(pdf.starts_with("%PDF-1.7"), "it needs a version header");
    assert!(pdf.contains("/Type /Catalog"));
    assert!(pdf.contains("/Type /Pages"));
    assert!(pdf.contains("/Type /Page"));
    assert!(pdf.contains("/MediaBox"));
    assert!(pdf.contains("xref"), "a reader needs the cross-reference table");
    assert!(pdf.contains("trailer"));
    assert!(pdf.contains("startxref"));
    assert!(pdf.trim_end().ends_with("%%EOF"));
}

#[test]
fn pdf_cross_references_point_at_real_objects() {
    let bytes = {
        let document = document_with("Validate fields", 160.0, 60.0);
        let drawing = dc_export::drawing_for(&document, 0).unwrap();
        dc_export::pdf::render(&drawing, Options::default())
    };
    let text = String::from_utf8_lossy(&bytes);

    // Every offset in the table must land on the `N 0 obj` it claims.
    let xref_at = text.rfind("xref\n").expect("a table");
    let table = &text[xref_at..];
    let mut lines = table.lines().skip(2); // past "xref" and the count
    let _free = lines.next();
    for (i, line) in lines.enumerate() {
        let Some(offset) = line.split_whitespace().next() else {
            break;
        };
        let Ok(offset) = offset.parse::<usize>() else {
            break;
        };
        if offset == 0 {
            break;
        }
        let expected = format!("{} 0 obj", i + 1);
        assert!(
            text[offset..].starts_with(&expected),
            "object {} should start at {offset}, found {:?}",
            i + 1,
            &text[offset..(offset + 20).min(text.len())]
        );
    }
}

#[test]
fn pdf_writes_the_label_as_real_text_in_a_standard_font() {
    let pdf = pdf_of(&document_with("Validate fields", 160.0, 60.0));
    assert!(
        pdf.contains("(Validate fields) Tj"),
        "the label should be a text-showing operator"
    );
    assert!(
        pdf.contains("/BaseFont /Helvetica"),
        "and should use a standard font rather than an embedded one"
    );
    assert!(
        !pdf.contains("/FontFile"),
        "nothing should be embedded: that is the point of the base fonts"
    );
}

#[test]
fn pdf_confines_an_overflowing_label() {
    let pdf = pdf_of(&document_with(&"word ".repeat(40), 60.0, 30.0));
    assert!(
        pdf.contains(" re W n "),
        "an overflowing label needs a clip, as it has on the canvas"
    );
}

#[test]
fn both_writers_frame_the_same_region() {
    let document = document_with("Validate fields", 160.0, 60.0);
    let drawing = dc_export::drawing_for(&document, 0).unwrap();
    let region = drawing.region;

    let svg = dc_export::svg::render(&drawing, Options::default());
    let pdf = String::from_utf8_lossy(&dc_export::pdf::render(&drawing, Options::default()))
        .into_owned();

    // The same width reaches the SVG root and the PDF media box.
    let w = format!("{}", (region.w * 1000.0).round() / 1000.0);
    assert!(svg.contains(&format!(r#"width="{w}""#)), "svg width {w}");
    assert!(pdf.contains(&format!("/MediaBox [0 0 {w} ")), "pdf media box {w}");
}

#[test]
fn a_document_with_no_pages_is_refused_rather_than_written_blank() {
    // A default document already has one page, so the pages have to be
    // cleared to reach the case at all.
    let mut document = Document::default();
    document.pages.clear();
    assert!(dc_export::drawing_for(&document, 0).is_err());
}

#[test]
fn a_page_with_no_shapes_still_exports_a_sheet() {
    let document = Document::default();
    let drawing = dc_export::drawing_for(&document, 0).expect("the default page");
    assert!(drawing.region.w > 0.0 && drawing.region.h > 0.0);
    assert!(svg_of(&document).contains("<svg"));
}

#[test]
fn a_page_index_past_the_end_falls_back_to_the_first() {
    let document = document_with("x", 160.0, 60.0);
    assert!(dc_export::drawing_for(&document, 99).is_ok());
}
