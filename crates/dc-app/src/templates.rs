//! Starter templates, offered from the "New from Template" gallery.
//!
//! Each one is a plain `Document` builder, the same way `samples.rs`'s
//! `approval_flow` is — just picked from a list instead of hard-coded to one
//! command. Built only from the published shape and line vocabulary, the
//! same constraint the rest of this crate holds to; see
//! `docs/FORMAT-PROVENANCE.md`.

use dc_model::{
    BoundingBox, Color, Document, Endpoint, EndpointStyle, Fill, Line, LineType, Shape, ShapeType,
    Stroke, Style, Units,
};

fn styled(fill: Color, stroke: Color) -> Style {
    Style { fill: Some(Fill::solid(fill)), stroke: Some(Stroke::new(stroke, 1.5)), ..Default::default() }
}

/// A gallery entry: a name, a one-line description, and the builder itself.
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub build: fn() -> Document,
}

pub const ALL: &[Template] = &[
    Template {
        name: "Basic Flowchart",
        description: "A start-to-end process with a decision and a revise loop.",
        build: basic_flowchart,
    },
    Template {
        name: "Org Chart",
        description: "A manager with direct reports, two levels deep.",
        build: org_chart,
    },
    Template {
        name: "Cross-Functional Flowchart",
        description: "A swimlane diagram showing handoff between teams.",
        build: cross_functional_flowchart,
    },
    Template {
        name: "Network Diagram",
        description: "A simple web, app, and database tier behind the internet.",
        build: network_diagram,
    },
    Template {
        name: "ER Diagram",
        description: "Entities related with crow's-foot cardinality.",
        build: er_diagram,
    },
    Template {
        name: "SWOT Analysis",
        description: "Strengths, Weaknesses, Opportunities, and Threats, in a grid.",
        build: swot_analysis,
    },
];

fn new_doc(title: &str) -> Document {
    let mut doc = Document::new();
    doc.set_units(Units::Pt);
    doc.pages[0].title = title.to_string();
    doc
}

fn basic_flowchart() -> Document {
    let mut doc = new_doc("Flowchart");
    let blue = styled(Color::rgb(0xdb, 0xea, 0xfe), Color::rgb(0x25, 0x63, 0xeb));
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));
    let amber = styled(Color::rgb(0xfe, 0xf3, 0xc7), Color::rgb(0xca, 0x8a, 0x04));
    let green = styled(Color::rgb(0xd1, 0xfa, 0xe5), Color::rgb(0x05, 0x96, 0x69));
    let red = styled(Color::rgb(0xfe, 0xe2, 0xe2), Color::rgb(0xdc, 0x26, 0x26));

    let page = &mut doc.pages[0];
    page.add_shape(
        Shape::new("start", ShapeType::Terminator, BoundingBox::new(220.0, 40.0, 160.0, 54.0))
            .with_text("Start")
            .with_style(blue.clone()),
    );
    page.add_shape(
        Shape::new("step1", ShapeType::Process, BoundingBox::new(220.0, 140.0, 160.0, 56.0))
            .with_text("Do the work")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("decision", ShapeType::Decision, BoundingBox::new(210.0, 240.0, 180.0, 110.0))
            .with_text("Approved?")
            .with_style(amber),
    );
    page.add_shape(
        Shape::new("revise", ShapeType::Process, BoundingBox::new(450.0, 265.0, 160.0, 56.0))
            .with_text("Revise")
            .with_style(red),
    );
    page.add_shape(
        Shape::new("step2", ShapeType::Process, BoundingBox::new(220.0, 400.0, 160.0, 56.0))
            .with_text("Finish up")
            .with_style(plain),
    );
    page.add_shape(
        Shape::new("end", ShapeType::Terminator, BoundingBox::new(220.0, 500.0, 160.0, 54.0))
            .with_text("End")
            .with_style(green),
    );

    let edge = |id: &str, from: &str, to: &str| {
        Line::new(id, Endpoint::at_shape(from, EndpointStyle::None), Endpoint::at_shape(to, EndpointStyle::Arrow))
            .with_line_type(LineType::Elbow)
    };
    page.add_line(edge("e1", "start", "step1"));
    page.add_line(edge("e2", "step1", "decision"));
    page.add_line(edge("e3", "decision", "step2").with_label("yes"));
    page.add_line(edge("e4", "decision", "revise").with_label("no"));
    page.add_line(edge("e5", "revise", "decision"));
    page.add_line(edge("e6", "step2", "end"));

    doc
}

fn org_chart() -> Document {
    let mut doc = new_doc("Org Chart");
    let top = styled(Color::rgb(0xdb, 0xea, 0xfe), Color::rgb(0x25, 0x63, 0xeb));
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));

    let page = &mut doc.pages[0];
    page.add_shape(
        Shape::new("ceo", ShapeType::Rectangle, BoundingBox::new(330.0, 40.0, 160.0, 56.0))
            .with_text("CEO")
            .with_style(top),
    );
    for (id, x, text) in [
        ("vp_eng", 100.0, "VP Engineering"),
        ("vp_sales", 330.0, "VP Sales"),
        ("vp_ops", 560.0, "VP Operations"),
    ] {
        page.add_shape(
            Shape::new(id, ShapeType::Rectangle, BoundingBox::new(x, 170.0, 160.0, 56.0))
                .with_text(text)
                .with_style(plain.clone()),
        );
    }
    for (id, x, text) in [("eng_a", 20.0, "Eng Manager A"), ("eng_b", 190.0, "Eng Manager B")] {
        page.add_shape(
            Shape::new(id, ShapeType::Rectangle, BoundingBox::new(x, 300.0, 160.0, 56.0))
                .with_text(text)
                .with_style(plain.clone()),
        );
    }

    // Org charts connect with plain lines: reporting structure, not flow.
    let link = |id: &str, from: &str, to: &str| {
        Line::new(id, Endpoint::at_shape(from, EndpointStyle::None), Endpoint::at_shape(to, EndpointStyle::None))
            .with_line_type(LineType::Elbow)
    };
    page.add_line(link("e1", "ceo", "vp_eng"));
    page.add_line(link("e2", "ceo", "vp_sales"));
    page.add_line(link("e3", "ceo", "vp_ops"));
    page.add_line(link("e4", "vp_eng", "eng_a"));
    page.add_line(link("e5", "vp_eng", "eng_b"));

    doc
}

fn cross_functional_flowchart() -> Document {
    let mut doc = new_doc("Cross-Functional Flowchart");
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));
    let lane = Style { stroke: Some(Stroke::new(Color::rgb(0x6b, 0x72, 0x80), 1.5)), ..Default::default() };

    let page = &mut doc.pages[0];
    // Default three horizontal lanes, one row per team. The container is
    // left untitled: its own title would render centered in the same band
    // the row labels below use, and the two would overlap.
    page.add_shape(
        Shape::new("lanes", ShapeType::SwimLanes, BoundingBox::new(40.0, 40.0, 820.0, 360.0)).with_style(lane),
    );
    for (id, y, text) in [("role_sales", 40.0, "Sales"), ("role_wh", 160.0, "Warehouse"), ("role_ship", 280.0, "Shipping")]
    {
        page.add_shape(
            Shape::new(id, ShapeType::TextBlock, BoundingBox::new(88.0, y + 58.0, 90.0, 24.0)).with_text(text),
        );
    }
    page.add_shape(
        Shape::new("receive", ShapeType::Process, BoundingBox::new(200.0, 70.0, 160.0, 56.0))
            .with_text("Receive order")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("pick", ShapeType::Process, BoundingBox::new(430.0, 190.0, 160.0, 56.0))
            .with_text("Pick & pack")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("ship", ShapeType::Process, BoundingBox::new(650.0, 310.0, 160.0, 56.0))
            .with_text("Ship order")
            .with_style(plain),
    );

    let edge = |id: &str, from: &str, to: &str| {
        Line::new(id, Endpoint::at_shape(from, EndpointStyle::None), Endpoint::at_shape(to, EndpointStyle::Arrow))
            .with_line_type(LineType::Elbow)
    };
    page.add_line(edge("e1", "receive", "pick"));
    page.add_line(edge("e2", "pick", "ship"));

    doc
}

fn network_diagram() -> Document {
    let mut doc = new_doc("Network Diagram");
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));
    let sky = styled(Color::rgb(0xe0, 0xf2, 0xfe), Color::rgb(0x02, 0x84, 0xc7));

    let page = &mut doc.pages[0];
    page.add_shape(
        Shape::new("internet", ShapeType::Cloud, BoundingBox::new(260.0, 30.0, 140.0, 90.0))
            .with_text("Internet")
            .with_style(sky),
    );
    page.add_shape(
        Shape::new("web", ShapeType::Rectangle, BoundingBox::new(250.0, 170.0, 160.0, 56.0))
            .with_text("Web Server")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("app", ShapeType::Rectangle, BoundingBox::new(250.0, 280.0, 160.0, 56.0))
            .with_text("App Server")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("db", ShapeType::Database, BoundingBox::new(260.0, 390.0, 140.0, 70.0))
            .with_text("Database")
            .with_style(plain),
    );

    let edge = |id: &str, from: &str, to: &str| {
        Line::new(id, Endpoint::at_shape(from, EndpointStyle::None), Endpoint::at_shape(to, EndpointStyle::Arrow))
            .with_line_type(LineType::Straight)
    };
    page.add_line(edge("e1", "internet", "web"));
    page.add_line(edge("e2", "web", "app"));
    page.add_line(edge("e3", "app", "db"));

    doc
}

fn er_diagram() -> Document {
    let mut doc = new_doc("ER Diagram");
    let plain = styled(Color::WHITE, Color::rgb(0x6b, 0x72, 0x80));

    let page = &mut doc.pages[0];
    page.add_shape(
        Shape::new("customer", ShapeType::Rectangle, BoundingBox::new(40.0, 120.0, 160.0, 56.0))
            .with_text("Customer")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("order", ShapeType::Rectangle, BoundingBox::new(320.0, 120.0, 160.0, 56.0))
            .with_text("Order")
            .with_style(plain.clone()),
    );
    page.add_shape(
        Shape::new("product", ShapeType::Rectangle, BoundingBox::new(600.0, 120.0, 160.0, 56.0))
            .with_text("Product")
            .with_style(plain),
    );

    // Crow's-foot cardinality: one customer places many orders, and each
    // order references exactly one product.
    page.add_line(Line::new(
        "e1",
        Endpoint::at_shape("customer", EndpointStyle::ExactlyOne),
        Endpoint::at_shape("order", EndpointStyle::Many),
    ));
    page.add_line(Line::new(
        "e2",
        Endpoint::at_shape("order", EndpointStyle::Many),
        Endpoint::at_shape("product", EndpointStyle::ExactlyOne),
    ));

    doc
}

fn swot_analysis() -> Document {
    let mut doc = new_doc("SWOT Analysis");
    let green = styled(Color::rgb(0xd1, 0xfa, 0xe5), Color::rgb(0x05, 0x96, 0x69));
    let red = styled(Color::rgb(0xfe, 0xe2, 0xe2), Color::rgb(0xdc, 0x26, 0x26));
    let blue = styled(Color::rgb(0xdb, 0xea, 0xfe), Color::rgb(0x25, 0x63, 0xeb));
    let amber = styled(Color::rgb(0xfe, 0xf3, 0xc7), Color::rgb(0xca, 0x8a, 0x04));

    let page = &mut doc.pages[0];
    let (w, h, gap) = (280.0, 180.0, 16.0);
    page.add_shape(
        Shape::new("strengths", ShapeType::RectangleContainer, BoundingBox::new(40.0, 40.0, w, h))
            .with_text("Strengths")
            .with_style(green),
    );
    page.add_shape(
        Shape::new("weaknesses", ShapeType::RectangleContainer, BoundingBox::new(40.0 + w + gap, 40.0, w, h))
            .with_text("Weaknesses")
            .with_style(red),
    );
    page.add_shape(
        Shape::new("opportunities", ShapeType::RectangleContainer, BoundingBox::new(40.0, 40.0 + h + gap, w, h))
            .with_text("Opportunities")
            .with_style(blue),
    );
    page.add_shape(
        Shape::new("threats", ShapeType::RectangleContainer, BoundingBox::new(40.0 + w + gap, 40.0 + h + gap, w, h))
            .with_text("Threats")
            .with_style(amber),
    );

    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_lucid::LucidPackage;

    #[test]
    fn every_template_is_valid() {
        for t in ALL {
            let package = LucidPackage::new((t.build)());
            let issues = package.validate();
            assert_eq!(issues, vec![], "{} failed validation", t.name);
        }
    }

    #[test]
    fn every_connector_in_every_template_resolves() {
        for t in ALL {
            let doc = (t.build)();
            let page = &doc.pages[0];
            let routes = dc_geom::Router::new(page).route_all();
            assert_eq!(routes.len(), page.lines().len(), "a connector in {} failed to route", t.name);
        }
    }

    #[test]
    fn every_template_round_trips_through_the_package_format() {
        for t in ALL {
            let package = LucidPackage::new((t.build)());
            let bytes = package.to_bytes().unwrap();
            let back = LucidPackage::from_bytes(&bytes).unwrap();
            assert_eq!(back.document, package.document, "{} did not round-trip", t.name);
        }
    }

    #[test]
    fn every_template_has_a_name_and_description() {
        for t in ALL {
            assert!(!t.name.is_empty());
            assert!(!t.description.is_empty());
        }
    }

    #[test]
    fn template_names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|t| t.name).collect();
        let before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), before, "a template name is duplicated");
    }
}
