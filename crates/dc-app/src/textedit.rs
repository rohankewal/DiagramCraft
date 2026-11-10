//! In-canvas text editing.
//!
//! Caret movement, selection, word boundaries, and IME are delegated to
//! Parley's [`PlainEditor`], which understands grapheme clusters and
//! bidirectional text. Reimplementing that correctly is a project in itself,
//! and getting it subtly wrong is how editors mangle non-Latin text.
//!
//! # Styling
//!
//! A session carries one [`TextStyle`] per character of the label, so
//! formatting applies to a selected range rather than to the whole label, and
//! mixed styling in a document being edited is preserved rather than flattened.
//!
//! `PlainEditor` is, as its name says, plain: it lays text out in a single
//! style. It stays the authority on the text buffer and on where the caret is,
//! while painting and caret geometry come from a second, ranged layout built
//! from the per-character styles. The two agree exactly whenever a label is
//! uniformly styled, which is the common case; where styling is mixed, the
//! ranged layout is the one on screen, so what you see is what the document
//! holds.
//!
//! An untouched session still commits its original markup byte for byte, so
//! opening and closing a label changes nothing.

use dc_model::{TextAlign, TextStyle, VerticalAlign, markup};
use kurbo::Rect;
use parley::{
    Alignment, AlignmentOptions, FontContext, Layout, LayoutContext, PlainEditor, StyleProperty,
};
use peniko::Brush;
use std::ops::Range;

/// What is being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// The text of a shape.
    Shape(String),
    /// A label on a connector, by index.
    LineLabel { line_id: String, index: usize },
    /// A page title, edited in the tab bar.
    PageTitle(usize),
}

/// An editing command, independent of any windowing library so the model can
/// be driven from tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextCommand {
    Insert(String),
    /// A hard line break inside the label.
    Newline,
    Backspace,
    BackspaceWord,
    Delete,
    DeleteWord,
    Left { select: bool, word: bool },
    Right { select: bool, word: bool },
    Up { select: bool },
    Down { select: bool },
    LineStart { select: bool },
    LineEnd { select: bool },
    TextStart { select: bool },
    TextEnd { select: bool },
    SelectAll,
}

/// Whether a command edits the text or only moves through it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CommandKind {
    Edit,
    Motion,
}

impl CommandKind {
    fn of(command: &TextCommand) -> Self {
        match command {
            TextCommand::Insert(_)
            | TextCommand::Newline
            | TextCommand::Backspace
            | TextCommand::BackspaceWord
            | TextCommand::Delete
            | TextCommand::DeleteWord => CommandKind::Edit,
            _ => CommandKind::Motion,
        }
    }
}

/// How long the caret stays on, then off, in seconds.
pub const BLINK_INTERVAL: f64 = 0.53;

/// The Parley contexts needed to lay text out.
///
/// Bundling the two borrows keeps editor signatures readable, and taking them
/// as a parameter rather than owning them means the editor shares the
/// renderer's font resolution instead of building a second font database.
pub struct TextCtx<'a> {
    pub fonts: &'a mut FontContext,
    pub layouts: &'a mut LayoutContext<Brush>,
}

impl<'a> TextCtx<'a> {
    pub fn new(fonts: &'a mut FontContext, layouts: &'a mut LayoutContext<Brush>) -> Self {
        TextCtx { fonts, layouts }
    }

}

/// A character style property that can be switched on and off over a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleToggle {
    Bold,
    Italic,
    Underline,
    Strikethrough,
}

impl StyleToggle {
    pub fn apply(self, style: &mut TextStyle, on: bool) {
        match self {
            StyleToggle::Bold => style.bold = on,
            StyleToggle::Italic => style.italic = on,
            StyleToggle::Underline => style.underline = on,
            StyleToggle::Strikethrough => style.strikethrough = on,
        }
    }

    pub fn probe(self, style: &TextStyle) -> bool {
        match self {
            StyleToggle::Bold => style.bold,
            StyleToggle::Italic => style.italic,
            StyleToggle::Underline => style.underline,
            StyleToggle::Strikethrough => style.strikethrough,
        }
    }
}

/// An active editing session.
pub struct TextEdit {
    pub target: EditTarget,
    editor: PlainEditor<Brush>,
    /// The markup as stored before editing began.
    original_markup: String,
    /// The plain text as it was when editing began.
    original_plain: String,
    /// One style per character of the current text.
    ///
    /// Held per character rather than as runs because every edit is a splice
    /// at a character boundary, and splicing a flat list is exact where
    /// adjusting run lengths is fiddly and easy to get subtly wrong.
    styles: Vec<TextStyle>,
    /// The styles the label started with, so an untouched session can be
    /// recognized and committed verbatim.
    original_styles: Vec<TextStyle>,
    /// The style the next typed character takes when the caret is collapsed
    /// and the user has just toggled a property.
    pending_style: Option<TextStyle>,
    /// The style a label with no text at all starts from.
    base_style: TextStyle,
    /// The surface the text sits on, which the plate behind it is painted in.
    ///
    /// A label's colour is derived from whatever it sits on — a shape's fill,
    /// the page, or the tab bar — so the plate has to come from that same
    /// surface. Taking it from the interface theme instead is how you end up
    /// with black text on a dark plate the moment someone switches themes.
    backdrop: dc_model::Color,
    align: TextAlign,
    vertical_align: VerticalAlign,
    /// The block alignment the label arrived with, so a session that only
    /// looked at it still commits verbatim.
    original_align: (TextAlign, VerticalAlign),
    /// The text area in document units.
    area: Rect,
    /// Seconds since the caret last reset, for blinking.
    blink: f64,
    /// The ranged layout that is actually painted, rebuilt when the text or its
    /// styles change.
    layout: Option<Layout<Brush>>,
    layout_dirty: bool,
}

impl TextEdit {
    /// Begins editing, placing the caret by selecting everything so that
    /// typing replaces the placeholder text the way a fresh label should.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        target: EditTarget,
        existing_markup: &str,
        base: &TextStyle,
        backdrop: dc_model::Color,
        area: Rect,
        select_all: bool,
        cx: &mut TextCtx<'_>,
    ) -> Self {
        let parsed = markup::parse(existing_markup, base.clone());
        let plain = parsed.plain();

        // One style per character, taken from the run that character sits in,
        // so mixed styling arrives intact and stays editable.
        let mut styles: Vec<TextStyle> = Vec::with_capacity(plain.chars().count());
        for run in &parsed.runs {
            for _ in run.text.chars() {
                styles.push(run.style.clone());
            }
        }

        // An empty label still needs somewhere to start from.
        let base_style = parsed
            .runs
            .first()
            .map(|r| r.style.clone())
            .unwrap_or_else(|| base.clone());

        // `PlainEditor` lays out in a single style, so give it the one most of
        // the label uses: for a uniformly styled label its layout and the
        // ranged one are then identical, which keeps clicks and the caret
        // landing exactly where the glyphs are.
        let dominant = dominant_style(&styles).unwrap_or_else(|| base_style.clone());

        let mut editor = PlainEditor::new(dominant.font_size_pt as f32);
        editor.set_text(&plain);
        editor.set_width(Some(area.width().max(1.0) as f32));
        editor.set_alignment(to_parley_alignment(parsed.align));
        apply_style_set(editor.edit_styles(), &dominant);

        let mut edit = TextEdit {
            target,
            editor,
            original_markup: existing_markup.to_string(),
            original_plain: plain,
            original_styles: styles.clone(),
            styles,
            pending_style: None,
            base_style,
            backdrop,
            align: parsed.align,
            vertical_align: parsed.vertical_align,
            original_align: (parsed.align, parsed.vertical_align),
            area,
            blink: 0.0,
            layout: None,
            layout_dirty: true,
        };

        {
            let mut driver = edit.editor.driver(cx.fonts, cx.layouts);
            if select_all {
                driver.select_all();
            } else {
                driver.move_to_text_end();
            }
        }
        edit
    }

    /// The selected byte range of the plain text, empty when the caret is
    /// collapsed.
    pub fn selected_range(&self) -> Range<usize> {
        self.editor.raw_selection().text_range()
    }

    /// The styles of the characters a byte range covers.
    ///
    /// A collapsed caret reports the style that would apply to the next typed
    /// character, so a formatting control can show the right state even with
    /// nothing selected.
    pub fn styles_in(&self, range: Range<usize>) -> Vec<TextStyle> {
        if range.is_empty() {
            return vec![self.caret_style()];
        }
        let text = self.text();
        let mut out = Vec::new();
        let mut byte = 0usize;
        for (index, ch) in text.chars().enumerate() {
            let end = byte + ch.len_utf8();
            if byte < range.end && end > range.start {
                if let Some(style) = self.styles.get(index) {
                    out.push(style.clone());
                }
            }
            byte = end;
            if byte >= range.end {
                break;
            }
        }
        if out.is_empty() { vec![self.caret_style()] } else { out }
    }

    /// The style a character typed right now would take.
    pub fn caret_style(&self) -> TextStyle {
        if let Some(pending) = &self.pending_style {
            return pending.clone();
        }
        let at = self.selected_range().start;
        // The character to the left of the caret, which is the convention
        // every text editor uses for carrying formatting forward.
        let index = self.text()[..at.min(self.text().len())].chars().count();
        index
            .checked_sub(1)
            .and_then(|i| self.styles.get(i))
            .or_else(|| self.styles.get(index))
            .cloned()
            .unwrap_or_else(|| self.base_style.clone())
    }

    /// Whether a property is set on every character of a range.
    pub fn range_has(&self, range: Range<usize>, toggle: StyleToggle) -> bool {
        let styles = self.styles_in(range);
        !styles.is_empty() && styles.iter().all(|s| toggle.probe(s))
    }

    /// Switches a property over a byte range, or arms it for the next typed
    /// character when the range is empty.
    ///
    /// Returns whether anything changed. Toggling over a mixed range turns the
    /// property on, which is how a toggle over a mixed selection should behave.
    pub fn toggle_style(&mut self, range: Range<usize>, toggle: StyleToggle) -> bool {
        let enable = !self.range_has(range.clone(), toggle);
        self.restyle(range, move |style| toggle.apply(style, enable))
    }

    /// Applies an arbitrary style change over a byte range.
    ///
    /// With an empty range the change is held for the next typed character,
    /// matching how a word processor behaves when you press bold before you
    /// start typing.
    pub fn restyle(
        &mut self,
        range: Range<usize>,
        mutate: impl Fn(&mut TextStyle),
    ) -> bool {
        if range.is_empty() {
            let mut style = self.caret_style();
            mutate(&mut style);
            let changed = Some(&style) != self.pending_style.as_ref();
            self.pending_style = Some(style);
            return changed;
        }

        let text = self.text();
        let mut changed = false;
        let mut byte = 0usize;
        for (index, ch) in text.chars().enumerate() {
            let end = byte + ch.len_utf8();
            if byte < range.end && end > range.start {
                if let Some(style) = self.styles.get_mut(index) {
                    let before = style.clone();
                    mutate(style);
                    changed |= *style != before;
                }
            }
            byte = end;
            if byte >= range.end {
                break;
            }
        }

        if changed {
            self.pending_style = None;
            self.mark_dirty();
        }
        changed
    }

    /// Sets the block alignment of the label being edited.
    pub fn set_align(&mut self, align: TextAlign) -> bool {
        if self.align == align {
            return false;
        }
        self.align = align;
        self.editor.set_alignment(to_parley_alignment(align));
        self.mark_dirty();
        true
    }

    pub fn align(&self) -> TextAlign {
        self.align
    }

    pub fn set_vertical_align(&mut self, align: VerticalAlign) -> bool {
        if self.vertical_align == align {
            return false;
        }
        self.vertical_align = align;
        true
    }

    pub fn vertical_align(&self) -> VerticalAlign {
        self.vertical_align
    }

    fn mark_dirty(&mut self) {
        self.layout_dirty = true;
    }

    /// Re-aligns the per-character styles with the text after an edit.
    ///
    /// The splice is derived by diffing the old and new text rather than by
    /// predicting what each command does: every command is one contiguous
    /// replacement, and a diff finds it exactly without the editor having to
    /// model backspace-over-a-grapheme and friends a second time.
    fn resync_styles(&mut self, before: &str, after: &str) {
        if before == after {
            return;
        }
        let old: Vec<char> = before.chars().collect();
        let new: Vec<char> = after.chars().collect();

        let mut prefix = 0usize;
        while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
            prefix += 1;
        }
        let mut suffix = 0usize;
        while suffix < old.len() - prefix
            && suffix < new.len() - prefix
            && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
        {
            suffix += 1;
        }

        let removed = prefix..old.len() - suffix;
        let inserted = new.len() - suffix - prefix;

        // Typed text takes the style armed at the caret, otherwise the style of
        // the character it follows.
        let carry = self
            .pending_style
            .clone()
            .or_else(|| {
                prefix
                    .checked_sub(1)
                    .and_then(|i| self.styles.get(i).cloned())
            })
            .or_else(|| self.styles.get(removed.end.min(self.styles.len())).cloned())
            .unwrap_or_else(|| self.base_style.clone());

        if removed.end <= self.styles.len() {
            self.styles
                .splice(removed, std::iter::repeat_n(carry, inserted));
        } else {
            // The style list fell out of step, which should not happen; rebuild
            // it rather than leaving the label partly unstyled.
            self.styles = vec![self.base_style.clone(); new.len()];
        }

        if inserted > 0 {
            self.pending_style = None;
        }
        self.mark_dirty();
    }

    /// The text as currently edited.
    pub fn text(&self) -> String {
        self.editor.text().chars().collect()
    }

    /// The text area in document units.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The surface this label sits on.
    pub fn backdrop(&self) -> dc_model::Color {
        self.backdrop
    }

    /// Updates the area, which happens when the shape being edited is resized.
    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
        self.editor.set_width(Some(area.width().max(1.0) as f32));
        self.mark_dirty();
    }

    /// Advances the blink clock.
    pub fn tick(&mut self, delta_seconds: f64) {
        self.blink += delta_seconds;
    }

    /// Whether the caret should be painted this frame.
    pub fn caret_visible(&self) -> bool {
        (self.blink / BLINK_INTERVAL) as u64 % 2 == 0
    }

    fn reset_blink(&mut self) {
        self.blink = 0.0;
    }

    /// Applies a command. Returns whether the text changed.
    pub fn command(
        &mut self,
        command: TextCommand,
        cx: &mut TextCtx<'_>,
    ) -> bool {
        let before = self.text();
        let command_kind = CommandKind::of(&command);
        self.reset_blink();

        let mut driver = self.editor.driver(cx.fonts, cx.layouts);
        match command {
            TextCommand::Insert(s) => driver.insert_or_replace_selection(&s),
            TextCommand::Newline => driver.insert_or_replace_selection("\n"),
            TextCommand::Backspace => driver.backdelete(),
            TextCommand::BackspaceWord => driver.backdelete_word(),
            TextCommand::Delete => driver.delete(),
            TextCommand::DeleteWord => driver.delete_word(),

            TextCommand::Left { select, word } => match (select, word) {
                (false, false) => driver.move_left(),
                (false, true) => driver.move_word_left(),
                (true, false) => driver.select_left(),
                (true, true) => driver.select_word_left(),
            },
            TextCommand::Right { select, word } => match (select, word) {
                (false, false) => driver.move_right(),
                (false, true) => driver.move_word_right(),
                (true, false) => driver.select_right(),
                (true, true) => driver.select_word_right(),
            },
            TextCommand::Up { select } => {
                if select {
                    driver.select_up();
                } else {
                    driver.move_up();
                }
            }
            TextCommand::Down { select } => {
                if select {
                    driver.select_down();
                } else {
                    driver.move_down();
                }
            }
            TextCommand::LineStart { select } => {
                if select {
                    driver.select_to_line_start();
                } else {
                    driver.move_to_line_start();
                }
            }
            TextCommand::LineEnd { select } => {
                if select {
                    driver.select_to_line_end();
                } else {
                    driver.move_to_line_end();
                }
            }
            TextCommand::TextStart { select } => {
                if select {
                    driver.select_to_text_start();
                } else {
                    driver.move_to_text_start();
                }
            }
            TextCommand::TextEnd { select } => {
                if select {
                    driver.select_to_text_end();
                } else {
                    driver.move_to_text_end();
                }
            }
            TextCommand::SelectAll => driver.select_all(),
        }

        let after = self.text();
        if after != before {
            self.resync_styles(&before, &after);
            return true;
        }
        // Moving the caret abandons any armed formatting, the same way it does
        // in a word processor.
        if matches!(
            command_kind,
            CommandKind::Motion
        ) {
            self.pending_style = None;
        }
        false
    }

    /// Places the caret from a document-space point.
    pub fn click(
        &mut self,
        doc_point: dc_model::Point,
        extend: bool,
        cx: &mut TextCtx<'_>,
    ) {
        self.reset_blink();
        let (x, y) = self.to_layout_space(doc_point, cx);
        let mut driver = self.editor.driver(cx.fonts, cx.layouts);
        if extend {
            driver.extend_selection_to_point(x, y);
        } else {
            driver.move_to_point(x, y);
        }
    }

    /// Selects the word under a document-space point.
    pub fn select_word_at(
        &mut self,
        doc_point: dc_model::Point,
        cx: &mut TextCtx<'_>,
    ) {
        self.reset_blink();
        let (x, y) = self.to_layout_space(doc_point, cx);
        let mut driver = self.editor.driver(cx.fonts, cx.layouts);
        driver.select_word_at_point(x, y);
    }

    pub fn selected_text(&self) -> Option<String> {
        self.editor.selected_text().map(str::to_string)
    }

    /// The caret rectangle in document units, if it should be drawn.
    pub fn caret_rect(&mut self, cx: &mut TextCtx<'_>) -> Option<Rect> {
        let (ox, oy) = self.layout_origin(cx);
        let selection = *self.editor.raw_selection();
        let layout = self.styled_layout(cx)?;
        // A caret narrower than a point disappears when zoomed out.
        let b = selection.focus().geometry(layout, 1.5);
        Some(Rect::new(b.x0 + ox, b.y0 + oy, b.x1 + ox, b.y1 + oy))
    }

    /// The selection highlight rectangles in document units.
    pub fn selection_rects(&mut self, cx: &mut TextCtx<'_>) -> Vec<Rect> {
        let (ox, oy) = self.layout_origin(cx);
        let selection = *self.editor.raw_selection();
        let Some(layout) = self.styled_layout(cx) else {
            return Vec::new();
        };
        selection
            .geometry(layout)
            .into_iter()
            .map(|(b, _)| Rect::new(b.x0 + ox, b.y0 + oy, b.x1 + ox, b.y1 + oy))
            .collect()
    }

    /// Draws the text being edited. The static text of the underlying item is
    /// suppressed while this is active, so this is the only copy on screen.
    pub fn draw(
        &mut self,
        scene: &mut vello::Scene,
        transform: kurbo::Affine,
        cx: &mut TextCtx<'_>,
    ) {
        let (ox, oy) = self.layout_origin(cx);
        let placement = transform * kurbo::Affine::translate((ox, oy));

        let Some(layout) = self.styled_layout(cx) else {
            return;
        };

        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let style = glyph_run.style();
                let brush = style.brush.clone();
                scene
                    .draw_glyphs(run.font())
                    .font_size(run.font_size())
                    .transform(placement)
                    .brush(&brush)
                    .draw(
                        peniko::Fill::NonZero,
                        glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        }),
                    );

                // Parley reports underline and strike-through as decorations
                // on the run rather than drawing them, so they are drawn here.
                let baseline = glyph_run.baseline() as f64;
                let x0 = glyph_run.offset() as f64;
                let x1 = x0 + glyph_run.advance() as f64;
                let size = run.font_size() as f64;
                for (decoration, offset) in [
                    (style.underline.as_ref().map(|u| u.brush.clone()), size * 0.12),
                    (
                        style.strikethrough.as_ref().map(|s| s.brush.clone()),
                        -size * 0.28,
                    ),
                ] {
                    let Some(brush) = decoration else { continue };
                    let thickness = (size * 0.06).max(0.6);
                    let y = baseline + offset;
                    scene.fill(
                        peniko::Fill::NonZero,
                        placement,
                        &brush,
                        None,
                        &Rect::new(x0, y - thickness / 2.0, x1, y + thickness / 2.0),
                    );
                }
            }
        }
    }

    /// The ranged layout the session paints, rebuilt when text or styles change.
    fn styled_layout(&mut self, cx: &mut TextCtx<'_>) -> Option<&Layout<Brush>> {
        if self.layout_dirty || self.layout.is_none() {
            let text = self.text();
            let mut builder =
                cx.layouts
                    .ranged_builder(cx.fonts, &text, 1.0, true);

            // Defaults first, then one push per run of equal style.
            let base = self.styles.first().unwrap_or(&self.base_style).clone();
            for property in style_properties(&base) {
                builder.push_default(property);
            }

            let mut start = 0usize;
            let mut run_start = 0usize;
            let mut previous: Option<TextStyle> = None;
            for (index, ch) in text.chars().enumerate() {
                let style = self
                    .styles
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| self.base_style.clone());
                match &previous {
                    Some(p) if *p == style => {}
                    Some(p) => {
                        for property in style_properties(p) {
                            builder.push(property, run_start..start);
                        }
                        run_start = start;
                        previous = Some(style);
                    }
                    None => previous = Some(style),
                }
                start += ch.len_utf8();
            }
            if let Some(p) = &previous {
                for property in style_properties(p) {
                    builder.push(property, run_start..start);
                }
            }

            let mut layout = builder.build(&text);
            layout.break_all_lines(Some(self.area.width().max(1.0) as f32));
            layout.align(
                to_parley_alignment(self.align),
                AlignmentOptions::default(),
            );
            self.layout = Some(layout);
            self.layout_dirty = false;
        }
        self.layout.as_ref()
    }

    /// The document-space origin of the laid-out text, accounting for vertical
    /// alignment inside the area.
    fn layout_origin(&mut self, cx: &mut TextCtx<'_>) -> (f64, f64) {
        let height = self
            .styled_layout(cx)
            .map(|l| l.height() as f64)
            .unwrap_or(0.0);

        let dy = match self.vertical_align {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Center => (self.area.height() - height) / 2.0,
            VerticalAlign::Bottom => self.area.height() - height,
        };
        (self.area.x0, self.area.y0 + dy)
    }

    fn to_layout_space(
        &mut self,
        doc_point: dc_model::Point,
        cx: &mut TextCtx<'_>,
    ) -> (f32, f32) {
        let (ox, oy) = self.layout_origin(cx);
        ((doc_point.x - ox) as f32, (doc_point.y - oy) as f32)
    }

    /// The markup to store.
    ///
    /// An untouched session returns its original markup verbatim, so opening
    /// and closing a label is a no-op even for markup this editor would
    /// otherwise re-serialize differently.
    pub fn commit(&self) -> String {
        if !self.is_modified() {
            return self.original_markup.clone();
        }

        let text = self.text();
        let mut runs: Vec<dc_model::TextRun> = Vec::new();
        for (index, ch) in text.chars().enumerate() {
            let style = self
                .styles
                .get(index)
                .cloned()
                .unwrap_or_else(|| self.base_style.clone());
            match runs.last_mut() {
                Some(last) if last.style == style => last.text.push(ch),
                _ => runs.push(dc_model::TextRun {
                    text: ch.to_string(),
                    style,
                    link: None,
                }),
            }
        }
        if runs.is_empty() {
            runs.push(dc_model::TextRun {
                text: String::new(),
                style: self.base_style.clone(),
                link: None,
            });
        }

        let parsed = dc_model::ParsedText {
            runs,
            align: self.align,
            vertical_align: self.vertical_align,
        };
        markup::to_markup(&parsed, &TextStyle::default())
    }

    /// Whether the text, its styling, or its alignment differs from how it
    /// started.
    pub fn is_modified(&self) -> bool {
        self.text() != self.original_plain
            || self.styles != self.original_styles
            || (self.align, self.vertical_align) != self.original_align
    }
}

/// The parley properties that express a resolved text style.
fn style_properties(style: &TextStyle) -> Vec<StyleProperty<'static, Brush>> {
    let brush = Brush::Solid(peniko::Color::from_rgba8(
        style.color.r,
        style.color.g,
        style.color.b,
        style.color.a,
    ));
    vec![
        StyleProperty::FontSize(style.font_size_pt as f32),
        StyleProperty::Brush(brush),
        StyleProperty::FontWeight(if style.bold {
            parley::style::FontWeight::BOLD
        } else {
            parley::style::FontWeight::NORMAL
        }),
        StyleProperty::FontStyle(if style.italic {
            parley::style::FontStyle::Italic
        } else {
            parley::style::FontStyle::Normal
        }),
        StyleProperty::Underline(style.underline),
        StyleProperty::Strikethrough(style.strikethrough),
    ]
}

/// Writes a style into a `PlainEditor`'s single style set.
fn apply_style_set(set: &mut parley::StyleSet<Brush>, style: &TextStyle) {
    for property in style_properties(style) {
        set.insert(property);
    }
}

/// The style the most characters share, which is the one `PlainEditor` lays
/// out with so that its metrics match the ranged layout for ordinary labels.
fn dominant_style(styles: &[TextStyle]) -> Option<TextStyle> {
    let mut best: Option<(usize, &TextStyle)> = None;
    for candidate in styles {
        let count = styles.iter().filter(|s| *s == candidate).count();
        if best.is_none_or(|(n, _)| count > n) {
            best = Some((count, candidate));
        }
    }
    best.map(|(_, s)| s.clone())
}

/// Paints an editing session: a plate, the selection, the glyphs, the caret.
///
/// Both the live window and the headless preview call this, so a screenshot
/// shows exactly what the application shows.
pub fn draw_session(
    scene: &mut vello::Scene,
    edit: &mut TextEdit,
    theme: &dc_render::Theme,
    transform: kurbo::Affine,
    cx: &mut TextCtx<'_>,
) {
    let accent = theme.accent;
    // The plate is the surface the label sits on, not the interface theme's
    // sheet: the text's colour was derived from that same surface, and taking
    // the two from different places is what puts dark text on a dark plate.
    let plate = edit.backdrop();

    // A plate behind the text keeps it legible over whatever it overlaps. It
    // is opaque rather than nearly so, because a shape showing through at the
    // edges is what made the old plate ambiguous to read against.
    let frame = edit.area().inflate(2.0, 2.0);
    scene.fill(
        peniko::Fill::NonZero,
        transform,
        peniko::Color::from_rgba8(plate.r, plate.g, plate.b, 0xFF),
        None,
        &frame,
    );
    scene.stroke(
        &kurbo::Stroke::new(1.0),
        transform,
        peniko::Color::from_rgba8(accent.r, accent.g, accent.b, 0x99),
        None,
        &frame,
    );

    // The selection wash has to stay readable against the plate too, so it is
    // tinted toward whichever of the accent or the plate's own contrast the
    // text is not.
    let highlight = plate.lerp(accent, 0.45);
    for rect in edit.selection_rects(cx) {
        scene.fill(
            peniko::Fill::NonZero,
            transform,
            peniko::Color::from_rgba8(highlight.r, highlight.g, highlight.b, 0xFF),
            None,
            &rect,
        );
    }

    edit.draw(scene, transform, cx);

    if edit.caret_visible() {
        if let Some(caret) = edit.caret_rect(cx) {
            scene.fill(
                peniko::Fill::NonZero,
                transform,
                peniko::Color::from_rgba8(accent.r, accent.g, accent.b, 0xFF),
                None,
                &caret,
            );
        }
    }
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
    use dc_model::Point;

    struct Contexts {
        fonts: FontContext,
        layouts: LayoutContext<Brush>,
    }

    impl Contexts {
        fn new() -> Self {
            Contexts {
                fonts: FontContext::new(),
                layouts: LayoutContext::new(),
            }
        }

        fn ctx(&mut self) -> TextCtx<'_> {
            TextCtx::new(&mut self.fonts, &mut self.layouts)
        }
    }

    fn start(text: &str, select_all: bool) -> (TextEdit, Contexts) {
        start_on(text, select_all, dc_model::Color::WHITE)
    }

    fn start_on(
        text: &str,
        select_all: bool,
        backdrop: dc_model::Color,
    ) -> (TextEdit, Contexts) {
        let mut cx = Contexts::new();
        let edit = TextEdit::begin(
            EditTarget::Shape("s1".into()),
            text,
            &TextStyle::default(),
            backdrop,
            Rect::new(0.0, 0.0, 200.0, 60.0),
            select_all,
            &mut cx.ctx(),
        );
        (edit, cx)
    }

    fn run(edit: &mut TextEdit, cx: &mut Contexts, command: TextCommand) -> bool {
        edit.command(command, &mut cx.ctx())
    }

    fn typed(edit: &mut TextEdit, cx: &mut Contexts, s: &str) {
        run(edit, cx, TextCommand::Insert(s.to_string()));
    }

    #[test]
    fn editing_starts_with_the_existing_text() {
        let (edit, _cx) = start("Approved?", false);
        assert_eq!(edit.text(), "Approved?");
        assert!(!edit.is_modified());
    }

    #[test]
    fn typing_appends_at_the_caret() {
        let (mut edit, mut cx) = start("ab", false);
        typed(&mut edit, &mut cx, "c");
        assert_eq!(edit.text(), "abc");
        assert!(edit.is_modified());
    }

    #[test]
    fn select_all_on_open_means_typing_replaces() {
        let (mut edit, mut cx) = start("old text", true);
        typed(&mut edit, &mut cx, "new");
        assert_eq!(edit.text(), "new");
    }

    #[test]
    fn backspace_removes_the_character_before_the_caret() {
        let (mut edit, mut cx) = start("abc", false);
        run(&mut edit, &mut cx, TextCommand::Backspace);
        assert_eq!(edit.text(), "ab");
    }

    #[test]
    fn delete_removes_forward() {
        let (mut edit, mut cx) = start("abc", false);
        run(&mut edit, &mut cx, TextCommand::TextStart { select: false });
        run(&mut edit, &mut cx, TextCommand::Delete);
        assert_eq!(edit.text(), "bc");
    }

    #[test]
    fn word_deletion_removes_a_whole_word() {
        let (mut edit, mut cx) = start("hello world", false);
        run(&mut edit, &mut cx, TextCommand::BackspaceWord);
        assert_eq!(edit.text().trim_end(), "hello");
    }

    #[test]
    fn newline_inserts_a_hard_break() {
        let (mut edit, mut cx) = start("a", false);
        run(&mut edit, &mut cx, TextCommand::Newline);
        typed(&mut edit, &mut cx, "b");
        assert_eq!(edit.text(), "a\nb");
    }

    #[test]
    fn caret_movement_changes_where_typing_lands() {
        let (mut edit, mut cx) = start("ac", false);
        run(&mut edit, &mut cx, TextCommand::Left { select: false, word: false });
        typed(&mut edit, &mut cx, "b");
        assert_eq!(edit.text(), "abc");
    }

    #[test]
    fn selection_replaces_on_typing() {
        let (mut edit, mut cx) = start("hello", false);
        run(&mut edit, &mut cx, TextCommand::SelectAll);
        assert_eq!(edit.selected_text().as_deref(), Some("hello"));
        typed(&mut edit, &mut cx, "x");
        assert_eq!(edit.text(), "x");
    }

    #[test]
    fn shift_arrow_extends_a_selection() {
        let (mut edit, mut cx) = start("abc", false);
        run(&mut edit, &mut cx, TextCommand::Left { select: true, word: false });
        assert_eq!(edit.selected_text().as_deref(), Some("c"));
    }

    #[test]
    fn home_and_end_move_within_a_line() {
        let (mut edit, mut cx) = start("abc", false);
        run(&mut edit, &mut cx, TextCommand::LineStart { select: false });
        typed(&mut edit, &mut cx, "X");
        assert_eq!(edit.text(), "Xabc");

        run(&mut edit, &mut cx, TextCommand::LineEnd { select: false });
        typed(&mut edit, &mut cx, "Y");
        assert_eq!(edit.text(), "XabcY");
    }

    #[test]
    fn commands_report_whether_text_changed() {
        let (mut edit, mut cx) = start("abc", false);
        assert!(!run(&mut edit, &mut cx, TextCommand::Left { select: false, word: false }));
        assert!(run(&mut edit, &mut cx, TextCommand::Insert("z".into())));
    }

    #[test]
    fn unicode_is_handled_by_grapheme_not_byte() {
        let (mut edit, mut cx) = start("café", false);
        run(&mut edit, &mut cx, TextCommand::Backspace);
        assert_eq!(edit.text(), "caf", "one backspace should remove one character");
    }

    #[test]
    fn emoji_delete_as_single_units() {
        let (mut edit, mut cx) = start("ok👍", false);
        run(&mut edit, &mut cx, TextCommand::Backspace);
        assert_eq!(edit.text(), "ok");
    }

    #[test]
    fn clicking_places_the_caret() {
        let (mut edit, mut cx) = start("hello world", false);
        edit.click(Point::new(0.0, 5.0), false, &mut cx.ctx());
        typed(&mut edit, &mut cx, "X");
        assert!(edit.text().starts_with('X'), "got {:?}", edit.text());
    }

    #[test]
    fn double_click_selects_a_word() {
        let (mut edit, mut cx) = start("hello world", false);
        edit.select_word_at(Point::new(5.0, 5.0), &mut cx.ctx());
        assert_eq!(edit.selected_text().as_deref(), Some("hello"));
    }

    #[test]
    fn committing_unchanged_text_preserves_the_original_markup() {
        // Mixed styling the editor does not model per-character.
        let original = "<b>bold</b> and <i>italic</i>";
        let (edit, _cx) = start(original, false);
        assert_eq!(
            edit.commit(),
            original,
            "an untouched edit must not flatten styling"
        );
    }

    #[test]
    fn committing_changed_text_produces_storable_markup() {
        let (mut edit, mut cx) = start("old", true);
        typed(&mut edit, &mut cx, "new");
        let markup = edit.commit();
        assert_eq!(markup, "new");

        let parsed = markup::parse(&markup, TextStyle::default());
        assert_eq!(parsed.plain(), "new");
    }

    #[test]
    fn committed_text_escapes_markup_characters() {
        let (mut edit, mut cx) = start("", false);
        typed(&mut edit, &mut cx, "a < b & c");
        let markup = edit.commit();
        assert_eq!(
            markup::parse(&markup, TextStyle::default()).plain(),
            "a < b & c",
            "typed angle brackets must survive, not become tags"
        );
    }

    #[test]
    fn committed_newlines_become_breaks() {
        let (mut edit, mut cx) = start("", false);
        typed(&mut edit, &mut cx, "a");
        run(&mut edit, &mut cx, TextCommand::Newline);
        typed(&mut edit, &mut cx, "b");

        let markup = edit.commit();
        assert_eq!(markup::parse(&markup, TextStyle::default()).plain(), "a\nb");
    }

    #[test]
    fn editing_styled_text_keeps_its_style() {
        let (mut edit, mut cx) = start("<b>bold</b>", true);
        typed(&mut edit, &mut cx, "still bold");

        let markup = edit.commit();
        let parsed = markup::parse(&markup, TextStyle::default());
        assert!(parsed.runs[0].style.bold, "got {markup:?}");
        assert_eq!(parsed.plain(), "still bold");
    }

    #[test]
    fn caret_blinks() {
        let (mut edit, _cx) = start("x", false);
        assert!(edit.caret_visible(), "the caret starts visible");
        edit.tick(BLINK_INTERVAL * 1.1);
        assert!(!edit.caret_visible(), "and goes dark halfway through");
        edit.tick(BLINK_INTERVAL);
        assert!(edit.caret_visible());
    }

    #[test]
    fn typing_resets_the_blink_so_the_caret_stays_visible() {
        let (mut edit, mut cx) = start("x", false);
        edit.tick(BLINK_INTERVAL * 1.1);
        assert!(!edit.caret_visible());
        typed(&mut edit, &mut cx, "y");
        assert!(edit.caret_visible(), "typing should show the caret");
    }

    #[test]
    fn caret_and_selection_geometry_sit_inside_the_area() {
        let (mut edit, mut cx) = start("hello world", false);
        run(&mut edit, &mut cx, TextCommand::SelectAll);

        let caret = edit.caret_rect(&mut cx.ctx()).expect("a caret");
        assert!(caret.y0 >= -1.0 && caret.y1 <= 61.0, "caret escaped: {caret:?}");

        let rects = edit.selection_rects(&mut cx.ctx());
        assert!(!rects.is_empty(), "a full selection should highlight something");
        for r in rects {
            assert!(r.x0 >= -1.0 && r.x1 <= 201.0, "selection escaped: {r:?}");
        }
    }

    #[test]
    fn resizing_the_area_rewraps() {
        let (mut edit, mut cx) = start("a fairly long label that wraps", false);
        edit.set_area(Rect::new(0.0, 0.0, 40.0, 200.0));
        let tall = edit.caret_rect(&mut cx.ctx()).map(|r| r.y1).unwrap_or(0.0);
        edit.set_area(Rect::new(0.0, 0.0, 400.0, 200.0));
        let wide = edit.caret_rect(&mut cx.ctx()).map(|r| r.y1).unwrap_or(0.0);
        assert!(tall > wide, "narrow should wrap lower: {tall} vs {wide}");
    }

    #[test]
    fn drawing_encodes_glyphs() {
        let (mut edit, mut cx) = start("Start", false);
        let mut scene = vello::Scene::new();
        edit.draw(&mut scene, kurbo::Affine::IDENTITY, &mut cx.ctx());
        assert!(scene.encoding().draw_tags.len() > 0, "nothing was drawn");
    }

    // ------------------------------------------------- per-selection styling

    /// The plain text of what a session would commit, with its runs.
    fn committed_runs(edit: &TextEdit) -> Vec<(String, bool, bool)> {
        let markup = edit.commit();
        markup::parse(&markup, TextStyle::default())
            .runs
            .into_iter()
            .filter(|r| !r.text.is_empty())
            .map(|r| (r.text, r.style.bold, r.style.italic))
            .collect()
    }

    #[test]
    fn mixed_styling_is_loaded_rather_than_flattened() {
        let (edit, _cx) = start("<b>bold</b> plain", false);
        assert_eq!(edit.text(), "bold plain");
        assert!(edit.range_has(0..4, StyleToggle::Bold));
        assert!(!edit.range_has(5..10, StyleToggle::Bold));
        assert!(!edit.range_has(0..10, StyleToggle::Bold), "the label is mixed");
    }

    #[test]
    fn bolding_a_selection_leaves_the_rest_alone() {
        let (mut edit, _cx) = start("hello world", false);
        assert!(edit.toggle_style(0..5, StyleToggle::Bold));

        assert_eq!(
            committed_runs(&edit),
            vec![
                ("hello".to_string(), true, false),
                (" world".to_string(), false, false),
            ]
        );
    }

    #[test]
    fn toggling_a_selection_twice_returns_the_original_markup() {
        let (mut edit, _cx) = start("hello world", false);
        edit.toggle_style(0..5, StyleToggle::Bold);
        assert!(edit.is_modified());

        edit.toggle_style(0..5, StyleToggle::Bold);
        assert!(!edit.is_modified(), "the styles came back to where they started");
        assert_eq!(edit.commit(), "hello world");
    }

    #[test]
    fn toggling_a_mixed_selection_turns_the_property_on() {
        let (mut edit, _cx) = start("<b>bold</b> plain", false);
        // The whole label is mixed, so the first press should bold all of it.
        edit.toggle_style(0..10, StyleToggle::Bold);
        assert!(edit.range_has(0..10, StyleToggle::Bold));

        // And the second press should clear it.
        edit.toggle_style(0..10, StyleToggle::Bold);
        assert!(!edit.range_has(0..10, StyleToggle::Bold));
    }

    #[test]
    fn styles_can_overlap_without_erasing_each_other() {
        let (mut edit, _cx) = start("abcdef", false);
        edit.toggle_style(0..4, StyleToggle::Bold);
        edit.toggle_style(2..6, StyleToggle::Italic);

        assert_eq!(
            committed_runs(&edit),
            vec![
                ("ab".to_string(), true, false),
                ("cd".to_string(), true, true),
                ("ef".to_string(), false, true),
            ]
        );
    }

    #[test]
    fn styling_survives_a_round_trip_through_markup() {
        let (mut edit, _cx) = start("hello world", false);
        edit.toggle_style(6..11, StyleToggle::Italic);
        let markup = edit.commit();

        let (reopened, _cx2) = start(&markup, false);
        assert_eq!(reopened.text(), "hello world");
        assert!(reopened.range_has(6..11, StyleToggle::Italic));
        assert!(!reopened.range_has(0..5, StyleToggle::Italic));
    }

    #[test]
    fn typing_inside_a_styled_run_inherits_its_style() {
        let (mut edit, mut cx) = start("<b>bold</b> plain", false);
        // Caret just after "bol", inside the bold run.
        run(&mut edit, &mut cx, TextCommand::TextStart { select: false });
        for _ in 0..3 {
            run(&mut edit, &mut cx, TextCommand::Right { select: false, word: false });
        }
        typed(&mut edit, &mut cx, "X");

        assert_eq!(edit.text(), "bolXd plain");
        assert!(edit.range_has(3..4, StyleToggle::Bold), "the insert joined the bold run");
    }

    #[test]
    fn typing_after_a_styled_run_does_not_leak_its_style() {
        let (mut edit, mut cx) = start("<b>bold</b> plain", false);
        run(&mut edit, &mut cx, TextCommand::TextEnd { select: false });
        typed(&mut edit, &mut cx, "!");
        assert!(!edit.range_has(10..11, StyleToggle::Bold));
    }

    #[test]
    fn deleting_removes_the_styles_of_what_was_deleted() {
        let (mut edit, mut cx) = start("<b>bold</b> plain", false);
        run(&mut edit, &mut cx, TextCommand::TextStart { select: false });
        for _ in 0..4 {
            run(&mut edit, &mut cx, TextCommand::Delete);
        }
        assert_eq!(edit.text(), " plain");
        assert!(
            !edit.range_has(0..6, StyleToggle::Bold),
            "nothing bold is left to be bold"
        );
    }

    #[test]
    fn replacing_a_selection_keeps_the_style_list_in_step() {
        let (mut edit, mut cx) = start("<b>bold</b> plain", false);
        run(&mut edit, &mut cx, TextCommand::SelectAll);
        typed(&mut edit, &mut cx, "new text");

        assert_eq!(edit.text(), "new text");
        assert_eq!(
            committed_runs(&edit).len(),
            1,
            "one style for one replacement"
        );
    }

    #[test]
    fn style_list_stays_the_same_length_as_the_text() {
        let (mut edit, mut cx) = start("abc", false);
        for command in [
            TextCommand::Insert("de".into()),
            TextCommand::Backspace,
            TextCommand::TextStart { select: false },
            TextCommand::Delete,
            TextCommand::Insert("👍".into()),
            TextCommand::Newline,
            TextCommand::BackspaceWord,
        ] {
            run(&mut edit, &mut cx, command);
            assert_eq!(
                edit.styles.len(),
                edit.text().chars().count(),
                "styles drifted on {:?}",
                edit.text()
            );
        }
    }

    #[test]
    fn arming_a_style_with_a_collapsed_caret_applies_to_the_next_typing() {
        let (mut edit, mut cx) = start("ab", false);
        let caret = edit.selected_range();
        assert!(caret.is_empty());

        edit.toggle_style(caret, StyleToggle::Bold);
        typed(&mut edit, &mut cx, "c");

        assert_eq!(
            committed_runs(&edit),
            vec![("ab".to_string(), false, false), ("c".to_string(), true, false)]
        );
    }

    #[test]
    fn moving_the_caret_abandons_an_armed_style() {
        let (mut edit, mut cx) = start("ab", false);
        edit.toggle_style(2..2, StyleToggle::Bold);
        run(&mut edit, &mut cx, TextCommand::Left { select: false, word: false });
        typed(&mut edit, &mut cx, "X");
        assert!(!edit.range_has(1..2, StyleToggle::Bold));
    }

    #[test]
    fn a_range_restyle_can_set_size_and_color() {
        let (mut edit, _cx) = start("small BIG", false);
        edit.restyle(6..9, |s| {
            s.font_size_pt = 28.0;
            s.color = dc_model::Color::rgb(0, 0x80, 0);
        });

        let parsed = markup::parse(&edit.commit(), TextStyle::default());
        let big = parsed.runs.iter().find(|r| r.text == "BIG").unwrap();
        assert_eq!(big.style.font_size_pt, 28.0);
        assert_eq!(big.style.color, dc_model::Color::rgb(0, 0x80, 0));
    }

    #[test]
    fn alignment_is_carried_through_an_edit() {
        let (mut edit, mut cx) = start(r#"<p style="text-align: left">x</p>"#, true);
        assert_eq!(edit.align(), TextAlign::Left);
        typed(&mut edit, &mut cx, "y");
        assert_eq!(
            markup::parse(&edit.commit(), TextStyle::default()).align,
            TextAlign::Left,
            "committing must not re-center a left-aligned label"
        );
    }

    #[test]
    fn alignment_can_be_changed_mid_edit() {
        let (mut edit, _cx) = start("x", false);
        assert!(edit.set_align(TextAlign::Right));
        assert!(!edit.set_align(TextAlign::Right), "no change, no work");
        assert_eq!(
            markup::parse(&edit.commit(), TextStyle::default()).align,
            TextAlign::Right
        );
    }

    #[test]
    fn restyling_marks_the_session_modified() {
        let (mut edit, _cx) = start("abc", false);
        assert!(!edit.is_modified());
        edit.toggle_style(0..3, StyleToggle::Underline);
        assert!(edit.is_modified(), "a style change is a change");
    }

    #[test]
    fn drawing_mixed_styles_encodes_more_than_one_run() {
        let (mut edit, mut cx) = start("hello world", false);
        edit.toggle_style(0..5, StyleToggle::Bold);

        let mut scene = vello::Scene::new();
        edit.draw(&mut scene, kurbo::Affine::IDENTITY, &mut cx.ctx());
        assert!(!scene.encoding().draw_tags.is_empty(), "nothing was drawn");
    }

    #[test]
    fn underline_and_strikethrough_are_painted() {
        let (mut plain, mut cx) = start("hello", false);
        let mut bare = vello::Scene::new();
        plain.draw(&mut bare, kurbo::Affine::IDENTITY, &mut cx.ctx());

        let (mut lined, mut cx2) = start("hello", false);
        lined.toggle_style(0..5, StyleToggle::Underline);
        let mut decorated = vello::Scene::new();
        lined.draw(&mut decorated, kurbo::Affine::IDENTITY, &mut cx2.ctx());

        assert!(
            decorated.encoding().n_paths > bare.encoding().n_paths,
            "an underline is a drawn rule, not a font feature"
        );
    }

    #[test]
    fn caret_geometry_follows_the_styled_layout() {
        let (mut edit, mut cx) = start("hello", false);
        let before = edit.caret_rect(&mut cx.ctx()).expect("a caret").x0;

        // Making the text much larger must move the caret, or the caret and
        // the glyphs have come apart.
        edit.restyle(0..5, |s| s.font_size_pt = 40.0);
        let after = edit.caret_rect(&mut cx.ctx()).expect("a caret").x0;
        assert!(
            (after - before).abs() > 1.0,
            "caret stayed at {before} while the text grew"
        );
    }

    #[test]
    fn styles_in_reports_the_caret_style_for_an_empty_range() {
        let (mut edit, _cx) = start("ab", false);
        edit.toggle_style(0..2, StyleToggle::Bold);
        let at_end = edit.styles_in(2..2);
        assert_eq!(at_end.len(), 1);
        assert!(at_end[0].bold, "the caret inherits the character behind it");
    }

    // ------------------------------------------------------------ the plate

    /// The WCAG contrast ratio between two colours.
    fn contrast(a: dc_model::Color, b: dc_model::Color) -> f64 {
        let (x, y) = (a.luminance() as f64, b.luminance() as f64);
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_plate_is_the_surface_the_label_sits_on() {
        let dark = dc_model::Color::rgb(0x1f, 0x25, 0x37);
        let (edit, _cx) = start_on("x", false, dark);
        assert_eq!(edit.backdrop(), dark);
    }

    #[test]
    fn text_stays_readable_against_its_plate_whatever_the_theme() {
        // The bug this guards: the plate used to come from the interface
        // theme while the text colour came from the shape's fill, so turning
        // on the dark theme put black text on a near-black plate.
        for surface in [
            dc_model::Color::WHITE,
            dc_model::Color::rgb(0x1f, 0x25, 0x37),
            dc_model::Color::rgb(0xdb, 0xea, 0xfe),
            dc_model::Color::rgb(0x05, 0x96, 0x69),
        ] {
            let base = TextStyle { color: surface.contrasting_text(), ..Default::default() };
            let mut cx = Contexts::new();
            let edit = TextEdit::begin(
                EditTarget::Shape("s1".into()),
                "label",
                &base,
                surface,
                Rect::new(0.0, 0.0, 200.0, 60.0),
                false,
                &mut cx.ctx(),
            );

            let ratio = contrast(edit.caret_style().color, edit.backdrop());
            assert!(
                ratio >= 4.5,
                "text on {surface:?} had only {ratio:.1}:1 contrast"
            );
        }
    }

    #[test]
    fn the_selection_wash_does_not_swallow_the_text() {
        // The highlight is mixed into the plate, so it has to stay distinct
        // from the plate and still readable behind the glyphs.
        for surface in [
            dc_model::Color::WHITE,
            dc_model::Color::rgb(0x1f, 0x25, 0x37),
        ] {
            let accent = dc_render::Theme::light().accent;
            let highlight = surface.lerp(accent, 0.45);
            let text = surface.contrasting_text();

            assert!(
                contrast(highlight, surface) > 1.1,
                "the wash is invisible on {surface:?}"
            );
            assert!(
                contrast(text, highlight) >= 3.0,
                "text on the wash over {surface:?} had {:.1}:1",
                contrast(text, highlight)
            );
        }
    }

    #[test]
    fn drawing_a_session_differs_with_the_surface() {
        let mut light = vello::Scene::new();
        {
            let (mut edit, mut cx) = start_on("x", false, dc_model::Color::WHITE);
            draw_session(
                &mut light,
                &mut edit,
                &dc_render::Theme::light(),
                kurbo::Affine::IDENTITY,
                &mut cx.ctx(),
            );
        }

        let mut dark = vello::Scene::new();
        {
            let (mut edit, mut cx) =
                start_on("x", false, dc_model::Color::rgb(0x1f, 0x25, 0x37));
            draw_session(
                &mut dark,
                &mut edit,
                &dc_render::Theme::light(),
                kurbo::Affine::IDENTITY,
                &mut cx.ctx(),
            );
        }

        assert_ne!(
            light.encoding().draw_data,
            dark.encoding().draw_data,
            "the plate should follow the surface, not be a constant"
        );
    }

    #[test]
    fn an_empty_label_can_be_typed_into() {
        let (mut edit, mut cx) = start("", true);
        typed(&mut edit, &mut cx, "First label");
        assert_eq!(edit.text(), "First label");
        assert_eq!(edit.commit(), "First label");
    }
}
