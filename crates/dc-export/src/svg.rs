//! The SVG writer.
//!
//! Paths are emitted as `<path>` with their geometry in document units, and
//! text as `<text>` so it stays selectable and searchable. Each run carries
//! a family stack rather than one name, so a viewer without the document's
//! font falls back the way the canvas does instead of picking something
//! arbitrary.

use crate::scene::{Drawing, FontFamily, Item, Paint};
use crate::Options;
use dc_model::Color;
use dc_render::text::PositionedRun;
use kurbo::{Affine, BezPath, PathEl};
use std::fmt::Write as _;

/// The flat grey an unembedded image block is drawn as.
const IMAGE_PLACEHOLDER: &str = "#e5e7eb";

/// Renders a drawing to an SVG document.
pub fn render(drawing: &Drawing, options: Options) -> String {
    let r = drawing.region;
    let mut out = String::with_capacity(4096);

    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        num(r.w),
        num(r.h),
        num(r.x),
        num(r.y),
        num(r.w),
        num(r.h),
    );
    out.push('\n');

    if !options.transparent {
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            num(r.x),
            num(r.y),
            num(r.w),
            num(r.h),
            hex(drawing.background),
        );
        out.push('\n');
    }

    let mut clip_id = 0usize;
    for item in &drawing.items {
        match item {
            Item::Path { path, paint, transform, alpha } => {
                write_path(&mut out, path, paint, *transform, *alpha);
            }
            Item::Text { run, transform, alpha, clip } => {
                clip_id += 1;
                write_text(&mut out, run, *transform, *alpha, *clip, clip_id);
            }
            Item::ImagePlaceholder { bounds, transform, alpha } => {
                let _ = write!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"{}{}/>"#,
                    num(bounds.x),
                    num(bounds.y),
                    num(bounds.w),
                    num(bounds.h),
                    IMAGE_PLACEHOLDER,
                    opacity_attr(*alpha),
                    transform_attr(*transform),
                );
                out.push('\n');
            }
        }
    }

    out.push_str("</svg>\n");
    out
}

fn write_path(out: &mut String, path: &BezPath, paint: &Paint, transform: Affine, alpha: f64) {
    let d = path_data(path);
    if d.is_empty() {
        return;
    }
    let _ = write!(out, r#"<path d="{d}""#);

    match paint.fill {
        Some(color) => {
            let _ = write!(out, r#" fill="{}""#, hex(color));
            if color.a != 255 {
                let _ = write!(out, r#" fill-opacity="{}""#, num(color.a as f64 / 255.0));
            }
        }
        None => out.push_str(r#" fill="none""#),
    }

    if let Some(stroke) = &paint.stroke {
        let _ = write!(
            out,
            r#" stroke="{}" stroke-width="{}""#,
            hex(stroke.color),
            num(stroke.width),
        );
        if stroke.color.a != 255 {
            let _ = write!(
                out,
                r#" stroke-opacity="{}""#,
                num(stroke.color.a as f64 / 255.0)
            );
        }
        if let Some(dashes) = &stroke.dashes {
            let pattern: Vec<String> = dashes.iter().map(|d| num(*d)).collect();
            let _ = write!(out, r#" stroke-dasharray="{}""#, pattern.join(" "));
        }
        // Matches the canvas, which strokes with round joins and caps.
        out.push_str(r#" stroke-linejoin="round" stroke-linecap="round""#);
    }

    let _ = write!(out, "{}{}", opacity_attr(alpha), transform_attr(transform));
    out.push_str("/>\n");
}

fn write_text(
    out: &mut String,
    run: &PositionedRun,
    transform: Affine,
    alpha: f64,
    clip: Option<dc_model::BoundingBox>,
    clip_id: usize,
) {
    // The clip is declared in the same space the text is drawn in, so a
    // rotated shape's clip turns with it.
    if let Some(bounds) = clip {
        let _ = write!(
            out,
            r#"<clipPath id="tc{}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath>"#,
            clip_id,
            num(bounds.x),
            num(bounds.y),
            num(bounds.w),
            num(bounds.h),
        );
        out.push('\n');
        let _ = write!(out, r#"<g clip-path="url(#tc{clip_id})">"#);
    }

    let _ = write!(
        out,
        r#"<text x="{}" y="{}" font-family="{}" font-size="{}" fill="{}""#,
        num(run.x),
        num(run.y),
        family_stack(&run.font_family),
        num(run.size_pt),
        hex(run.color),
    );
    if run.bold {
        out.push_str(r#" font-weight="bold""#);
    }
    if run.italic {
        out.push_str(r#" font-style="italic""#);
    }
    match (run.underline, run.strikethrough) {
        (true, true) => out.push_str(r#" text-decoration="underline line-through""#),
        (true, false) => out.push_str(r#" text-decoration="underline""#),
        (false, true) => out.push_str(r#" text-decoration="line-through""#),
        (false, false) => {}
    }
    if run.color.a != 255 {
        let _ = write!(out, r#" fill-opacity="{}""#, num(run.color.a as f64 / 255.0));
    }
    // Parley has already placed the run, so the viewer must not re-space it.
    out.push_str(r#" xml:space="preserve""#);
    let _ = write!(out, "{}{}", opacity_attr(alpha), transform_attr(transform));
    let _ = write!(out, ">{}</text>\n", escape(&run.text));
    if clip.is_some() {
        out.push_str("</g>\n");
    }
}

/// The family stack a run asks for, matching what the canvas falls back
/// through so a viewer without the named face lands in the same place.
fn family_stack(requested: &str) -> String {
    let generic = match FontFamily::classify(requested) {
        FontFamily::Sans => "sans-serif",
        FontFamily::Serif => "serif",
        FontFamily::Mono => "monospace",
    };
    let named = escape(requested);
    if requested.eq_ignore_ascii_case("Liberation Sans") {
        // The format's default, which is metrically compatible with these.
        return format!("{named}, Arial, Helvetica, {generic}");
    }
    format!("{named}, {generic}")
}

fn transform_attr(transform: Affine) -> String {
    if transform == Affine::IDENTITY {
        return String::new();
    }
    let c = transform.as_coeffs();
    format!(
        r#" transform="matrix({} {} {} {} {} {})""#,
        num(c[0]),
        num(c[1]),
        num(c[2]),
        num(c[3]),
        num(c[4]),
        num(c[5])
    )
}

fn opacity_attr(alpha: f64) -> String {
    if alpha >= 1.0 {
        String::new()
    } else {
        format!(r#" opacity="{}""#, num(alpha))
    }
}

/// Serializes a path to SVG path data.
///
/// Kurbo gives cubics for everything, so only `M`, `L`, `C` and `Z` are
/// needed; a quadratic is written as the cubic it already is.
pub fn path_data(path: &BezPath) -> String {
    let mut d = String::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                let _ = write!(d, "M{} {}", num(p.x), num(p.y));
            }
            PathEl::LineTo(p) => {
                let _ = write!(d, "L{} {}", num(p.x), num(p.y));
            }
            PathEl::QuadTo(c, p) => {
                let _ = write!(d, "Q{} {} {} {}", num(c.x), num(c.y), num(p.x), num(p.y));
            }
            PathEl::CurveTo(c1, c2, p) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    num(c1.x),
                    num(c1.y),
                    num(c2.x),
                    num(c2.y),
                    num(p.x),
                    num(p.y)
                );
            }
            PathEl::ClosePath => d.push('Z'),
        }
    }
    d
}

/// Formats a number without a trailing `.0`, keeping files small and diffs
/// readable.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

pub fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
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
    use kurbo::Point;

    #[test]
    fn numbers_drop_a_pointless_fraction() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(-3.0), "-3");
        assert_eq!(num(1.0 / 3.0), "0.333");
    }

    #[test]
    fn markup_characters_are_escaped() {
        assert_eq!(escape("a & b < c"), "a &amp; b &lt; c");
        assert_eq!(escape(r#"say "hi""#), "say &quot;hi&quot;");
    }

    #[test]
    fn a_path_becomes_path_data() {
        let mut p = BezPath::new();
        p.move_to(Point::new(0.0, 0.0));
        p.line_to(Point::new(10.0, 0.0));
        p.close_path();
        assert_eq!(path_data(&p), "M0 0L10 0Z");
    }

    #[test]
    fn an_identity_transform_is_not_written_out() {
        assert_eq!(transform_attr(Affine::IDENTITY), "");
        assert!(transform_attr(Affine::rotate(0.5)).contains("matrix("));
    }

    #[test]
    fn the_default_family_falls_back_through_its_metric_twins() {
        let stack = family_stack("Liberation Sans");
        assert!(stack.contains("Arial"), "{stack}");
        assert!(stack.ends_with("sans-serif"), "{stack}");
    }

    #[test]
    fn a_serif_request_ends_at_the_serif_generic() {
        assert!(family_stack("Times New Roman").ends_with("serif"));
        assert!(family_stack("Courier New").ends_with("monospace"));
    }
}
