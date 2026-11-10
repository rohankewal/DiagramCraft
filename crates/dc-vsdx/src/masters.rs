//! Mapping Visio master names onto shape types.
//!
//! A `.vsdx` shape names the stencil master it was stamped from rather than a
//! geometric type, so importing means recognizing those names. The names below
//! are the universal names (`NameU`) Visio writes for its stock Basic Shapes
//! and Basic Flowchart stencils; they are stable across localizations, which is
//! why the universal name is matched rather than the displayed one.
//!
//! A master that is not recognized still imports: its shape keeps its
//! geometry, text, and style and draws as a rectangle, and its master name is
//! recorded on the shape so nothing about it is lost.

use dc_model::ShapeType;

/// The shape type a master name corresponds to, if one does.
pub fn shape_type_for(master_name: &str) -> Option<ShapeType> {
    let key = normalize(master_name);
    MAPPINGS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, t)| t.clone())
}

/// Whether a master is one of the connector masters, which import as lines
/// rather than as shapes.
pub fn is_connector(master_name: &str) -> bool {
    let key = normalize(master_name);
    key.contains("connector") && !key.contains("page")
        || key == "line"
        || key == "dynamicconnector"
}

/// Lowercases and strips the punctuation and spacing that varies between
/// stencil versions, so "Off-page reference" and "Off page Reference" agree.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Master name to shape type, keyed by the normalized name.
static MAPPINGS: &[(&str, ShapeType)] = &[
    // Basic Flowchart stencil.
    ("process", ShapeType::Process),
    ("decision", ShapeType::Decision),
    ("startend", ShapeType::Terminator),
    ("terminator", ShapeType::Terminator),
    ("document", ShapeType::Document),
    ("multidocument", ShapeType::MultipleDocuments),
    ("multipledocuments", ShapeType::MultipleDocuments),
    ("data", ShapeType::Data),
    ("predefinedprocess", ShapeType::PredefinedProcess),
    ("subprocess", ShapeType::PredefinedProcess),
    ("internalstorage", ShapeType::InternalStorage),
    ("manualinput", ShapeType::ManualInput),
    ("manualoperation", ShapeType::ManualOperation),
    ("preparation", ShapeType::Preparation),
    ("directdata", ShapeType::DirectAccessStorage),
    ("storeddata", ShapeType::StoredData),
    ("database", ShapeType::Database),
    ("display", ShapeType::Display),
    ("delay", ShapeType::Delay),
    ("offpagereference", ShapeType::OffPageLink),
    ("onpagereference", ShapeType::Connector),
    ("merge", ShapeType::Merge),
    ("or", ShapeType::Or),
    ("summingjunction", ShapeType::SummingJunction),
    ("sequentialdata", ShapeType::PaperTape),
    ("papertape", ShapeType::PaperTape),
    ("extract", ShapeType::Merge),
    ("annotation", ShapeType::BraceNote),
    ("card", ShapeType::Rectangle),
    ("externaldata", ShapeType::Data),
    ("autooperation", ShapeType::Process),

    // Basic Shapes stencil.
    ("rectangle", ShapeType::Rectangle),
    ("square", ShapeType::Rectangle),
    ("roundedrectangle", ShapeType::Rectangle),
    ("circle", ShapeType::Circle),
    ("ellipse", ShapeType::Circle),
    ("diamond", ShapeType::Diamond),
    ("triangle", ShapeType::IsoscelesTriangle),
    ("isoscelestriangle", ShapeType::IsoscelesTriangle),
    ("righttriangle", ShapeType::RightTriangle),
    ("pentagon", ShapeType::Pentagon),
    ("hexagon", ShapeType::Hexagon),
    ("heptagon", ShapeType::FlexiblePolygon),
    ("octagon", ShapeType::Octagon),
    ("cross", ShapeType::Cross),
    ("star4", ShapeType::PolyStar),
    ("star5", ShapeType::PolyStar),
    ("star6", ShapeType::PolyStar),
    ("star7", ShapeType::PolyStar),
    ("arrow", ShapeType::SingleArrow),
    ("simplearrow", ShapeType::SingleArrow),
    ("doublearrow", ShapeType::DoubleArrow),
    ("cloud", ShapeType::Cloud),
    ("stickynote", ShapeType::StickyNote),
    ("note", ShapeType::StickyNote),
    ("textbox", ShapeType::TextBlock),
    ("text", ShapeType::TextBlock),
    ("picture", ShapeType::Image),

    // Container stencil.
    ("plaincontainer", ShapeType::RectangleContainer),
    ("container", ShapeType::RectangleContainer),
    ("roundedcontainer", ShapeType::RoundedRectangleContainer),
    ("bracketcontainer", ShapeType::BracketContainer),
    ("bracecontainer", ShapeType::BraceContainer),
    ("swimlane", ShapeType::SwimLanes),
    ("swimlanelist", ShapeType::SwimLanes),

    // BPMN stencil.
    ("task", ShapeType::BpmnActivity),
    ("bpmntask", ShapeType::BpmnActivity),
    ("subprocessbpmn", ShapeType::BpmnActivity),
    ("gateway", ShapeType::BpmnGateway),
    ("event", ShapeType::BpmnEvent),
    ("startevent", ShapeType::BpmnEvent),
    ("intermediateevent", ShapeType::BpmnEvent),
    ("endevent", ShapeType::BpmnEvent),
    ("dataobject", ShapeType::BpmnDataObject),
    ("datastore", ShapeType::BpmnDataStore),
    ("pool", ShapeType::BpmnPool),
    ("lane", ShapeType::BpmnPool),
    ("textannotation", ShapeType::BpmnTextAnnotation),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_flowchart_masters_are_recognized() {
        assert_eq!(shape_type_for("Process"), Some(ShapeType::Process));
        assert_eq!(shape_type_for("Decision"), Some(ShapeType::Decision));
        assert_eq!(shape_type_for("Start/End"), Some(ShapeType::Terminator));
        assert_eq!(shape_type_for("Document"), Some(ShapeType::Document));
        assert_eq!(
            shape_type_for("Predefined process"),
            Some(ShapeType::PredefinedProcess)
        );
        assert_eq!(shape_type_for("Off-page reference"), Some(ShapeType::OffPageLink));
    }

    #[test]
    fn basic_shape_masters_are_recognized() {
        assert_eq!(shape_type_for("Circle"), Some(ShapeType::Circle));
        assert_eq!(shape_type_for("Hexagon"), Some(ShapeType::Hexagon));
        assert_eq!(shape_type_for("Star 5"), Some(ShapeType::PolyStar));
        assert_eq!(shape_type_for("Simple Arrow"), Some(ShapeType::SingleArrow));
    }

    #[test]
    fn matching_ignores_case_spacing_and_punctuation() {
        for spelling in [
            "Off-page reference",
            "off page reference",
            "OffPageReference",
            "  Off-Page   Reference  ",
        ] {
            assert_eq!(
                shape_type_for(spelling),
                Some(ShapeType::OffPageLink),
                "{spelling} should match"
            );
        }
    }

    #[test]
    fn an_unknown_master_has_no_mapping() {
        assert_eq!(shape_type_for("Rack Unit 42U"), None);
        assert_eq!(shape_type_for(""), None);
    }

    #[test]
    fn connector_masters_are_recognized_as_lines() {
        assert!(is_connector("Dynamic connector"));
        assert!(is_connector("Line-curve connector"));
        assert!(is_connector("Connector"));
        assert!(!is_connector("Process"));
        assert!(
            !is_connector("On-page reference"),
            "a page reference is a shape, not a line"
        );
    }

    #[test]
    fn every_mapping_names_a_type_this_build_draws() {
        for (name, shape_type) in MAPPINGS {
            assert!(
                shape_type.is_known(),
                "{name} maps to an unimplemented type"
            );
        }
    }

    #[test]
    fn mapping_keys_are_already_normalized() {
        for (name, _) in MAPPINGS {
            assert_eq!(&normalize(name), name, "{name} would never be matched");
        }
    }
}
