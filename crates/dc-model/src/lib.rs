//! The DiagramCraft document model.
//!
//! These types mirror the Lucid Standard Import interchange format, which Lucid
//! publishes at <https://developer.lucid.co/docs/overview-si>. They were written
//! from that published specification; see `docs/FORMAT-PROVENANCE.md` for the
//! full source list and methodology.
//!
//! # Fidelity
//!
//! Loading and saving a document preserves it *semantically*: every field the
//! input carried is present in the output with the same value, including fields
//! this build does not interpret. It is deliberately not byte-exact, because
//! JSON number formatting and key order are not meaningful in the format.
//!
//! Unrecognized values in open enumerations (shape types, endpoint styles) are
//! retained verbatim rather than rejected, so a document built from a shape
//! library this version lacks is never silently degraded.

pub mod color;
pub mod document;
pub mod geom;
pub mod line;
pub mod markup;
pub mod rules;
pub mod shape;
pub mod style;
mod string_enum;

pub use color::Color;
pub use document::{
    Comment,
    DataLink,
    Guide,
    Collection, Document, DocumentSettings, ExtensionBootstrapData, FORMAT_VERSION, Group, Layer,
    Page, PageSettings, PageSize, Units,
};
pub use geom::{BoundingBox, Point, RelPoint};
pub use markup::{ParsedText, TextRun};
pub use line::{Endpoint, EndpointStyle, LabelSide, Line, LineLabel, LineType};
pub use rules::{CATEGORY_COLORS, FormatRule, RuleOp};
pub use shape::{Action, CloudVendor, CustomDataEntry, ImageRef, Shape, ShapeLibrary, ShapeType, StarShape};
pub use style::{
    Fill, ImageScale, Stroke, StrokeStyle, Style, Text, TextAlign, TextStyle, VerticalAlign,
};

/// Whether an ID satisfies the format's constraint: 1 to 36 characters drawn
/// from alphanumerics and `-`, `_`, `.`, `~`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 36
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
}

/// Rewrites a string into a valid ID by replacing disallowed characters with
/// `-` and truncating to the length limit.
pub fn sanitize_id(input: &str) -> String {
    let mut out: String = input
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') { c } else { '-' })
        .take(36)
        .collect();
    if out.is_empty() {
        out.push('x');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_rules_match_spec() {
        assert!(is_valid_id("block-1"));
        assert!(is_valid_id("a.b~c_d"));
        assert!(is_valid_id(&"a".repeat(36)));

        assert!(!is_valid_id(""), "ids need at least one character");
        assert!(!is_valid_id(&"a".repeat(37)), "ids cap at 36 characters");
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("emoji✨"));
    }

    #[test]
    fn sanitize_produces_valid_ids() {
        assert_eq!(sanitize_id("has space"), "has-space");
        assert_eq!(sanitize_id(""), "x");
        assert!(is_valid_id(&sanitize_id(&"x".repeat(100))));
        assert!(is_valid_id(&sanitize_id("✨✨✨")));
    }
}
