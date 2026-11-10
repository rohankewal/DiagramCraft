//! A sample document, used by the demo command and by rendering tests.

use dc_model::{
    BoundingBox, Color, Document, Endpoint, EndpointStyle, Fill, Line, LineType, Shape, ShapeType,
    Stroke, Style, Units,
};

fn styled(fill: Color, stroke: Color) -> Style {
    Style {
        fill: Some(Fill::solid(fill)),
        stroke: Some(Stroke::new(stroke, 1.5)),
        rounding: None,
        text_color: None,
        ..Default::default()
    }
}

/// A small approval flowchart exercising several shape types, all three
/// connector routings, and labeled edges.
pub fn approval_flow() -> Document {
    let mut doc = Document::new();
    doc.set_units(Units::Pt);
    doc.pages[0].title = "Approval".to_string();

    let blue = styled(Color::rgb(0xdb, 0xea, 0xfe), Color::rgb(0x25, 0x63, 0xeb));
    let amber = styled(Color::rgb(0xfe, 0xf3, 0xc7), Color::rgb(0xca, 0x8a, 0x04));
    let green = styled(Color::rgb(0xd1, 0xfa, 0xe5), Color::rgb(0x05, 0x96, 0x69));
    let red = styled(Color::rgb(0xfe, 0xe2, 0xe2), Color::rgb(0xdc, 0x26, 0x26));
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));

    let page = &mut doc.pages[0];
    page.add_shape(
        Shape::new("start", ShapeType::Terminator, BoundingBox::new(200.0, 40.0, 160.0, 54.0))
            .with_text("Request submitted")
            .with_style(blue.clone()),
    );
    page.add_shape(
        Shape::new("intake", ShapeType::Process, BoundingBox::new(200.0, 140.0, 160.0, 56.0))
            .with_text("Validate fields")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("data", ShapeType::Database, BoundingBox::new(430.0, 140.0, 130.0, 70.0))
            .with_text("Records")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("check", ShapeType::Decision, BoundingBox::new(195.0, 240.0, 170.0, 110.0))
            .with_text("Within budget?")
            .with_style(amber),
    );
    page.add_shape(
        Shape::new("approve", ShapeType::Process, BoundingBox::new(30.0, 400.0, 160.0, 56.0))
            .with_text("Approve")
            .with_style(green.clone()),
    );
    page.add_shape(
        Shape::new("escalate", ShapeType::ManualOperation, BoundingBox::new(370.0, 400.0, 170.0, 60.0))
            .with_text("Escalate to finance")
            .with_style(red),
    );
    page.add_shape(
        Shape::new("doc", ShapeType::Document, BoundingBox::new(30.0, 510.0, 160.0, 70.0))
            .with_text("Notify requester")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("done", ShapeType::Terminator, BoundingBox::new(200.0, 620.0, 160.0, 54.0))
            .with_text("Closed")
            .with_style(green),
    );
    page.add_shape(
        Shape::new("note", ShapeType::Note, BoundingBox::new(430.0, 510.0, 170.0, 90.0))
            .with_text("<b>SLA:</b> two business days")
            .with_style(plain),
    );

    let edge = |id: &str, from: &str, to: &str| {
        Line::new(
            id,
            Endpoint::at_shape(from, EndpointStyle::None),
            Endpoint::at_shape(to, EndpointStyle::Arrow),
        )
        .with_line_type(LineType::Elbow)
    };

    page.add_line(edge("e1", "start", "intake"));
    page.add_line(edge("e2", "intake", "check"));
    page.add_line(
        Line::new(
            "e3",
            Endpoint::at_shape("intake", EndpointStyle::None),
            Endpoint::at_shape("data", EndpointStyle::OpenArrow),
        )
        .with_line_type(LineType::Straight)
        .with_label("read"),
    );
    page.add_line(edge("e4", "check", "approve").with_label("yes"));
    page.add_line(edge("e5", "check", "escalate").with_label("no"));
    page.add_line(edge("e6", "approve", "doc"));
    page.add_line(edge("e7", "doc", "done"));
    page.add_line(
        Line::new(
            "e8",
            Endpoint::at_shape("escalate", EndpointStyle::None),
            Endpoint::at_shape("done", EndpointStyle::Arrow),
        )
        .with_line_type(LineType::Curved),
    );

    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_lucid::LucidPackage;

    #[test]
    fn the_sample_is_valid() {
        let package = LucidPackage::new(approval_flow());
        let issues = package.validate();
        assert_eq!(issues, vec![], "the shipped sample must be clean");
    }

    #[test]
    fn the_sample_exercises_all_three_routings() {
        let doc = approval_flow();
        let types: Vec<LineType> = doc.pages[0].lines().iter().map(|l| l.line_type).collect();
        assert!(types.contains(&LineType::Elbow));
        assert!(types.contains(&LineType::Straight));
        assert!(types.contains(&LineType::Curved));
    }

    #[test]
    fn every_connector_in_the_sample_resolves() {
        let doc = approval_flow();
        let page = &doc.pages[0];
        let routes = dc_geom::Router::new(page).route_all();
        assert_eq!(
            routes.len(),
            page.lines().len(),
            "some connector failed to route"
        );
    }

    #[test]
    fn the_sample_round_trips_through_the_package_format() {
        let package = LucidPackage::new(approval_flow());
        let bytes = package.to_bytes().unwrap();
        let back = LucidPackage::from_bytes(&bytes).unwrap();
        assert_eq!(back.document, package.document);
    }
}
