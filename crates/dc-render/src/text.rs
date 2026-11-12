//! Text shaping and painting.
//!
//! Layout runs through Parley, which handles shaping, bidi, and line breaking,
//! and the resulting glyph runs are encoded into a Vello scene.
//!
//! The format names Liberation Sans as its default face. That font is rarely
//! installed on macOS or Windows, so requests fall through a stack of
//! metric-compatible alternatives before reaching the system sans-serif. This
//! keeps line breaks close to what the authoring tool produced.

use dc_model::{ParsedText, TextAlign, TextStyle, VerticalAlign};
use kurbo::{Affine, Rect};
use parley::style::{FontFamilyName, FontStyle, FontWeight, GenericFamily};
use parley::{
    Alignment, AlignmentOptions, FontContext, Layout, LayoutContext, OverflowWrap,
    PositionedLayoutItem, StyleProperty,
};
use peniko::{Brush, Color as PenikoColor, Fill};
use vello::Scene;

/// Faces tried in order when a document asks for Liberation Sans, which is
/// metrically compatible with Arial and Helvetica.
const LIBERATION_STACK: &[FontFamilyName<'static>] = &[
    FontFamilyName::named("Liberation Sans"),
    FontFamilyName::named("Arial"),
    FontFamilyName::named("Helvetica"),
    FontFamilyName::Generic(GenericFamily::SansSerif),
];

/// Faces tried for any other requested family: the family itself, then the
/// system sans-serif.
fn family_stack(name: &str) -> Vec<FontFamilyName<'_>> {
    if name.eq_ignore_ascii_case("liberation sans") || name.is_empty() {
        return LIBERATION_STACK.to_vec();
    }
    vec![
        FontFamilyName::named(name),
        FontFamilyName::named("Liberation Sans"),
        FontFamilyName::named("Arial"),
        FontFamilyName::named("Helvetica"),
        FontFamilyName::Generic(GenericFamily::SansSerif),
    ]
}

/// Text laid out and ready to paint.
pub struct TextBlock {
    layout: Layout<Brush>,
    vertical_align: VerticalAlign,
}

impl TextBlock {
    /// Width of the longest line, in document units.
    pub fn width(&self) -> f64 {
        self.layout.width() as f64
    }

    /// Total height of all lines, in document units.
    pub fn height(&self) -> f64 {
        self.layout.height() as f64
    }

    /// Whether there is anything to paint.
    ///
    /// Parley reports one empty line for empty input, so line count is not the
    /// right question to ask here; character count is.
    pub fn is_empty(&self) -> bool {
        self.layout.text_len() == 0
    }
}

/// Owns the font database and layout scratch space.
///
/// Building the font context scans system fonts, so an application should keep
/// one of these for its lifetime rather than creating them per frame.
pub struct TextEngine {
    fonts: FontContext,
    layouts: LayoutContext<Brush>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        TextEngine {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
        }
    }

    /// The underlying Parley contexts, for callers driving Parley's own text
    /// editor.
    ///
    /// Sharing these rather than building a second [`FontContext`] matters for
    /// more than startup cost: an editor resolving fonts differently from the
    /// renderer would wrap text differently while being edited than after.
    pub fn contexts(&mut self) -> (&mut FontContext, &mut LayoutContext<Brush>) {
        (&mut self.fonts, &mut self.layouts)
    }

    /// Lays out parsed text, wrapping to `max_width` document units.
    pub fn layout(&mut self, parsed: &ParsedText, max_width: Option<f64>) -> TextBlock {
        let text = parsed.plain();
        // Borrow the two fields separately; the builder needs both mutably.
        let Self { fonts, layouts } = self;

        let base = parsed
            .runs
            .first()
            .map(|r| r.style.clone())
            .unwrap_or_default();

        // Layout happens in document units; the camera transform supplies zoom
        // at paint time, so glyph outlines stay resolution-independent.
        let mut builder = layouts.ranged_builder(fonts, &text, 1.0, false);

        let base_stack = family_stack(&base.font_family);
        builder.push_default(StyleProperty::FontFamily(base_stack.as_slice().into()));
        builder.push_default(StyleProperty::FontSize(base.font_size_pt as f32));
        builder.push_default(StyleProperty::Brush(Brush::Solid(to_peniko(base.color))));
        // A word longer than the shape is wide has no break opportunity, and
        // without this it runs out past both edges rather than wrapping. Only
        // words that would otherwise overflow are broken, so ordinary text
        // still breaks between words.
        builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));

        // Per-run styling, keyed by byte ranges into the flattened text.
        let mut offset = 0usize;
        let mut stacks: Vec<Vec<FontFamilyName<'_>>> = Vec::with_capacity(parsed.runs.len());
        for run in &parsed.runs {
            stacks.push(family_stack(&run.style.font_family));
        }
        for (run, stack) in parsed.runs.iter().zip(stacks.iter()) {
            let end = offset + run.text.len();
            if run.text.is_empty() {
                continue;
            }
            let range = offset..end;

            builder.push(StyleProperty::FontFamily(stack.as_slice().into()), range.clone());
            builder.push(
                StyleProperty::FontSize(run.style.font_size_pt as f32),
                range.clone(),
            );
            builder.push(
                StyleProperty::Brush(Brush::Solid(to_peniko(run.style.color))),
                range.clone(),
            );
            if run.style.bold {
                builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
            }
            if run.style.italic {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            if run.style.underline {
                builder.push(StyleProperty::Underline(true), range.clone());
            }
            if run.style.strikethrough {
                builder.push(StyleProperty::Strikethrough(true), range.clone());
            }
            offset = end;
        }

        let mut layout = builder.build(&text);
        layout.break_all_lines(max_width.map(|w| w.max(1.0) as f32));
        layout.align(
            to_parley_alignment(parsed.align),
            AlignmentOptions::default(),
        );

        TextBlock { layout, vertical_align: parsed.vertical_align }
    }

    /// Convenience for a single-style string.
    pub fn layout_plain(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f64>,
    ) -> TextBlock {
        let parsed = ParsedText {
            runs: vec![dc_model::TextRun {
                text: text.to_string(),
                style: style.clone(),
                link: None,
            }],
            align: style.align,
            vertical_align: style.vertical_align,
        };
        self.layout(&parsed, max_width)
    }
}

/// Paints a laid-out block inside `rect`, honoring its vertical alignment.
///
/// `transform` maps document space to the target surface.
pub fn draw(scene: &mut Scene, block: &TextBlock, rect: Rect, transform: Affine) {
    draw_clipped(scene, block, rect, transform, None);
}

/// As [`draw`], but confined to `clip`.
///
/// More lines than the shape is tall still overflow — making them fit is the
/// author's decision, not the renderer's — but the overflow is kept inside
/// the shape it belongs to instead of painting over its neighbours.
pub fn draw_clipped(
    scene: &mut Scene,
    block: &TextBlock,
    rect: Rect,
    transform: Affine,
    clip: Option<Rect>,
) {
    if block.is_empty() {
        return;
    }

    // Only overflowing text needs a layer; the common case pays nothing.
    let clip = clip.filter(|c| block.height() > c.height() || block.width() > c.width());
    if let Some(c) = clip {
        scene.push_clip_layer(Fill::NonZero, transform, &c);
    }

    let dy = vertical_offset(block.height(), rect.height(), block.vertical_align);
    // Horizontal placement comes from Parley's own alignment within the
    // layout's advance width, so only the block origin is applied here.
    let origin = Affine::translate((rect.x0, rect.y0 + dy));
    let full = transform * origin;

    for line in block.layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let style = glyph_run.style();
            let brush = style.brush.clone();

            scene
                .draw_glyphs(run.font())
                .font_size(run.font_size())
                .transform(full)
                .brush(&brush)
                .draw(
                    Fill::NonZero,
                    glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    }),
                );

            // Underline and strikethrough are not glyphs; they are rules the
            // renderer has to draw itself.
            let size = run.font_size() as f64;
            let x0 = glyph_run.offset() as f64;
            let x1 = x0 + glyph_run.advance() as f64;
            let baseline = glyph_run.baseline() as f64;

            if let Some(decoration) = &style.underline {
                // Positive offsets sit above the baseline, so an underline's
                // default is a small negative one.
                let offset = decoration.offset.map(|o| -(o as f64)).unwrap_or(size * 0.12);
                let thickness = decoration.size.map(|t| t as f64).unwrap_or(size * 0.06).max(0.5);
                let y = baseline + offset;
                scene.fill(
                    Fill::NonZero,
                    full,
                    &decoration.brush,
                    None,
                    &Rect::new(x0, y, x1, y + thickness),
                );
            }

            if let Some(decoration) = &style.strikethrough {
                let offset = decoration.offset.map(|o| -(o as f64)).unwrap_or(-size * 0.28);
                let thickness = decoration.size.map(|t| t as f64).unwrap_or(size * 0.06).max(0.5);
                let y = baseline + offset;
                scene.fill(
                    Fill::NonZero,
                    full,
                    &decoration.brush,
                    None,
                    &Rect::new(x0, y, x1, y + thickness),
                );
            }
        }
    }

    if clip.is_some() {
        scene.pop_layer();
    }
}

/// Where a block sits within an area of `area_height`, vertically.
///
/// Text taller than its area would otherwise be centred on its own overflow,
/// losing its first line as readily as its last. Pinning it to the top means
/// it reads from the beginning and only the tail runs off the bottom.
fn vertical_offset(block_height: f64, area_height: f64, align: VerticalAlign) -> f64 {
    let dy = match align {
        VerticalAlign::Top => 0.0,
        VerticalAlign::Center => (area_height - block_height) / 2.0,
        VerticalAlign::Bottom => area_height - block_height,
    };
    dy.max(0.0)
}

pub fn to_peniko(c: dc_model::Color) -> PenikoColor {
    PenikoColor::from_rgba8(c.r, c.g, c.b, c.a)
}

fn to_parley_alignment(a: TextAlign) -> Alignment {
    match a {
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
        TextAlign::Justify => Alignment::Justify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::markup;

    fn engine() -> TextEngine {
        TextEngine::new()
    }

    #[test]
    fn plain_text_lays_out_with_positive_extent() {
        let mut e = engine();
        let parsed = markup::parse("Approved?", TextStyle::default());
        let block = e.layout(&parsed, Some(200.0));

        assert!(!block.is_empty());
        assert!(block.width() > 0.0, "width was {}", block.width());
        assert!(block.height() > 0.0, "height was {}", block.height());
    }

    #[test]
    fn empty_text_produces_an_empty_block() {
        let mut e = engine();
        let parsed = markup::parse("", TextStyle::default());
        let block = e.layout(&parsed, Some(200.0));
        assert!(block.is_empty());
    }

    #[test]
    fn a_word_longer_than_the_width_is_broken_rather_than_overflowing() {
        let mut e = engine();
        let parsed = markup::parse("Supercalifragilisticexpialidocious", TextStyle::default());

        // No space to break at, so without overflow-wrap this would lay out
        // as one line running past both edges of its shape.
        let block = e.layout(&parsed, Some(60.0));
        assert!(
            block.width() <= 60.0,
            "the longest line should fit the width, was {}",
            block.width()
        );
        assert!(block.layout.len() > 1, "it should have taken more than one line");
    }

    #[test]
    fn an_ordinary_word_is_not_broken_when_it_fits() {
        let mut e = engine();
        let parsed = markup::parse("Validate fields", TextStyle::default());
        let block = e.layout(&parsed, Some(400.0));
        assert_eq!(
            block.layout.len(),
            1,
            "breaking only applies to words that would otherwise overflow"
        );
    }

    #[test]
    fn text_that_fits_honours_its_vertical_alignment() {
        assert_eq!(vertical_offset(20.0, 100.0, VerticalAlign::Top), 0.0);
        assert_eq!(vertical_offset(20.0, 100.0, VerticalAlign::Center), 40.0);
        assert_eq!(vertical_offset(20.0, 100.0, VerticalAlign::Bottom), 80.0);
    }

    #[test]
    fn text_taller_than_its_area_starts_at_the_top() {
        // Centring would put the overflow across both edges, losing the
        // first line as readily as the last; bottom alignment would lose the
        // beginning outright.
        for align in [VerticalAlign::Top, VerticalAlign::Center, VerticalAlign::Bottom] {
            assert_eq!(
                vertical_offset(140.0, 40.0, align),
                0.0,
                "{align:?} should pin overflowing text to the top"
            );
        }
    }

    #[test]
    fn an_overflowing_block_is_clipped_to_its_shape() {
        let mut e = engine();
        let parsed = markup::parse(&"word ".repeat(40), TextStyle::default());
        let block = e.layout(&parsed, Some(80.0));
        let area = Rect::new(0.0, 0.0, 80.0, 20.0);
        assert!(block.height() > area.height(), "the fixture must overflow to be a test");

        let mut clipped = Scene::new();
        draw_clipped(&mut clipped, &block, area, Affine::IDENTITY, Some(area));
        let mut loose = Scene::new();
        draw_clipped(&mut loose, &block, area, Affine::IDENTITY, None);

        // The clipped pass pushes and pops a layer the loose one does not.
        assert!(
            clipped.encoding().n_clips > loose.encoding().n_clips,
            "overflowing text should be drawn inside a clip layer"
        );
    }

    #[test]
    fn a_block_that_fits_is_not_wrapped_in_a_clip_layer() {
        let mut e = engine();
        let parsed = markup::parse("Short", TextStyle::default());
        let block = e.layout(&parsed, Some(200.0));
        let area = Rect::new(0.0, 0.0, 200.0, 100.0);

        let mut scene = Scene::new();
        draw_clipped(&mut scene, &block, area, Affine::IDENTITY, Some(area));
        assert_eq!(
            scene.encoding().n_clips,
            0,
            "the common case should not pay for a layer"
        );
    }

    #[test]
    fn narrow_width_wraps_to_more_height() {
        let mut e = engine();
        let parsed = markup::parse(
            "a reasonably long label that must wrap onto several lines",
            TextStyle::default(),
        );

        let wide = e.layout(&parsed, Some(1000.0)).height();
        let narrow = e.layout(&parsed, Some(80.0)).height();
        assert!(narrow > wide, "narrow {narrow} should exceed wide {wide}");
    }

    #[test]
    fn larger_font_size_produces_taller_text() {
        let mut e = engine();
        let small = TextStyle { font_size_pt: 8.0, ..Default::default() };
        let large = TextStyle { font_size_pt: 32.0, ..Default::default() };

        let h_small = e.layout_plain("Hg", &small, None).height();
        let h_large = e.layout_plain("Hg", &large, None).height();
        assert!(h_large > h_small * 2.0, "{h_large} vs {h_small}");
    }

    #[test]
    fn explicit_line_break_adds_a_line() {
        let mut e = engine();
        let one = markup::parse("one", TextStyle::default());
        let two = markup::parse("one<br>two", TextStyle::default());

        let h1 = e.layout(&one, None).height();
        let h2 = e.layout(&two, None).height();
        assert!(h2 > h1 * 1.5, "{h2} should be about twice {h1}");
    }

    #[test]
    fn bold_runs_do_not_break_layout() {
        let mut e = engine();
        let parsed = markup::parse("normal <b>bold</b> normal", TextStyle::default());
        let block = e.layout(&parsed, Some(500.0));
        assert!(block.width() > 0.0);
    }

    #[test]
    fn vertical_alignment_is_carried_through() {
        let mut e = engine();
        let parsed = markup::parse(r#"<div style="vertical-align: top">x</div>"#, TextStyle::default());
        let block = e.layout(&parsed, None);
        assert_eq!(block.vertical_align, VerticalAlign::Top);
    }

    #[test]
    fn drawing_into_a_scene_encodes_something() {
        let mut e = engine();
        let parsed = markup::parse("Start", TextStyle::default());
        let block = e.layout(&parsed, Some(120.0));

        let mut scene = Scene::new();
        assert_eq!(scene.encoding().n_paths, 0);
        draw(
            &mut scene,
            &block,
            Rect::new(0.0, 0.0, 120.0, 50.0),
            Affine::IDENTITY,
        );
        // Glyph drawing pushes draw objects into the encoding.
        assert!(
            scene.encoding().draw_tags.len() > 0,
            "nothing was encoded for the glyph run"
        );
    }

    #[test]
    fn drawing_empty_text_is_a_no_op() {
        let mut e = engine();
        let parsed = markup::parse("", TextStyle::default());
        let block = e.layout(&parsed, None);

        let mut scene = Scene::new();
        draw(&mut scene, &block, Rect::new(0.0, 0.0, 10.0, 10.0), Affine::IDENTITY);
        assert_eq!(scene.encoding().draw_tags.len(), 0);
    }

    #[test]
    fn liberation_sans_requests_fall_back_rather_than_failing() {
        // The named face is usually absent; layout must still produce glyphs.
        let mut e = engine();
        let style = TextStyle { font_family: "Liberation Sans".into(), ..Default::default() };
        let block = e.layout_plain("fallback", &style, None);
        assert!(block.width() > 0.0, "no glyphs were produced");
    }

    #[test]
    fn unknown_family_falls_back_too() {
        let mut e = engine();
        let style = TextStyle {
            font_family: "NoSuchFontExistsAnywhere".into(),
            ..Default::default()
        };
        let block = e.layout_plain("fallback", &style, None);
        assert!(block.width() > 0.0);
    }

    #[test]
    fn family_stack_always_ends_at_a_generic() {
        for name in ["Liberation Sans", "", "Comic Sans MS"] {
            let stack = family_stack(name);
            assert!(matches!(
                stack.last(),
                Some(FontFamilyName::Generic(GenericFamily::SansSerif))
            ));
        }
    }
}

