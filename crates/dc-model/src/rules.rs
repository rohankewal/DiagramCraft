//! Conditional formatting: colour driven by a shape's own data.
//!
//! A rule tests one custom-data field and, when it matches, says what the
//! shape should be coloured. This is an extension — the published
//! specification has no formatting rules — so it is written under a
//! namespaced key and a reader that does not know it carries it through
//! untouched, the same as any other field it does not interpret.
//!
//! Rules are *applied* rather than live: running them writes the colours
//! onto the shapes, which means every output path — canvas, SVG, PDF — shows
//! the same thing without any of them needing to know rules exist.

use crate::color::Color;
use crate::shape::Shape;
use serde::{Deserialize, Serialize};

/// How a rule compares a field against its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleOp {
    #[default]
    Equals,
    NotEquals,
    Contains,
    /// Numeric comparisons. A field that does not parse as a number never
    /// matches, rather than falling back to comparing text.
    GreaterThan,
    LessThan,
    IsEmpty,
    IsNotEmpty,
}

/// One conditional-formatting rule.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatRule {
    /// The custom-data key this rule reads.
    pub field: String,
    #[serde(default)]
    pub op: RuleOp,
    /// The value compared against. Ignored by `IsEmpty` and `IsNotEmpty`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<Color>,
}

impl FormatRule {
    /// A rule colouring the fill when a field equals a value.
    pub fn fill_when(field: impl Into<String>, equals: impl Into<String>, fill: Color) -> Self {
        FormatRule {
            field: field.into(),
            op: RuleOp::Equals,
            value: equals.into(),
            fill: Some(fill),
            ..Default::default()
        }
    }

    /// Whether this rule applies to a shape.
    ///
    /// Comparisons ignore case and surrounding space, because the values
    /// come from spreadsheets and from people typing into a panel.
    pub fn matches(&self, shape: &Shape) -> bool {
        let found = shape
            .custom_data
            .as_ref()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|e| e.key.trim().eq_ignore_ascii_case(self.field.trim()))
            })
            .map(|e| e.value.trim());

        match self.op {
            RuleOp::IsEmpty => found.is_none_or(|v| v.is_empty()),
            RuleOp::IsNotEmpty => found.is_some_and(|v| !v.is_empty()),
            _ => {
                let Some(actual) = found else {
                    return false;
                };
                let wanted = self.value.trim();
                match self.op {
                    RuleOp::Equals => actual.eq_ignore_ascii_case(wanted),
                    RuleOp::NotEquals => !actual.eq_ignore_ascii_case(wanted),
                    RuleOp::Contains => {
                        actual.to_lowercase().contains(&wanted.to_lowercase())
                    }
                    RuleOp::GreaterThan | RuleOp::LessThan => {
                        let (Ok(a), Ok(b)) =
                            (actual.parse::<f64>(), wanted.parse::<f64>())
                        else {
                            return false;
                        };
                        if self.op == RuleOp::GreaterThan { a > b } else { a < b }
                    }
                    RuleOp::IsEmpty | RuleOp::IsNotEmpty => unreachable!("handled above"),
                }
            }
        }
    }
}

/// A readable set of distinct colours for colouring by category.
///
/// Chosen to stay apart from one another and to take dark text legibly, so a
/// label stays readable on any of them.
pub const CATEGORY_COLORS: &[Color] = &[
    Color::rgb(0xdb, 0xea, 0xfe),
    Color::rgb(0xd1, 0xfa, 0xe5),
    Color::rgb(0xfe, 0xf3, 0xc7),
    Color::rgb(0xfe, 0xe2, 0xe2),
    Color::rgb(0xed, 0xe9, 0xfe),
    Color::rgb(0xcf, 0xfa, 0xfe),
    Color::rgb(0xfc, 0xe7, 0xf3),
    Color::rgb(0xec, 0xfc, 0xcb),
    Color::rgb(0xff, 0xed, 0xd5),
    Color::rgb(0xe5, 0xe7, 0xeb),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::BoundingBox;
    use crate::shape::{CustomDataEntry, ShapeType};

    fn shape_with(pairs: &[(&str, &str)]) -> Shape {
        let mut s = Shape::new("s", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        s.custom_data = Some(
            pairs
                .iter()
                .map(|(k, v)| CustomDataEntry { key: k.to_string(), value: v.to_string() })
                .collect(),
        );
        s
    }

    fn rule(field: &str, op: RuleOp, value: &str) -> FormatRule {
        FormatRule { field: field.into(), op, value: value.into(), ..Default::default() }
    }

    #[test]
    fn equality_ignores_case_and_surrounding_space() {
        let s = shape_with(&[("status", " Live ")]);
        assert!(rule("status", RuleOp::Equals, "live").matches(&s));
        assert!(rule("STATUS", RuleOp::Equals, "LIVE").matches(&s));
        assert!(!rule("status", RuleOp::Equals, "draft").matches(&s));
    }

    #[test]
    fn a_field_the_shape_does_not_have_never_matches() {
        let s = shape_with(&[("status", "live")]);
        assert!(!rule("owner", RuleOp::Equals, "ops").matches(&s));
        assert!(!rule("owner", RuleOp::Contains, "o").matches(&s));
    }

    #[test]
    fn not_equals_needs_the_field_to_be_present() {
        let s = shape_with(&[("status", "live")]);
        assert!(rule("status", RuleOp::NotEquals, "draft").matches(&s));
        assert!(
            !rule("absent", RuleOp::NotEquals, "draft").matches(&s),
            "a missing field is not a different value, it is no value"
        );
    }

    #[test]
    fn emptiness_is_asked_about_directly() {
        let present = shape_with(&[("note", "something")]);
        let blank = shape_with(&[("note", "   ")]);
        let missing = shape_with(&[("other", "x")]);

        assert!(rule("note", RuleOp::IsNotEmpty, "").matches(&present));
        assert!(!rule("note", RuleOp::IsEmpty, "").matches(&present));
        assert!(rule("note", RuleOp::IsEmpty, "").matches(&blank));
        assert!(rule("note", RuleOp::IsEmpty, "").matches(&missing));
    }

    #[test]
    fn numeric_comparisons_need_both_sides_to_be_numbers() {
        let s = shape_with(&[("count", "42")]);
        assert!(rule("count", RuleOp::GreaterThan, "10").matches(&s));
        assert!(!rule("count", RuleOp::GreaterThan, "100").matches(&s));
        assert!(rule("count", RuleOp::LessThan, "100").matches(&s));

        let text = shape_with(&[("count", "many")]);
        assert!(
            !rule("count", RuleOp::GreaterThan, "10").matches(&text),
            "text should not be compared as though it were a number"
        );
    }

    #[test]
    fn contains_is_a_substring_test() {
        let s = shape_with(&[("owner", "Platform Team")]);
        assert!(rule("owner", RuleOp::Contains, "platform").matches(&s));
        assert!(!rule("owner", RuleOp::Contains, "design").matches(&s));
    }

    #[test]
    fn a_shape_with_no_data_at_all_is_handled() {
        let bare = Shape::new("s", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        assert!(!rule("status", RuleOp::Equals, "live").matches(&bare));
        assert!(rule("status", RuleOp::IsEmpty, "").matches(&bare));
    }

    #[test]
    fn rules_round_trip_through_json() {
        let rule = FormatRule::fill_when("status", "blocked", Color::rgb(1, 2, 3));
        let text = serde_json::to_string(&rule).unwrap();
        assert_eq!(serde_json::from_str::<FormatRule>(&text).unwrap(), rule);
    }
}
