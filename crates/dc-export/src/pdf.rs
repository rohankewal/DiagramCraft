//! The PDF writer.
//!
//! A minimal PDF 1.7 file assembled by hand: a catalogue, one page, one
//! content stream, and a font resource per face actually used. Nothing is
//! embedded, because nothing needs to be — the three families the editor
//! offers map onto the standard fonts every conforming reader is required
//! to provide, so text stays real, selectable and searchable without
//! carrying a font program along.
//!
//! PDF measures from the bottom-left corner with its y axis pointing up,
//! while documents here measure from the top-left with y pointing down. The
//! page therefore opens with a flip, and each piece of text counter-flips
//! through its own text matrix so the glyphs come out upright.
//!
//! ## Text encoding
//!
//! Standard fonts are addressed through `WinAnsiEncoding`, which reaches
//! Latin-1 and little else. A character outside it is written as `?` rather
//! than silently dropped. Exporting CJK, or emoji, as *text* would mean
//! embedding a font with a CID mapping, which this writer does not do.

use crate::scene::{Drawing, FontFamily, Item, Paint};
use crate::Options;
use dc_model::Color;
use dc_render::text::PositionedRun;
use kurbo::{Affine, BezPath, PathEl};
use std::fmt::Write as _;

/// A standard font, named as PDF names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Face {
    family: FamilyKey,
    bold: bool,
    italic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FamilyKey {
    Helvetica,
    Times,
    Courier,
}

impl Face {
    fn of(run: &PositionedRun) -> Face {
        let family = match FontFamily::classify(&run.font_family) {
            FontFamily::Sans => FamilyKey::Helvetica,
            FontFamily::Serif => FamilyKey::Times,
            FontFamily::Mono => FamilyKey::Courier,
        };
        Face { family, bold: run.bold, italic: run.italic }
    }

    /// The `BaseFont` name, from the set of standard fonts.
    ///
    /// Times names its upright face `Times-Roman` and its slanted one
    /// `Times-Italic`, where the other two families use the base name and
    /// `Oblique`; the irregularity is the specification's, not ours.
    pub fn base_name(&self) -> &'static str {
        match (self.family, self.bold, self.italic) {
            (FamilyKey::Helvetica, false, false) => "Helvetica",
            (FamilyKey::Helvetica, true, false) => "Helvetica-Bold",
            (FamilyKey::Helvetica, false, true) => "Helvetica-Oblique",
            (FamilyKey::Helvetica, true, true) => "Helvetica-BoldOblique",
            (FamilyKey::Times, false, false) => "Times-Roman",
            (FamilyKey::Times, true, false) => "Times-Bold",
            (FamilyKey::Times, false, true) => "Times-Italic",
            (FamilyKey::Times, true, true) => "Times-BoldItalic",
            (FamilyKey::Courier, false, false) => "Courier",
            (FamilyKey::Courier, true, false) => "Courier-Bold",
            (FamilyKey::Courier, false, true) => "Courier-Oblique",
            (FamilyKey::Courier, true, true) => "Courier-BoldOblique",
        }
    }
}

/// Renders a drawing to a PDF file's bytes.
pub fn render(drawing: &Drawing, options: Options) -> Vec<u8> {
    let faces = faces_used(drawing);
    let alphas = alphas_used(drawing);
    let content = content_stream(drawing, options, &faces, &alphas);
    assemble(drawing, &content, &faces, &alphas)
}

/// Every standard face the drawing calls for, in a stable order.
fn faces_used(drawing: &Drawing) -> Vec<Face> {
    let mut set: Vec<Face> = drawing
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { run, .. } => Some(Face::of(run)),
            _ => None,
        })
        .collect();
    set.sort();
    set.dedup();
    set
}

/// Every distinct transparency level, as thousandths so it can be a key.
fn alphas_used(drawing: &Drawing) -> Vec<u32> {
    let mut set: Vec<u32> = drawing
        .items
        .iter()
        .map(|item| {
            let a = match item {
                Item::Path { alpha, paint, .. } => {
                    let c = paint
                        .fill
                        .map(|f| f.a)
                        .or_else(|| paint.stroke.as_ref().map(|s| s.color.a))
                        .unwrap_or(255);
                    alpha * (c as f64 / 255.0)
                }
                Item::Text { alpha, run, .. } => alpha * (run.color.a as f64 / 255.0),
                Item::ImagePlaceholder { alpha, .. } => *alpha,
            };
            (a.clamp(0.0, 1.0) * 1000.0).round() as u32
        })
        .filter(|a| *a < 1000)
        .collect();
    set.sort();
    set.dedup();
    set
}

/// Builds the page's content stream.
///
/// Exposed so a test can read the operators without unpacking a file.
pub fn content_stream(
    drawing: &Drawing,
    options: Options,
    faces: &[Face],
    alphas: &[u32],
) -> String {
    let r = drawing.region;
    let mut out = String::with_capacity(4096);

    if !options.transparent {
        let _ = writeln!(out, "q {} rg 0 0 {} {} re f Q", rgb(drawing.background), n(r.w), n(r.h));
    }

    // From here on everything is written in document coordinates: the flip
    // turns the page the right way up and shifts the framed region to the
    // origin.
    let _ = writeln!(out, "q 1 0 0 -1 {} {} cm", n(-r.x), n(r.h + r.y));

    for item in &drawing.items {
        match item {
            Item::Path { path, paint, transform, alpha } => {
                write_path(&mut out, path, paint, *transform, *alpha, alphas);
            }
            Item::Text { run, transform, alpha, clip } => {
                write_text(&mut out, run, *transform, *alpha, *clip, faces, alphas);
            }
            Item::ImagePlaceholder { bounds, transform, alpha } => {
                out.push_str("q ");
                push_gs(&mut out, *alpha, alphas);
                push_cm(&mut out, *transform);
                let _ = writeln!(
                    out,
                    "0.898 0.906 0.922 rg {} {} {} {} re f Q",
                    n(bounds.x),
                    n(bounds.y),
                    n(bounds.w),
                    n(bounds.h)
                );
            }
        }
    }

    out.push_str("Q\n");
    out
}

fn write_path(
    out: &mut String,
    path: &BezPath,
    paint: &Paint,
    transform: Affine,
    alpha: f64,
    alphas: &[u32],
) {
    let geometry = path_ops(path);
    if geometry.is_empty() {
        return;
    }

    out.push_str("q ");
    let effective = paint
        .fill
        .map(|f| f.a)
        .or_else(|| paint.stroke.as_ref().map(|s| s.color.a))
        .unwrap_or(255);
    push_gs(out, alpha * (effective as f64 / 255.0), alphas);
    push_cm(out, transform);

    if let Some(fill) = paint.fill {
        let _ = write!(out, "{} rg ", rgb(fill));
    }
    if let Some(stroke) = &paint.stroke {
        let _ = write!(out, "{} RG {} w ", rgb(stroke.color), n(stroke.width));
        match &stroke.dashes {
            Some(d) => {
                let pattern: Vec<String> = d.iter().map(|v| n(*v)).collect();
                let _ = write!(out, "[{}] 0 d ", pattern.join(" "));
            }
            None => out.push_str("[] 0 d "),
        }
        // Round joins and caps, matching how the canvas strokes.
        out.push_str("1 J 1 j ");
    }

    out.push_str(&geometry);
    out.push_str(match (paint.fill.is_some(), paint.stroke.is_some()) {
        (true, true) => "B",
        (true, false) => "f",
        (false, true) => "S",
        (false, false) => "n",
    });
    out.push_str(" Q\n");
}

#[allow(clippy::too_many_arguments)]
fn write_text(
    out: &mut String,
    run: &PositionedRun,
    transform: Affine,
    alpha: f64,
    clip: Option<dc_model::BoundingBox>,
    faces: &[Face],
    alphas: &[u32],
) {
    let face = Face::of(run);
    let Some(index) = faces.iter().position(|f| *f == face) else {
        return;
    };

    out.push_str("q ");
    push_gs(out, alpha * (run.color.a as f64 / 255.0), alphas);
    push_cm(out, transform);
    // `W n` takes the rectangle just built as the clip and paints nothing,
    // confining everything up to the matching Q.
    push_clip(out, clip);

    // The page is flipped, so the text matrix flips back: without the -1 the
    // glyphs would be drawn mirrored about their own baseline.
    let _ = write!(
        out,
        "BT /F{} {} Tf {} rg 1 0 0 -1 {} {} Tm {} Tj ET",
        index + 1,
        n(run.size_pt),
        rgb(run.color),
        n(run.x),
        n(run.y),
        pdf_string(&run.text)
    );
    out.push_str(" Q\n");

    // Underline and strikethrough are rules, not glyphs, in PDF as on canvas.
    let rule = |out: &mut String, y: f64| {
        let _ = writeln!(
            out,
            "q {} RG {} w 1 J {} {} m {} {} l S Q",
            rgb(run.color),
            n((run.size_pt * 0.06).max(0.4)),
            n(run.x),
            n(y),
            n(run.x + run.advance),
            n(y)
        );
    };
    if run.underline || run.strikethrough {
        out.push_str("q ");
        push_cm(out, transform);
        push_clip(out, clip);
        if run.underline {
            rule(out, run.y + run.size_pt * 0.12);
        }
        if run.strikethrough {
            rule(out, run.y - run.size_pt * 0.28);
        }
        out.push_str("Q\n");
    }
}

fn push_clip(out: &mut String, clip: Option<dc_model::BoundingBox>) {
    let Some(b) = clip else {
        return;
    };
    let _ = write!(out, "{} {} {} {} re W n ", n(b.x), n(b.y), n(b.w), n(b.h));
}

fn push_cm(out: &mut String, transform: Affine) {
    if transform == Affine::IDENTITY {
        return;
    }
    let c = transform.as_coeffs();
    let _ = write!(
        out,
        "{} {} {} {} {} {} cm ",
        n(c[0]),
        n(c[1]),
        n(c[2]),
        n(c[3]),
        n(c[4]),
        n(c[5])
    );
}

fn push_gs(out: &mut String, alpha: f64, alphas: &[u32]) {
    let key = (alpha.clamp(0.0, 1.0) * 1000.0).round() as u32;
    if key >= 1000 {
        return;
    }
    if let Some(index) = alphas.iter().position(|a| *a == key) {
        let _ = write!(out, "/GS{} gs ", index + 1);
    }
}

/// Path construction operators, without the painting operator.
fn path_ops(path: &BezPath) -> String {
    let mut out = String::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                let _ = write!(out, "{} {} m ", n(p.x), n(p.y));
            }
            PathEl::LineTo(p) => {
                let _ = write!(out, "{} {} l ", n(p.x), n(p.y));
            }
            PathEl::QuadTo(c, p) => {
                // PDF has no quadratic operator, so it is raised to the cubic
                // that draws the identical curve.
                let last = last_point(&out);
                let (c1, c2) = quad_to_cubic(last, (c.x, c.y), (p.x, p.y));
                let _ = write!(
                    out,
                    "{} {} {} {} {} {} c ",
                    n(c1.0),
                    n(c1.1),
                    n(c2.0),
                    n(c2.1),
                    n(p.x),
                    n(p.y)
                );
            }
            PathEl::CurveTo(c1, c2, p) => {
                let _ = write!(
                    out,
                    "{} {} {} {} {} {} c ",
                    n(c1.x),
                    n(c1.y),
                    n(c2.x),
                    n(c2.y),
                    n(p.x),
                    n(p.y)
                );
            }
            PathEl::ClosePath => out.push_str("h "),
        }
    }
    out
}

/// Raises a quadratic to the cubic with the same shape.
fn quad_to_cubic(
    from: (f64, f64),
    control: (f64, f64),
    to: (f64, f64),
) -> ((f64, f64), (f64, f64)) {
    let c1 = (
        from.0 + 2.0 / 3.0 * (control.0 - from.0),
        from.1 + 2.0 / 3.0 * (control.1 - from.1),
    );
    let c2 = (
        to.0 + 2.0 / 3.0 * (control.0 - to.0),
        to.1 + 2.0 / 3.0 * (control.1 - to.1),
    );
    (c1, c2)
}

/// The current point, read back from what has been written.
///
/// Kurbo emits quadratics rarely and never as the first element of a path,
/// so the previous operator always carries the point this needs.
fn last_point(ops: &str) -> (f64, f64) {
    let parts: Vec<&str> = ops.split_whitespace().collect();
    // Every operator ends `x y <op>`, so the two before the last token are
    // the point it moved to.
    if parts.len() >= 3 {
        let x = parts[parts.len() - 3].parse().unwrap_or(0.0);
        let y = parts[parts.len() - 2].parse().unwrap_or(0.0);
        return (x, y);
    }
    (0.0, 0.0)
}

/// Assembles the objects, cross-reference table and trailer.
fn assemble(drawing: &Drawing, content: &str, faces: &[Face], alphas: &[u32]) -> Vec<u8> {
    let r = drawing.region;
    let mut objects: Vec<Vec<u8>> = Vec::new();

    // 1 catalogue, 2 pages, 3 page, 4 contents, then fonts, then states.
    let font_first = 5;
    let gs_first = font_first + faces.len();

    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());

    let mut resources = String::from("<< /ProcSet [/PDF /Text]");
    if !faces.is_empty() {
        resources.push_str(" /Font <<");
        for (i, _) in faces.iter().enumerate() {
            let _ = write!(resources, " /F{} {} 0 R", i + 1, font_first + i);
        }
        resources.push_str(" >>");
    }
    if !alphas.is_empty() {
        resources.push_str(" /ExtGState <<");
        for (i, _) in alphas.iter().enumerate() {
            let _ = write!(resources, " /GS{} {} 0 R", i + 1, gs_first + i);
        }
        resources.push_str(" >>");
    }
    resources.push_str(" >>");

    objects.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources {} /Contents 4 0 R >>",
            n(r.w),
            n(r.h),
            resources
        )
        .into_bytes(),
    );

    let stream = content.as_bytes();
    let mut contents = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
    contents.extend_from_slice(stream);
    contents.extend_from_slice(b"\nendstream");
    objects.push(contents);

    for face in faces {
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
                face.base_name()
            )
            .into_bytes(),
        );
    }
    for alpha in alphas {
        let value = *alpha as f64 / 1000.0;
        objects.push(
            format!("<< /Type /ExtGState /ca {} /CA {} >>", n(value), n(value)).into_bytes(),
        );
    }

    let mut out: Vec<u8> = Vec::with_capacity(content.len() + 2048);
    out.extend_from_slice(b"%PDF-1.7\n");
    // A comment of high bytes marks the file as binary for anything that
    // transfers it in text mode.
    out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_at
        )
        .as_bytes(),
    );
    out
}

/// A PDF literal string, escaped and encoded.
pub fn pdf_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('(');
    for c in text.chars() {
        match c {
            '(' => out.push_str(r"\("),
            ')' => out.push_str(r"\)"),
            '\\' => out.push_str(r"\\"),
            '\r' => out.push_str(r"\r"),
            '\n' => out.push_str(r"\n"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, r"\{:03o}", c as u32);
            }
            // WinAnsi is Latin-1 over this range, so the code point is the
            // byte; an octal escape avoids any transport mangling it.
            c if (c as u32) <= 0xFF => {
                let code = c as u32;
                if code > 0x7E {
                    let _ = write!(out, r"\{code:03o}");
                } else {
                    out.push(c);
                }
            }
            // Beyond Latin-1 a standard font has no glyph to name.
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

fn rgb(c: Color) -> String {
    format!(
        "{} {} {}",
        n(c.r as f64 / 255.0),
        n(c.g as f64 / 255.0),
        n(c.b as f64 / 255.0)
    )
}

/// Formats a number for a content stream. PDF has no exponent notation, so a
/// very small value is written as a plain zero rather than as `1e-7`.
fn n(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    if rounded == rounded.trunc() && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(family: &str, bold: bool, italic: bool) -> PositionedRun {
        PositionedRun {
            text: "x".into(),
            x: 0.0,
            y: 0.0,
            font_family: family.into(),
            size_pt: 10.0,
            color: Color::BLACK,
            bold,
            italic,
            underline: false,
            strikethrough: false,
            advance: 5.0,
        }
    }

    #[test]
    fn families_map_onto_the_standard_fonts() {
        assert_eq!(Face::of(&run("Liberation Sans", false, false)).base_name(), "Helvetica");
        assert_eq!(Face::of(&run("Liberation Sans", true, false)).base_name(), "Helvetica-Bold");
        assert_eq!(
            Face::of(&run("Liberation Sans", true, true)).base_name(),
            "Helvetica-BoldOblique"
        );
    }

    #[test]
    fn times_names_its_faces_differently_from_the_others() {
        assert_eq!(Face::of(&run("Times New Roman", false, false)).base_name(), "Times-Roman");
        assert_eq!(Face::of(&run("Times New Roman", false, true)).base_name(), "Times-Italic");
        assert_eq!(Face::of(&run("Courier New", false, true)).base_name(), "Courier-Oblique");
    }

    #[test]
    fn strings_escape_what_would_end_them_early() {
        assert_eq!(pdf_string("a(b)c"), r"(a\(b\)c)");
        assert_eq!(pdf_string(r"back\slash"), r"(back\\slash)");
    }

    #[test]
    fn latin1_is_written_as_octal_and_beyond_it_becomes_a_question_mark() {
        assert_eq!(pdf_string("café"), r"(caf\351)");
        assert_eq!(pdf_string("日本"), "(??)");
    }

    #[test]
    fn numbers_never_use_exponent_notation() {
        assert_eq!(n(0.0000001), "0");
        assert_eq!(n(12.0), "12");
        assert_eq!(n(-0.5), "-0.5");
    }

    #[test]
    fn a_quadratic_is_raised_to_the_cubic_with_the_same_ends() {
        let (c1, c2) = quad_to_cubic((0.0, 0.0), (6.0, 6.0), (12.0, 0.0));
        assert_eq!(c1, (4.0, 4.0));
        assert_eq!(c2, (8.0, 4.0));
    }
}
