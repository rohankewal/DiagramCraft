//! End-to-end import tests.
//!
//! The fixtures are built here rather than checked in as binaries, so every
//! byte a test asserts on is visible in the test itself.

use super::*;
use dc_model::{Color, Point};
use std::io::{Cursor, Write};

/// The fixed position an endpoint carries, when it has one.
fn endpoint_position(endpoint: &Endpoint) -> Option<Point> {
    match endpoint {
        Endpoint::PositionEndpoint { position, .. } => Some(*position),
        _ => None,
    }
}

/// Builds a `.vsdx` package from a list of parts.
fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in parts {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn rels(entries: &[(&str, &str)]) -> String {
    let mut out = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, target) in entries {
        out.push_str(&format!(
            r#"<Relationship Id="{id}" Target="{target}" Type="http://schemas.microsoft.com/visio/2010/relationships/page"/>"#
        ));
    }
    out.push_str("</Relationships>");
    out
}

const PAGES_ONE: &str = r#"<Pages xmlns="http://schemas.microsoft.com/office/visio/2012/main"
     xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <Page ID="0" NameU="Flow">
    <PageSheet>
      <Cell N="PageWidth" V="8.5"/>
      <Cell N="PageHeight" V="11"/>
    </PageSheet>
    <Rel r:id="rId1"/>
  </Page>
</Pages>"#;

/// A drawing with two flowchart shapes joined by a connector.
fn simple_drawing() -> Vec<u8> {
    let page = r#"<PageContents xmlns="http://schemas.microsoft.com/office/visio/2012/main">
  <Shapes>
    <Shape ID="1" NameU="Start/End" Type="Shape" Master="2">
      <Cell N="PinX" V="4.25"/><Cell N="PinY" V="10"/>
      <Cell N="Width" V="2"/><Cell N="Height" V="0.75"/>
      <Cell N="LocPinX" V="1"/><Cell N="LocPinY" V="0.375"/>
      <Text>Start</Text>
    </Shape>
    <Shape ID="2" NameU="Decision" Type="Shape" Master="3">
      <Cell N="PinX" V="4.25"/><Cell N="PinY" V="8"/>
      <Cell N="Width" V="2"/><Cell N="Height" V="1.5"/>
      <Cell N="LocPinX" V="1"/><Cell N="LocPinY" V="0.75"/>
      <Text>Approved?</Text>
    </Shape>
    <Shape ID="5" NameU="Dynamic connector" Type="Shape" Master="7">
      <Cell N="BeginX" V="4.25"/><Cell N="BeginY" V="9.625"/>
      <Cell N="EndX" V="4.25"/><Cell N="EndY" V="8.75"/>
      <Cell N="Width" V="0"/><Cell N="Height" V="0.875"/>
      <Text>yes</Text>
    </Shape>
  </Shapes>
  <Connects>
    <Connect FromSheet="5" FromCell="BeginX" FromPart="9" ToSheet="1" ToCell="PinX" ToPart="3"/>
    <Connect FromSheet="5" FromCell="EndX" FromPart="12" ToSheet="2" ToCell="PinX" ToPart="3"/>
  </Connects>
</PageContents>"#;

    let masters = r#"<Masters xmlns="http://schemas.microsoft.com/office/visio/2012/main"
      xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <Master ID="2" NameU="Start/End"/>
  <Master ID="3" NameU="Decision"/>
  <Master ID="7" NameU="Dynamic connector"/>
</Masters>"#;

    package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
        ("visio/masters/masters.xml", masters),
    ])
}

fn import(bytes: &[u8]) -> Import {
    read(Cursor::new(bytes.to_vec())).expect("the drawing should import")
}

#[test]
fn a_drawing_imports_its_pages_shapes_and_connectors() {
    let result = import(&simple_drawing());
    let page = &result.document.pages[0];

    assert_eq!(result.document.pages.len(), 1);
    assert_eq!(page.title, "Flow");
    assert_eq!(page.shapes().len(), 2);
    assert_eq!(page.lines().len(), 1);
}

#[test]
fn masters_are_mapped_onto_shape_types() {
    let result = import(&simple_drawing());
    let types: Vec<&str> = result.document.pages[0]
        .shapes()
        .iter()
        .map(|s| s.shape_type.as_str())
        .collect();
    assert_eq!(types, vec!["terminator", "decision"]);
}

#[test]
fn shape_text_comes_across() {
    let result = import(&simple_drawing());
    let texts: Vec<&str> = result.document.pages[0]
        .shapes()
        .iter()
        .map(|s| s.text_str())
        .collect();
    assert_eq!(texts, vec!["Start", "Approved?"]);
}

#[test]
fn geometry_is_converted_to_a_top_left_origin() {
    let result = import(&simple_drawing());
    let start = &result.document.pages[0].shapes()[0];

    // 2 x 0.75 inches, centred at 4.25 across and 10 up an 11 inch page.
    assert_eq!(start.bounding_box.w, 144.0);
    assert_eq!(start.bounding_box.h, 54.0);
    assert_eq!(start.bounding_box.x, 3.25 * 72.0);
    // Its top edge is 10.375 inches up, so 0.625 inches below the page top.
    assert!(
        (start.bounding_box.y - 0.625 * 72.0).abs() < 1e-9,
        "y was {}",
        start.bounding_box.y
    );
}

#[test]
fn the_page_keeps_its_size() {
    let result = import(&simple_drawing());
    let settings = result.document.pages[0].resolved_settings();
    assert_eq!(
        settings.bounds_points(result.document.units()),
        Some((612.0, 792.0)),
        "an 8.5 by 11 inch page"
    );
}

#[test]
fn connectors_attach_to_the_shapes_they_were_glued_to() {
    let result = import(&simple_drawing());
    let page = &result.document.pages[0];
    let line = &page.lines()[0];

    let start = &page.shapes()[0].id;
    let decision = &page.shapes()[1].id;
    assert_eq!(line.endpoint1.shape_id(), Some(start.as_str()));
    assert_eq!(line.endpoint2.shape_id(), Some(decision.as_str()));
}

#[test]
fn connector_labels_come_across() {
    let result = import(&simple_drawing());
    let line = &result.document.pages[0].lines()[0];
    assert_eq!(line.labels().first().map(|l| l.text.as_str()), Some("yes"));
}

#[test]
fn an_unglued_connector_keeps_its_own_coordinates() {
    let page = r#"<PageContents xmlns="x">
      <Shapes>
        <Shape ID="5" NameU="Dynamic connector" Type="Shape">
          <Cell N="BeginX" V="1"/><Cell N="BeginY" V="10"/>
          <Cell N="EndX" V="4"/><Cell N="EndY" V="6"/>
        </Shape>
      </Shapes>
    </PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
    ]);

    let result = import(&bytes);
    let line = &result.document.pages[0].lines()[0];
    assert_eq!(line.endpoint1.shape_id(), None);
    assert_eq!(endpoint_position(&line.endpoint1), Some(Point::new(72.0, 72.0)));
    assert_eq!(endpoint_position(&line.endpoint2), Some(Point::new(288.0, 360.0)));
}

#[test]
fn an_unknown_master_imports_as_a_rectangle_and_is_reported() {
    let page = r#"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Shape" Master="9">
          <Cell N="PinX" V="2"/><Cell N="PinY" V="9"/>
          <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
          <Text>Rack</Text>
        </Shape>
      </Shapes></PageContents>"#;
    let masters = r#"<Masters xmlns="x"><Master ID="9" NameU="Rack Unit 42U"/></Masters>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
        ("visio/masters/masters.xml", masters),
    ]);

    let result = import(&bytes);
    let shape = &result.document.pages[0].shapes()[0];
    assert_eq!(shape.shape_type, ShapeType::Rectangle);
    assert_eq!(shape.text_str(), "Rack", "the content still arrives");
    assert_eq!(
        shape.extra.get("vsdxMaster").and_then(|v| v.as_str()),
        Some("Rack Unit 42U"),
        "and what it was is recorded"
    );
    assert!(
        result.warnings.iter().any(|w| w.contains("Rack Unit 42U")),
        "got {:?}",
        result.warnings
    );
}

#[test]
fn a_shape_inherits_size_from_its_master() {
    let page = r#"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Shape" Master="2">
          <Cell N="PinX" V="2"/><Cell N="PinY" V="9"/>
        </Shape>
      </Shapes></PageContents>"#;
    let masters = r#"<Masters xmlns="x"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
        <Master ID="2" NameU="Process"><Rel r:id="rId1"/></Master>
      </Masters>"#;
    let master1 = r#"<MasterContents xmlns="x"><Shapes>
        <Shape ID="5" Type="Shape">
          <Cell N="Width" V="2"/><Cell N="Height" V="0.75"/>
        </Shape>
      </Shapes></MasterContents>"#;

    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
        ("visio/masters/masters.xml", masters),
        ("visio/masters/_rels/masters.xml.rels", &rels(&[("rId1", "master1.xml")])),
        ("visio/masters/master1.xml", master1),
    ]);

    let result = import(&bytes);
    let shape = &result.document.pages[0].shapes()[0];
    assert_eq!(shape.shape_type, ShapeType::Process);
    assert_eq!((shape.bounding_box.w, shape.bounding_box.h), (144.0, 54.0));
}

#[test]
fn groups_are_flattened_to_their_members() {
    let page = r#"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Group" NameU="Group">
          <Cell N="PinX" V="4"/><Cell N="PinY" V="9"/>
          <Cell N="Width" V="3"/><Cell N="Height" V="2"/>
          <Shapes>
            <Shape ID="2" Type="Shape" NameU="Process">
              <Cell N="PinX" V="1"/><Cell N="PinY" V="9"/>
              <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
            </Shape>
            <Shape ID="3" Type="Shape" NameU="Decision">
              <Cell N="PinX" V="3"/><Cell N="PinY" V="9"/>
              <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
            </Shape>
          </Shapes>
        </Shape>
      </Shapes></PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
    ]);

    let result = import(&bytes);
    let types: Vec<&str> = result.document.pages[0]
        .shapes()
        .iter()
        .map(|s| s.shape_type.as_str())
        .collect();
    assert_eq!(types, vec!["process", "decision"], "the frame itself is not drawn");
}

#[test]
fn a_labelled_group_keeps_its_label() {
    let page = r#"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Group" NameU="Group">
          <Cell N="PinX" V="4"/><Cell N="PinY" V="9"/>
          <Cell N="Width" V="3"/><Cell N="Height" V="2"/>
          <Text>Phase one</Text>
          <Shapes>
            <Shape ID="2" Type="Shape" NameU="Process">
              <Cell N="PinX" V="1"/><Cell N="PinY" V="9"/>
              <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
            </Shape>
          </Shapes>
        </Shape>
      </Shapes></PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
    ]);

    let result = import(&bytes);
    let shapes = result.document.pages[0].shapes();
    assert_eq!(shapes.len(), 2);
    assert!(
        shapes.iter().any(|s| s.text_str() == "Phase one"),
        "the group label should survive"
    );
}

#[test]
fn several_pages_import_in_order() {
    let pages = r#"<Pages xmlns="x" xmlns:r="y">
      <Page ID="0" NameU="First"><PageSheet><Cell N="PageHeight" V="11"/></PageSheet><Rel r:id="rId1"/></Page>
      <Page ID="1" NameU="Second"><PageSheet><Cell N="PageHeight" V="11"/></PageSheet><Rel r:id="rId2"/></Page>
    </Pages>"#;
    let empty = r#"<PageContents xmlns="x"><Shapes/></PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", pages),
        (
            "visio/pages/_rels/pages.xml.rels",
            &rels(&[("rId1", "page1.xml"), ("rId2", "page2.xml")]),
        ),
        ("visio/pages/page1.xml", empty),
        ("visio/pages/page2.xml", empty),
    ]);

    let result = import(&bytes);
    let titles: Vec<&str> = result.document.pages.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, vec!["First", "Second"]);
}

#[test]
fn background_pages_are_skipped() {
    let pages = r#"<Pages xmlns="x" xmlns:r="y">
      <Page ID="0" NameU="Visible"><Rel r:id="rId1"/></Page>
      <Page ID="1" NameU="Backdrop" Background="1"><Rel r:id="rId2"/></Page>
    </Pages>"#;
    let empty = r#"<PageContents xmlns="x"><Shapes/></PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", pages),
        (
            "visio/pages/_rels/pages.xml.rels",
            &rels(&[("rId1", "page1.xml"), ("rId2", "page2.xml")]),
        ),
        ("visio/pages/page1.xml", empty),
        ("visio/pages/page2.xml", empty),
    ]);

    let result = import(&bytes);
    assert_eq!(result.document.pages.len(), 1);
    assert_eq!(result.document.pages[0].title, "Visible");
}

#[test]
fn every_imported_id_is_unique_across_the_document() {
    let result = import(&simple_drawing());
    let mut seen = std::collections::HashSet::new();
    for page in &result.document.pages {
        assert!(seen.insert(page.id.clone()), "duplicate page id {}", page.id);
        for shape in page.shapes() {
            assert!(seen.insert(shape.id.clone()), "duplicate id {}", shape.id);
        }
        for line in page.lines() {
            assert!(seen.insert(line.id.clone()), "duplicate id {}", line.id);
        }
    }
}

#[test]
fn imported_ids_satisfy_the_target_formats_rules() {
    let result = import(&simple_drawing());
    for page in &result.document.pages {
        assert!(dc_model::is_valid_id(&page.id), "{}", page.id);
        for shape in page.shapes() {
            assert!(dc_model::is_valid_id(&shape.id), "{}", shape.id);
        }
    }
}

#[test]
fn styling_comes_across() {
    let page = r##"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Shape" NameU="Process">
          <Cell N="PinX" V="2"/><Cell N="PinY" V="9"/>
          <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
          <Cell N="FillForegnd" V="#dbeafe"/>
          <Cell N="LineColor" V="#2563eb"/>
          <Cell N="LineWeight" V="0.0277777777777778"/>
        </Shape>
      </Shapes></PageContents>"##;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
    ]);

    let result = import(&bytes);
    let style = result.document.pages[0].shapes()[0].resolved_style();
    assert_eq!(style.resolved_fill().color(), Some(Color::rgb(0xdb, 0xea, 0xfe)));
    assert_eq!(style.resolved_stroke().resolved_color(), Color::rgb(0x25, 0x63, 0xeb));
    assert!((style.resolved_stroke().resolved_width() - 2.0).abs() < 0.01);
}

#[test]
fn label_text_is_escaped_rather_than_read_back_as_markup() {
    let page = r#"<PageContents xmlns="x"><Shapes>
        <Shape ID="1" Type="Shape" NameU="Process">
          <Cell N="Width" V="1"/><Cell N="Height" V="1"/>
          <Text>if x &lt; y &amp; y &gt; z</Text>
        </Shape>
      </Shapes></PageContents>"#;
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", page),
    ]);

    let result = import(&bytes);
    let stored = result.document.pages[0].shapes()[0].text_str().to_string();
    assert_eq!(
        dc_model::markup::parse(&stored, Default::default()).plain(),
        "if x < y & y > z"
    );
}

#[test]
fn multi_line_labels_become_breaks() {
    assert_eq!(escape_markup("one\ntwo"), "one<br>two");
    assert_eq!(escape_markup("trailing\n"), "trailing", "a terminator is not a line");
}

#[test]
fn a_file_that_is_not_a_drawing_is_reported_clearly() {
    let not_zip = read(Cursor::new(b"this is not a zip".to_vec()));
    assert!(matches!(not_zip, Err(Error::Zip(_))), "got {not_zip:?}");

    let wrong_zip = package(&[("hello.txt", "hi")]);
    let err = read(Cursor::new(wrong_zip)).unwrap_err();
    assert!(matches!(err, Error::NotAVisioDrawing), "got {err:?}");
}

#[test]
fn a_drawing_with_no_foreground_pages_still_opens() {
    let pages = r#"<Pages xmlns="x"><Page ID="0" NameU="Backdrop" Background="1"/></Pages>"#;
    let bytes = package(&[("visio/pages/pages.xml", pages)]);
    let result = import(&bytes);
    assert_eq!(result.document.pages.len(), 1, "a document needs a page");
    assert!(!result.warnings.is_empty());
}

#[test]
fn a_page_whose_contents_part_is_missing_is_reported_not_fatal() {
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
    ]);
    let result = import(&bytes);
    assert_eq!(result.document.pages.len(), 1);
    assert_eq!(result.document.pages[0].shapes().len(), 0);
    assert!(!result.warnings.is_empty());
}

#[test]
fn a_traversal_entry_is_ignored_rather_than_followed() {
    let bytes = package(&[
        ("visio/pages/pages.xml", PAGES_ONE),
        ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml")])),
        ("visio/pages/page1.xml", r#"<PageContents xmlns="x"><Shapes/></PageContents>"#),
        ("../escape.xml", "<nope/>"),
    ]);
    let result = import(&bytes);
    assert_eq!(result.document.pages.len(), 1);
}

#[test]
fn the_imported_document_is_measured_in_points() {
    let result = import(&simple_drawing());
    assert_eq!(result.document.units(), Units::Pt);
}

#[test]
fn the_summary_counts_what_arrived() {
    let result = import(&simple_drawing());
    let summary = result.summary();
    assert!(summary.contains("1 page"), "got {summary}");
    assert!(summary.contains("2 shape"), "got {summary}");
    assert!(summary.contains("1 connector"), "got {summary}");
}

#[test]
fn visio_extensions_are_recognized() {
    assert!(is_vsdx_path(Path::new("diagram.vsdx")));
    assert!(is_vsdx_path(Path::new("diagram.VSDX")));
    assert!(is_vsdx_path(Path::new("macro.vsdm")));
    assert!(!is_vsdx_path(Path::new("diagram.lucid")));
    assert!(!is_vsdx_path(Path::new("diagram")));
}

#[test]
fn an_imported_drawing_survives_the_lucid_round_trip() {
    // The whole point of importing is to save it out again, so the result has
    // to be representable: valid ids, resolvable endpoints, nothing exotic.
    let result = import(&simple_drawing());
    let json = serde_json::to_string(&result.document).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, result.document);
}

#[test]
fn importing_from_a_file_on_disk_works() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drawing.vsdx");
    std::fs::write(&path, simple_drawing()).unwrap();

    let result = read_path(&path).unwrap();
    assert_eq!(result.document.pages[0].shapes().len(), 2);
}
