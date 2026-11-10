//! Package validation against the published constraints.
//!
//! Validation reports rather than refuses. A document that violates a
//! constraint still loads, because an editor that drops a user's work on a
//! technicality is worse than one that flags it.

use crate::{LucidPackage, MAX_DOCUMENT_BYTES};
use dc_model::{Endpoint, Fill};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Violates a published requirement; the document may not import elsewhere.
    Error,
    /// Legal but suspect, such as a connector pointing at nothing.
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: Severity,
    /// Where the problem is, as a document path like `pages[0].shapes[2]`.
    pub location: String,
    pub message: String,
}

impl ValidationIssue {
    fn error(location: impl Into<String>, message: impl Into<String>) -> Self {
        ValidationIssue {
            severity: Severity::Error,
            location: location.into(),
            message: message.into(),
        }
    }

    fn warning(location: impl Into<String>, message: impl Into<String>) -> Self {
        ValidationIssue {
            severity: Severity::Warning,
            location: location.into(),
            message: message.into(),
        }
    }
}

/// Checks a package against the format's published constraints.
pub fn validate(pkg: &LucidPackage) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let doc = &pkg.document;

    if doc.pages.is_empty() {
        issues.push(ValidationIssue::error("pages", "a document needs at least one page"));
    }

    if let Ok(json) = serde_json::to_vec(doc) {
        if json.len() > MAX_DOCUMENT_BYTES {
            issues.push(ValidationIssue::error(
                "document.json",
                format!("{} bytes exceeds the {MAX_DOCUMENT_BYTES} byte limit", json.len()),
            ));
        }
    }

    // IDs must be unique across the whole document, not just within a page,
    // because connectors reference them without page qualification.
    let mut seen_ids: HashSet<&str> = HashSet::new();

    for (pi, page) in doc.pages.iter().enumerate() {
        let page_loc = format!("pages[{pi}]");

        if !dc_model::is_valid_id(&page.id) {
            issues.push(ValidationIssue::error(
                &page_loc,
                format!("page id {:?} is not 1-36 characters of [A-Za-z0-9-_.~]", page.id),
            ));
        }
        if !seen_ids.insert(&page.id) {
            issues.push(ValidationIssue::error(
                &page_loc,
                format!("duplicate id {:?}", page.id),
            ));
        }

        let shape_ids: HashSet<&str> = page.shapes().iter().map(|s| s.id.as_str()).collect();
        let line_ids: HashSet<&str> = page.lines().iter().map(|l| l.id.as_str()).collect();

        for (si, shape) in page.shapes().iter().enumerate() {
            let loc = format!("{page_loc}.shapes[{si}]");

            if !dc_model::is_valid_id(&shape.id) {
                issues.push(ValidationIssue::error(
                    &loc,
                    format!("shape id {:?} is not 1-36 characters of [A-Za-z0-9-_.~]", shape.id),
                ));
            }
            if !seen_ids.insert(&shape.id) {
                issues.push(ValidationIssue::error(&loc, format!("duplicate id {:?}", shape.id)));
            }

            if shape.bounding_box.w < 0.0 || shape.bounding_box.h < 0.0 {
                issues.push(ValidationIssue::error(&loc, "bounding box has negative size"));
            }

            if shape.text.is_some() && !shape.shape_type.accepts_text() {
                issues.push(ValidationIssue::warning(
                    &loc,
                    format!("shape type {:?} does not accept text", shape.shape_type.as_str()),
                ));
            }
            if shape.style.is_some() && !shape.shape_type.accepts_style() {
                issues.push(ValidationIssue::warning(
                    &loc,
                    format!("shape type {:?} does not accept a style", shape.shape_type.as_str()),
                ));
            }

            if let Some(o) = shape.opacity {
                if !(0.0..=100.0).contains(&o) {
                    issues.push(ValidationIssue::warning(
                        &loc,
                        format!("opacity {o} is outside the 0-100 range"),
                    ));
                }
            }

            if let Some(sw) = shape.side_width {
                if !(0.0..=0.33).contains(&sw) {
                    issues.push(ValidationIssue::warning(
                        &loc,
                        format!("sideWidth {sw} is outside the 0-0.33 range"),
                    ));
                }
            }

            // An image fill that names a missing asset renders as nothing, so
            // it is worth catching before the user wonders why.
            if let Some(Fill::Image { r#ref: Some(name), .. }) =
                shape.style.as_ref().and_then(|s| s.fill.as_ref())
            {
                if !pkg.images.contains_key(name) {
                    issues.push(ValidationIssue::warning(
                        &loc,
                        format!("image fill references {name:?}, which is not in images/"),
                    ));
                }
            }
            if let Some(name) = shape.image.as_ref().and_then(|i| i.r#ref.as_ref()) {
                if !pkg.images.contains_key(name) {
                    issues.push(ValidationIssue::warning(
                        &loc,
                        format!("image references {name:?}, which is not in images/"),
                    ));
                }
            }
        }

        for (li, line) in page.lines().iter().enumerate() {
            let loc = format!("{page_loc}.lines[{li}]");

            if !dc_model::is_valid_id(&line.id) {
                issues.push(ValidationIssue::error(
                    &loc,
                    format!("line id {:?} is not 1-36 characters of [A-Za-z0-9-_.~]", line.id),
                ));
            }
            if !seen_ids.insert(&line.id) {
                issues.push(ValidationIssue::error(&loc, format!("duplicate id {:?}", line.id)));
            }

            for (ei, endpoint) in [&line.endpoint1, &line.endpoint2].iter().enumerate() {
                let eloc = format!("{loc}.endpoint{}", ei + 1);
                match endpoint {
                    Endpoint::ShapeEndpoint { shape_id, .. } => {
                        if !shape_ids.contains(shape_id.as_str()) {
                            issues.push(ValidationIssue::error(
                                &eloc,
                                format!("attaches to shape {shape_id:?}, which is not on this page"),
                            ));
                        }
                    }
                    Endpoint::LineEndpoint { line_id, position, .. } => {
                        if !line_ids.contains(line_id.as_str()) {
                            issues.push(ValidationIssue::error(
                                &eloc,
                                format!("attaches to line {line_id:?}, which is not on this page"),
                            ));
                        }
                        if line_id == &line.id {
                            issues.push(ValidationIssue::error(&eloc, "line attaches to itself"));
                        }
                        if !(0.0..=1.0).contains(position) {
                            issues.push(ValidationIssue::warning(
                                &eloc,
                                format!("position {position} is outside the 0.0-1.0 range"),
                            ));
                        }
                    }
                    Endpoint::PositionEndpoint { .. } => {}
                }
            }

            for (bi, label) in line.labels().iter().enumerate() {
                if let Some(p) = label.position {
                    if !(0.0..=1.0).contains(&p) {
                        issues.push(ValidationIssue::warning(
                            format!("{loc}.text[{bi}]"),
                            format!("label position {p} is outside the 0.0-1.0 range"),
                        ));
                    }
                }
            }
        }

        for (gi, group) in page.groups().iter().enumerate() {
            let loc = format!("{page_loc}.groups[{gi}]");
            if !seen_ids.insert(&group.id) {
                issues.push(ValidationIssue::error(&loc, format!("duplicate id {:?}", group.id)));
            }
            for item in group.item_ids() {
                if !shape_ids.contains(item.as_str()) && !line_ids.contains(item.as_str()) {
                    issues.push(ValidationIssue::warning(
                        &loc,
                        format!("group member {item:?} is not on this page"),
                    ));
                }
            }
        }
    }

    // A collection pointing at a missing CSV silently yields no data.
    for (ci, collection) in doc.collections.as_deref().unwrap_or(&[]).iter().enumerate() {
        if let Some(src) = &collection.data_source {
            if !pkg.data.contains_key(src) {
                issues.push(ValidationIssue::warning(
                    format!("collections[{ci}]"),
                    format!("dataSource {src:?} is not in data/"),
                ));
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{
        BoundingBox, Document, Endpoint, EndpointStyle, Fill, Line, Page, Shape, ShapeType, Style,
    };

    fn pkg_with(page: Page) -> LucidPackage {
        let mut doc = Document::new();
        doc.pages = vec![page];
        LucidPackage::new(doc)
    }

    fn rect(id: &str) -> Shape {
        Shape::new(id, ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0))
    }

    #[test]
    fn a_clean_document_has_no_issues() {
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(rect("a"));
        page.add_shape(rect("b"));
        page.add_line(Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("b", EndpointStyle::Arrow),
        ));
        assert_eq!(pkg_with(page).validate(), vec![]);
    }

    #[test]
    fn empty_document_is_an_error() {
        let mut doc = Document::new();
        doc.pages.clear();
        let issues = LucidPackage::new(doc).validate();
        assert!(issues.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("at least one page")));
    }

    #[test]
    fn duplicate_ids_are_errors() {
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(rect("dup"));
        page.add_shape(rect("dup"));
        let issues = pkg_with(page).validate();
        assert!(
            issues.iter().any(|i| i.severity == Severity::Error && i.message.contains("duplicate")),
            "{issues:?}"
        );
    }

    #[test]
    fn invalid_id_characters_are_errors() {
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(rect("has space"));
        let issues = pkg_with(page).validate();
        assert!(issues.iter().any(|i| i.message.contains("1-36 characters")), "{issues:?}");
    }

    #[test]
    fn connector_to_a_missing_shape_is_an_error() {
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(rect("a"));
        page.add_line(Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("ghost", EndpointStyle::Arrow),
        ));
        let issues = pkg_with(page).validate();
        assert!(
            issues.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("\"ghost\"")
                && i.location.ends_with("endpoint2")),
            "{issues:?}"
        );
    }

    #[test]
    fn self_referencing_line_is_an_error() {
        let mut page = Page::new("p1", "Page 1");
        page.add_shape(rect("a"));
        let mut line = Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("a", EndpointStyle::None),
        );
        line.endpoint2 = Endpoint::LineEndpoint {
            style: EndpointStyle::None,
            line_id: "l1".into(),
            position: 0.5,
        };
        page.add_line(line);
        let issues = pkg_with(page).validate();
        assert!(issues.iter().any(|i| i.message.contains("attaches to itself")), "{issues:?}");
    }

    #[test]
    fn missing_image_asset_is_a_warning() {
        let mut page = Page::new("p1", "Page 1");
        let mut s = rect("a");
        s.style = Some(Style {
            fill: Some(Fill::Image {
                r#ref: Some("absent.png".into()),
                url: None,
                image_scale: None,
            }),
            ..Default::default()
        });
        page.add_shape(s);

        let issues = pkg_with(page).validate();
        assert!(
            issues.iter().any(|i| i.severity == Severity::Warning
                && i.message.contains("absent.png")),
            "{issues:?}"
        );
    }

    #[test]
    fn present_image_asset_is_accepted() {
        let mut page = Page::new("p1", "Page 1");
        let mut s = rect("a");
        s.style = Some(Style {
            fill: Some(Fill::Image {
                r#ref: Some("logo.png".into()),
                url: None,
                image_scale: None,
            }),
            ..Default::default()
        });
        page.add_shape(s);

        let mut pkg = pkg_with(page);
        pkg.images.insert("logo.png".into(), vec![1]);
        assert_eq!(pkg.validate(), vec![]);
    }

    #[test]
    fn text_on_a_type_that_forbids_it_is_a_warning() {
        let mut page = Page::new("p1", "Page 1");
        let mut s = Shape::new("h", ShapeType::Hotspot, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        s.text = Some(dc_model::Text::new("nope"));
        page.add_shape(s);

        let issues = pkg_with(page).validate();
        assert!(
            issues.iter().any(|i| i.message.contains("does not accept text")),
            "{issues:?}"
        );
    }

    #[test]
    fn out_of_range_values_are_warnings() {
        let mut page = Page::new("p1", "Page 1");
        let mut s = rect("a");
        s.opacity = Some(140.0);
        page.add_shape(s);

        let issues = pkg_with(page).validate();
        assert!(issues.iter().any(|i| i.message.contains("0-100")), "{issues:?}");
    }

    #[test]
    fn missing_data_source_is_a_warning() {
        let mut doc = Document::new();
        doc.collections = Some(vec![dc_model::Collection {
            id: "c1".into(),
            data_source: Some("records.csv".into()),
            extra: Default::default(),
        }]);
        let issues = LucidPackage::new(doc).validate();
        assert!(issues.iter().any(|i| i.message.contains("records.csv")), "{issues:?}");
    }
}
