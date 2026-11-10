//! The inline markup subset permitted in shape and label text.
//!
//! The format allows text to carry a small amount of HTML. The published
//! support table is:
//!
//! | Markup | Effect |
//! |---|---|
//! | `<br>` | line break |
//! | `<b>`, `<strong>` | bold |
//! | `<i>`, `<em>` | italic |
//! | `<s>`, `<strike>` | strike-through |
//! | `<u>` | underline |
//! | `<a>` | hyperlink |
//! | `<ol>`, `<ul>`, `<li>` | lists |
//!
//! Block attributes (`color`, `font-family`, `font-size`, `font-style`,
//! `font-weight`, `text-align`, `text-decoration`, `vertical-align`) arrive as
//! inline CSS on a wrapping element.
//!
//! # Scope
//!
//! Character styling, line breaks, alignment, hyperlinks, and flat lists are
//! parsed. Nested lists collapse to a single level, and CSS beyond the
//! attributes named above is ignored. Plain text with no markup is the common
//! case and costs a single scan.

use crate::color::Color;
use crate::style::{TextAlign, TextStyle, VerticalAlign};

/// A span of text sharing one style.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub style: TextStyle,
    /// The target of a surrounding `<a>`, if any.
    pub link: Option<String>,
}

/// Text after markup resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedText {
    pub runs: Vec<TextRun>,
    pub align: TextAlign,
    pub vertical_align: VerticalAlign,
}

impl ParsedText {
    /// The text with all markup removed.
    pub fn plain(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.is_empty())
    }
}

/// Parses text that may contain the permitted markup, resolving every run
/// against `base`.
pub fn parse(input: &str, base: TextStyle) -> ParsedText {
    // The overwhelmingly common case is plain text; skip the parser entirely.
    if !input.contains('<') && !input.contains('&') {
        return ParsedText {
            runs: vec![TextRun { text: input.to_string(), style: base.clone(), link: None }],
            align: base.align,
            vertical_align: base.vertical_align,
        };
    }

    let mut parser = Parser {
        base: base.clone(),
        stack: vec![Frame { style: base.clone(), link: None }],
        runs: Vec::new(),
        pending: String::new(),
        align: base.align,
        vertical_align: base.vertical_align,
        list_depth: 0,
        pending_bullet: false,
    };
    parser.run(input);
    parser.flush();

    if parser.runs.is_empty() {
        parser.runs.push(TextRun { text: String::new(), style: base, link: None });
    }

    ParsedText {
        runs: parser.runs,
        align: parser.align,
        vertical_align: parser.vertical_align,
    }
}

/// Writes parsed text back to the format's markup subset.
///
/// Plain text with default styling is emitted as plain text, so a document
/// whose labels were never formatted stays readable and does not grow markup
/// it did not have. Anything else is wrapped in a `<p>` carrying the block
/// attributes, with character styling applied per run.
pub fn to_markup(parsed: &ParsedText, base: &TextStyle) -> String {
    let needs_block = parsed.align != base.align || parsed.vertical_align != base.vertical_align;
    let needs_runs = parsed
        .runs
        .iter()
        .any(|r| r.link.is_some() || !same_character_style(&r.style, base));

    if !needs_block && !needs_runs {
        return escape(&plain_with_breaks(&parsed.plain()));
    }

    let mut out = String::new();
    if needs_block {
        let mut declarations = Vec::new();
        if parsed.align != base.align {
            declarations.push(format!("text-align: {}", align_name(parsed.align)));
        }
        if parsed.vertical_align != base.vertical_align {
            declarations.push(format!(
                "vertical-align: {}",
                vertical_align_name(parsed.vertical_align)
            ));
        }
        out.push_str(&format!("<p style=\"{}\">", declarations.join(";")));
    }

    for run in &parsed.runs {
        if run.text.is_empty() {
            continue;
        }
        out.push_str(&run_markup(run, base));
    }

    if needs_block {
        out.push_str("</p>");
    }
    out
}

fn run_markup(run: &TextRun, base: &TextStyle) -> String {
    let mut open = String::new();
    let mut close = String::new();

    // Properties that have no dedicated tag ride on a span.
    let mut declarations = Vec::new();
    if run.style.color != base.color {
        declarations.push(format!("color: {}", run.style.color));
    }
    if (run.style.font_size_pt - base.font_size_pt).abs() > f64::EPSILON {
        declarations.push(format!("font-size: {}pt", trim_number(run.style.font_size_pt)));
    }
    if run.style.font_family != base.font_family {
        declarations.push(format!("font-family: {}", run.style.font_family));
    }
    if !declarations.is_empty() {
        open.push_str(&format!("<span style=\"{}\">", declarations.join(";")));
        close.insert_str(0, "</span>");
    }

    if let Some(href) = &run.link {
        open.push_str(&format!("<a href=\"{}\">", escape(href)));
        close.insert_str(0, "</a>");
    }
    // Underline is implied by an anchor, so it is not repeated inside one.
    if run.style.underline && run.link.is_none() {
        open.push_str("<u>");
        close.insert_str(0, "</u>");
    }
    if run.style.strikethrough {
        open.push_str("<s>");
        close.insert_str(0, "</s>");
    }
    if run.style.italic {
        open.push_str("<i>");
        close.insert_str(0, "</i>");
    }
    if run.style.bold {
        open.push_str("<b>");
        close.insert_str(0, "</b>");
    }

    format!("{open}{}{close}", escape(&plain_with_breaks(&run.text)))
}

/// Whether two styles differ only in ways `to_markup` need not express.
fn same_character_style(a: &TextStyle, b: &TextStyle) -> bool {
    a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
        && a.color == b.color
        && a.font_family == b.font_family
        && (a.font_size_pt - b.font_size_pt).abs() <= f64::EPSILON
}

fn plain_with_breaks(text: &str) -> String {
    // Newlines are written as breaks; the escaper leaves the tag alone because
    // it runs first on the surrounding text.
    text.replace('\n', "\u{0}")
}

/// Escapes markup-significant characters, restoring the line-break marker.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{0}' => out.push_str("<br>"),
            other => out.push(other),
        }
    }
    out
}

fn trim_number(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn align_name(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
        TextAlign::Justify => "justify",
    }
}

fn vertical_align_name(a: VerticalAlign) -> &'static str {
    match a {
        VerticalAlign::Top => "top",
        VerticalAlign::Center => "middle",
        VerticalAlign::Bottom => "bottom",
    }
}

/// Applies a change to every run, returning markup ready to store.
///
/// This is what the formatting controls use: parse, mutate, re-serialize.
pub fn restyle(
    existing: &str,
    base: &TextStyle,
    mutate: impl Fn(&mut TextStyle),
) -> String {
    let mut parsed = parse(existing, base.clone());
    for run in &mut parsed.runs {
        mutate(&mut run.style);
    }
    to_markup(&parsed, base)
}

/// Applies a change to the runs covering a byte range of the plain text.
///
/// This is what formatting a selection uses. Offsets are byte offsets into
/// [`ParsedText::plain`], which is the same text an editing session holds, so a
/// caller can take them straight from its selection. Offsets that fall inside a
/// character are moved outward to the nearest boundary rather than panicking.
pub fn restyle_range(
    existing: &str,
    base: &TextStyle,
    range: std::ops::Range<usize>,
    mutate: impl Fn(&mut TextStyle),
) -> String {
    let mut parsed = parse(existing, base.clone());
    restyle_runs_in_range(&mut parsed, range, mutate);
    to_markup(&parsed, base)
}

/// Whether every run touching a byte range has a property set.
pub fn all_runs_in_range(
    existing: &str,
    base: &TextStyle,
    range: std::ops::Range<usize>,
    probe: impl Fn(&TextStyle) -> bool,
) -> bool {
    let parsed = parse(existing, base.clone());
    let mut pos = 0usize;
    let mut saw_text = false;
    for run in &parsed.runs {
        let (start, end) = (pos, pos + run.text.len());
        pos = end;
        if start.max(range.start) >= end.min(range.end) {
            continue;
        }
        saw_text = true;
        if !probe(&run.style) {
            return false;
        }
    }
    saw_text
}

/// Splits runs at the range boundaries and restyles the ones inside it.
pub fn restyle_runs_in_range(
    parsed: &mut ParsedText,
    range: std::ops::Range<usize>,
    mutate: impl Fn(&mut TextStyle),
) {
    let mut out: Vec<TextRun> = Vec::with_capacity(parsed.runs.len() + 2);
    let mut pos = 0usize;

    for run in std::mem::take(&mut parsed.runs) {
        let (start, end) = (pos, pos + run.text.len());
        pos = end;

        // Entirely outside the range, or empty: nothing to split.
        let lo = range.start.clamp(start, end);
        let hi = range.end.clamp(start, end);
        if run.text.is_empty() || lo >= hi {
            out.push(run);
            continue;
        }

        // Boundaries arrive from a text editor and are normally already on
        // character boundaries; nudging rather than panicking keeps a bad
        // offset from taking the whole document down.
        let lo = floor_boundary(&run.text, lo - start);
        let hi = ceil_boundary(&run.text, hi - start);

        for (segment, inside) in [(0..lo, false), (lo..hi, true), (hi..run.text.len(), false)] {
            if segment.is_empty() {
                continue;
            }
            let mut style = run.style.clone();
            if inside {
                mutate(&mut style);
            }
            out.push(TextRun {
                text: run.text[segment].to_string(),
                style,
                link: run.link.clone(),
            });
        }
    }

    parsed.runs = merge_adjacent(out);
}

/// Collapses neighbouring runs that ended up sharing a style, so toggling a
/// property on and off returns the text to the markup it started with.
pub fn merge_adjacent(runs: Vec<TextRun>) -> Vec<TextRun> {
    let mut out: Vec<TextRun> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(last) if last.style == run.style && last.link == run.link => {
                last.text.push_str(&run.text);
            }
            _ => out.push(run),
        }
    }
    out
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

/// Applies a change to the block-level attributes.
pub fn realign(
    existing: &str,
    base: &TextStyle,
    mutate: impl Fn(&mut ParsedText),
) -> String {
    let mut parsed = parse(existing, base.clone());
    mutate(&mut parsed);
    to_markup(&parsed, base)
}

/// Whether every run in the text has a property set, used so a toggle button
/// can show the right state and flip the right way.
pub fn all_runs(existing: &str, base: &TextStyle, probe: impl Fn(&TextStyle) -> bool) -> bool {
    let parsed = parse(existing, base.clone());
    let mut saw_text = false;
    for run in &parsed.runs {
        if run.text.is_empty() {
            continue;
        }
        saw_text = true;
        if !probe(&run.style) {
            return false;
        }
    }
    saw_text
}

#[derive(Clone)]
struct Frame {
    style: TextStyle,
    link: Option<String>,
}

struct Parser {
    base: TextStyle,
    stack: Vec<Frame>,
    runs: Vec<TextRun>,
    pending: String,
    align: TextAlign,
    vertical_align: VerticalAlign,
    list_depth: u32,
    pending_bullet: bool,
}

impl Parser {
    fn frame(&self) -> &Frame {
        self.stack.last().unwrap_or_else(|| unreachable!("stack is never emptied"))
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let frame = self.frame().clone();
        let text = std::mem::take(&mut self.pending);
        // Merge into the previous run when the style matches, which keeps the
        // run list short for text that opens and closes tags repeatedly.
        if let Some(last) = self.runs.last_mut() {
            if last.style == frame.style && last.link == frame.link {
                last.text.push_str(&text);
                return;
            }
        }
        self.runs.push(TextRun { text, style: frame.style, link: frame.link });
    }

    fn push_text(&mut self, s: &str) {
        if self.pending_bullet {
            self.pending_bullet = false;
            // A flat bullet; nested lists are not tracked beyond indentation.
            let indent = "    ".repeat(self.list_depth.saturating_sub(1) as usize);
            self.pending.push_str(&indent);
            self.pending.push_str("• ");
        }
        self.pending.push_str(s);
    }

    fn run(&mut self, input: &str) {
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'<' => {
                    // An unterminated '<' is literal text.
                    let Some(close) = input[i..].find('>').map(|o| i + o) else {
                        self.push_text(&input[i..]);
                        break;
                    };
                    let raw = &input[i + 1..close];
                    self.handle_tag(raw);
                    i = close + 1;
                }
                b'&' => {
                    let (decoded, len) = decode_entity(&input[i..]);
                    self.push_text(&decoded);
                    i += len;
                }
                _ => {
                    // Consume a plain run in one go.
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'<' && bytes[i] != b'&' {
                        i += 1;
                    }
                    let chunk = input[start..i].to_string();
                    self.push_text(&chunk);
                }
            }
        }
    }

    fn handle_tag(&mut self, raw: &str) {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('!') {
            return;
        }

        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/');
        let name_end = body
            .find(|c: char| c.is_whitespace())
            .unwrap_or(body.len());
        let name = body[..name_end].to_ascii_lowercase();
        let attrs = &body[name_end..];

        if closing {
            self.close_tag(&name);
            return;
        }

        match name.as_str() {
            // Void elements that only break the line.
            "br" => {
                self.push_text("\n");
            }
            "b" | "strong" => self.open(|s| s.bold = true, None),
            "i" | "em" => self.open(|s| s.italic = true, None),
            "u" => self.open(|s| s.underline = true, None),
            "s" | "strike" | "del" => self.open(|s| s.strikethrough = true, None),
            "a" => {
                let href = attr_value(attrs, "href");
                self.open(|s| s.underline = true, href);
            }
            "ul" | "ol" => {
                self.flush();
                if !self.pending.is_empty() || !self.runs.is_empty() {
                    self.push_text("\n");
                }
                self.list_depth += 1;
                self.push_frame();
            }
            "li" => {
                self.flush();
                if self.ends_with_content() {
                    self.push_text("\n");
                }
                self.pending_bullet = true;
                self.push_frame();
            }
            // Block containers carry the alignment and base style attributes.
            "p" | "div" | "span" | "font" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let style_attr = attr_value(attrs, "style").unwrap_or_default();
                let (align, valign) = block_alignment(&style_attr);
                if let Some(a) = align {
                    self.align = a;
                }
                if let Some(v) = valign {
                    self.vertical_align = v;
                }
                // Headings are bold and larger by convention.
                let heading_scale = match name.as_str() {
                    "h1" => Some(2.0),
                    "h2" => Some(1.5),
                    "h3" => Some(1.25),
                    "h4" => Some(1.1),
                    _ => None,
                };
                self.flush();
                let mut frame = self.frame().clone();
                apply_css(&mut frame.style, &style_attr);
                if let Some(scale) = heading_scale {
                    frame.style.font_size_pt = self.base.font_size_pt * scale;
                    frame.style.bold = true;
                }
                self.stack.push(frame);
            }
            // Anything else is ignored but still balanced by its closing tag.
            _ => self.push_frame(),
        }
    }

    fn ends_with_content(&self) -> bool {
        if !self.pending.is_empty() {
            return !self.pending.ends_with('\n');
        }
        self.runs
            .last()
            .map(|r| !r.text.is_empty() && !r.text.ends_with('\n'))
            .unwrap_or(false)
    }

    fn open(&mut self, mutate: impl FnOnce(&mut TextStyle), link: Option<String>) {
        self.flush();
        let mut frame = self.frame().clone();
        mutate(&mut frame.style);
        if link.is_some() {
            frame.link = link;
        }
        self.stack.push(frame);
    }

    fn push_frame(&mut self) {
        let frame = self.frame().clone();
        self.stack.push(frame);
    }

    fn close_tag(&mut self, name: &str) {
        match name {
            // Void elements never pushed a frame.
            "br" => return,
            "ul" | "ol" => {
                self.list_depth = self.list_depth.saturating_sub(1);
            }
            _ => {}
        }
        self.flush();
        // Never pop the base frame, so malformed markup cannot underflow.
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
}

/// Reads an attribute value, accepting single or double quotes.
fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        // Must be a whole attribute name, not a suffix of another.
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = &attrs[at + name.len()..];
        let trimmed = rest.trim_start();
        if before_ok && trimmed.starts_with('=') {
            let value = trimmed[1..].trim_start();
            let quote = value.chars().next()?;
            return if quote == '"' || quote == '\'' {
                value[1..].find(quote).map(|end| value[1..=end].to_string())
            } else {
                let end = value
                    .find(char::is_whitespace)
                    .unwrap_or(value.len());
                Some(value[..end].to_string())
            };
        }
        from = at + name.len();
    }
    None
}

/// Extracts the block-level alignment declarations from an inline style.
fn block_alignment(css: &str) -> (Option<TextAlign>, Option<VerticalAlign>) {
    let mut align = None;
    let mut valign = None;
    for (prop, value) in css_declarations(css) {
        match prop.as_str() {
            "text-align" => {
                align = match value.as_str() {
                    "left" | "start" => Some(TextAlign::Left),
                    "center" => Some(TextAlign::Center),
                    "right" | "end" => Some(TextAlign::Right),
                    "justify" => Some(TextAlign::Justify),
                    _ => None,
                }
            }
            "vertical-align" => {
                valign = match value.as_str() {
                    "top" => Some(VerticalAlign::Top),
                    "middle" | "center" => Some(VerticalAlign::Center),
                    "bottom" => Some(VerticalAlign::Bottom),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    (align, valign)
}

/// Applies the character-level style declarations from an inline style.
fn apply_css(style: &mut TextStyle, css: &str) {
    for (prop, value) in css_declarations(css) {
        match prop.as_str() {
            "color" => {
                if let Some(c) = parse_css_color(&value) {
                    style.color = c;
                }
            }
            "font-family" => {
                // Take the first family and drop quoting.
                let first = value.split(',').next().unwrap_or(&value);
                let cleaned = first.trim().trim_matches('"').trim_matches('\'').trim();
                if !cleaned.is_empty() {
                    style.font_family = cleaned.to_string();
                }
            }
            "font-size" => {
                if let Some(pt) = parse_font_size(&value) {
                    style.font_size_pt = pt;
                }
            }
            "font-style" => style.italic = value == "italic" || value == "oblique",
            "font-weight" => {
                style.bold = value == "bold"
                    || value == "bolder"
                    || value.parse::<u32>().is_ok_and(|w| w >= 600);
            }
            "text-decoration" | "text-decoration-line" => {
                style.underline = value.contains("underline");
                style.strikethrough = value.contains("line-through");
            }
            _ => {}
        }
    }
}

fn css_declarations(css: &str) -> impl Iterator<Item = (String, String)> + '_ {
    css.split(';').filter_map(|decl| {
        let (prop, value) = decl.split_once(':')?;
        let prop = prop.trim().to_ascii_lowercase();
        let value = value.trim().to_ascii_lowercase();
        (!prop.is_empty() && !value.is_empty()).then_some((prop, value))
    })
}

/// Parses a CSS length into points. Pixels convert at 96 per inch.
fn parse_font_size(value: &str) -> Option<f64> {
    let v = value.trim();
    let (number, unit) = v.split_at(v.find(|c: char| c.is_alphabetic() || c == '%').unwrap_or(v.len()));
    let n: f64 = number.trim().parse().ok()?;
    let pt = match unit.trim() {
        "pt" | "" => n,
        "px" => n * 0.75,
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "em" | "rem" => n * 10.0,
        "%" => n / 100.0 * 10.0,
        _ => return None,
    };
    (pt > 0.0 && pt.is_finite()).then_some(pt)
}

/// Parses a hex color or one of the CSS named colors the format's own examples
/// use. Unknown names are left to the caller's default.
fn parse_css_color(value: &str) -> Option<Color> {
    let v = value.trim();
    if v.starts_with('#') {
        return Color::parse(v).ok();
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<u8> = inner
            .split(',')
            .filter_map(|p| p.trim().parse::<f64>().ok())
            .map(|n| n.clamp(0.0, 255.0) as u8)
            .collect();
        if parts.len() == 3 {
            return Some(Color::rgb(parts[0], parts[1], parts[2]));
        }
    }
    let named = match v {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" | "aqua" => (0, 255, 255),
        "magenta" | "fuchsia" => (255, 0, 255),
        "gray" | "grey" => (128, 128, 128),
        "silver" => (192, 192, 192),
        "maroon" => (128, 0, 0),
        "olive" => (128, 128, 0),
        "navy" => (0, 0, 128),
        "purple" => (128, 0, 128),
        "teal" => (0, 128, 128),
        "orange" => (255, 165, 0),
        "transparent" => return Some(Color::TRANSPARENT),
        _ => return None,
    };
    Some(Color::rgb(named.0, named.1, named.2))
}

/// Decodes one HTML entity, returning the text and how many bytes it consumed.
fn decode_entity(input: &str) -> (String, usize) {
    let Some(semi) = input[..input.len().min(12)].find(';') else {
        return ("&".to_string(), 1);
    };
    let name = &input[1..semi];
    let decoded = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        other => other
            .strip_prefix('#')
            .and_then(|digits| {
                if let Some(hex) = digits.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    digits.parse::<u32>().ok()
                }
            })
            .and_then(char::from_u32),
    };
    match decoded {
        Some(c) => (c.to_string(), semi + 1),
        // Not an entity after all; the ampersand is literal.
        None => ("&".to_string(), 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> TextStyle {
        TextStyle::default()
    }

    #[test]
    fn plain_text_is_one_run() {
        let p = parse("Shape title", base());
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.plain(), "Shape title");
        assert_eq!(p.align, TextAlign::Center, "defaults are preserved");
    }

    #[test]
    fn published_formatted_example_parses() {
        // Verbatim from the published reference.
        let p = parse(
            r#"<p style="font-size: 5pt;text-align: left;color: green">Formatted text</p>"#,
            base(),
        );
        assert_eq!(p.plain(), "Formatted text");
        assert_eq!(p.align, TextAlign::Left);
        assert_eq!(p.runs[0].style.font_size_pt, 5.0);
        assert_eq!(p.runs[0].style.color, Color::rgb(0, 128, 0), "css green is #008000");
    }

    #[test]
    fn every_published_markup_tag_is_handled() {
        let p = parse("<b>b</b><strong>s</strong><i>i</i><em>e</em><u>u</u><s>x</s><strike>y</strike>", base());
        assert_eq!(p.plain(), "bsieuxy");

        let styles: Vec<&TextStyle> = p.runs.iter().map(|r| &r.style).collect();
        assert!(styles.iter().any(|s| s.bold));
        assert!(styles.iter().any(|s| s.italic));
        assert!(styles.iter().any(|s| s.underline));
        assert!(styles.iter().any(|s| s.strikethrough));
    }

    #[test]
    fn br_becomes_a_newline() {
        let p = parse("first<br>second", base());
        assert_eq!(p.plain(), "first\nsecond");
        assert_eq!(p.runs.len(), 1, "a break does not split the style");
    }

    #[test]
    fn self_closing_br_works_too() {
        assert_eq!(parse("a<br/>b", base()).plain(), "a\nb");
        assert_eq!(parse("a<br />b", base()).plain(), "a\nb");
    }

    #[test]
    fn nested_tags_compose_styles() {
        let p = parse("<b>bold <i>and italic</i></b>", base());
        let italic_run = p.runs.iter().find(|r| r.text.contains("and italic")).unwrap();
        assert!(italic_run.style.bold && italic_run.style.italic);
    }

    #[test]
    fn anchors_capture_their_href() {
        let p = parse(r#"see <a href="https://example.com">the docs</a>"#, base());
        let link_run = p.runs.iter().find(|r| r.link.is_some()).unwrap();
        assert_eq!(link_run.text, "the docs");
        assert_eq!(link_run.link.as_deref(), Some("https://example.com"));
        assert!(link_run.style.underline);
    }

    #[test]
    fn single_quoted_attributes_work() {
        let p = parse("<a href='http://x.test'>x</a>", base());
        assert_eq!(p.runs[0].link.as_deref(), Some("http://x.test"));
    }

    #[test]
    fn lists_become_bulleted_lines() {
        let p = parse("<ul><li>one</li><li>two</li></ul>", base());
        let text = p.plain();
        assert!(text.contains("• one"), "got {text:?}");
        assert!(text.contains("• two"), "got {text:?}");
        assert!(text.contains('\n'), "list items are on separate lines");
    }

    #[test]
    fn entities_are_decoded() {
        let p = parse("a &amp; b &lt;c&gt; &quot;d&quot; &#65; &#x42;", base());
        assert_eq!(p.plain(), "a & b <c> \"d\" A B");
    }

    #[test]
    fn stray_ampersand_stays_literal() {
        assert_eq!(parse("Tom & Jerry", base()).plain(), "Tom & Jerry");
        assert_eq!(parse("a &notreal; b", base()).plain(), "a &notreal; b");
    }

    #[test]
    fn unterminated_tag_is_treated_as_text() {
        assert_eq!(parse("5 < 10", base()).plain(), "5 < 10");
    }

    #[test]
    fn unbalanced_close_tags_do_not_panic() {
        let p = parse("</b></b></i>text", base());
        assert_eq!(p.plain(), "text");
    }

    #[test]
    fn unknown_tags_are_stripped_but_content_kept() {
        let p = parse("<marquee>keep me</marquee>", base());
        assert_eq!(p.plain(), "keep me");
    }

    #[test]
    fn font_sizes_convert_to_points() {
        assert_eq!(parse_font_size("12pt"), Some(12.0));
        assert_eq!(parse_font_size("16px"), Some(12.0));
        assert_eq!(parse_font_size("1in"), Some(72.0));
        assert_eq!(parse_font_size("14"), Some(14.0), "a bare number is points");
        assert_eq!(parse_font_size("0pt"), None, "zero is not a usable size");
        assert_eq!(parse_font_size("abc"), None);
    }

    #[test]
    fn css_colors_parse_in_each_accepted_form() {
        assert_eq!(parse_css_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_css_color("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_css_color("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(parse_css_color("notacolor"), None);
    }

    #[test]
    fn font_weight_numbers_are_respected() {
        let mut s = base();
        apply_css(&mut s, "font-weight: 700");
        assert!(s.bold);

        let mut s2 = base();
        apply_css(&mut s2, "font-weight: 400");
        assert!(!s2.bold);
    }

    #[test]
    fn font_family_list_takes_the_first_entry() {
        let mut s = base();
        apply_css(&mut s, r#"font-family: "Helvetica Neue", Arial, sans-serif"#);
        assert_eq!(s.font_family, "helvetica neue");
    }

    #[test]
    fn vertical_align_is_read_from_the_block() {
        let p = parse(r#"<div style="vertical-align: top">x</div>"#, base());
        assert_eq!(p.vertical_align, VerticalAlign::Top);
    }

    #[test]
    fn adjacent_same_style_runs_merge() {
        let p = parse("<b>a</b><b>b</b>", base());
        assert_eq!(p.runs.len(), 1, "got {:?}", p.runs);
        assert_eq!(p.plain(), "ab");
    }

    #[test]
    fn empty_input_produces_one_empty_run() {
        let p = parse("", base());
        assert_eq!(p.runs.len(), 1);
        assert!(p.is_empty());
    }

    #[test]
    fn deeply_nested_markup_terminates() {
        let input = "<b>".repeat(500) + "x" + &"</b>".repeat(500);
        let p = parse(&input, base());
        assert_eq!(p.plain(), "x");
    }

    // ------------------------------------------------------------ serializing

    /// Parse, serialize, parse again; the two parses must agree.
    fn round_trip(input: &str) {
        let first = parse(input, base());
        let markup = to_markup(&first, &base());
        let second = parse(&markup, base());
        assert_eq!(
            first.plain(),
            second.plain(),
            "text changed: {input:?} -> {markup:?}"
        );
        assert_eq!(first.align, second.align, "align changed via {markup:?}");
        assert_eq!(
            first.vertical_align, second.vertical_align,
            "vertical align changed via {markup:?}"
        );

        let styles = |p: &ParsedText| {
            p.runs
                .iter()
                .filter(|r| !r.text.is_empty())
                .map(|r| (r.text.clone(), r.style.clone(), r.link.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(styles(&first), styles(&second), "styling changed via {markup:?}");
    }

    #[test]
    fn plain_text_serializes_without_adding_markup() {
        let p = parse("Approved?", base());
        assert_eq!(to_markup(&p, &base()), "Approved?");
    }

    #[test]
    fn markup_round_trips() {
        for input in [
            "plain",
            "<b>bold</b>",
            "<i>italic</i>",
            "<u>under</u>",
            "<s>struck</s>",
            "<b><i>both</i></b>",
            "normal <b>bold</b> normal",
            "line<br>break",
            r#"<p style="text-align: left">left</p>"#,
            r#"<p style="text-align: right">right</p>"#,
            r#"<div style="vertical-align: top">top</div>"#,
            r#"<span style="font-size: 18pt">big</span>"#,
            r#"<span style="color: #ff0000">red</span>"#,
            r#"<a href="https://example.com">link</a>"#,
            "",
        ] {
            round_trip(input);
        }
    }

    #[test]
    fn special_characters_are_escaped_on_write() {
        let p = parse("a &amp; b &lt;tag&gt;", base());
        let markup = to_markup(&p, &base());
        assert!(markup.contains("&amp;"), "got {markup:?}");
        assert!(markup.contains("&lt;"), "got {markup:?}");
        // And survive another pass.
        assert_eq!(parse(&markup, base()).plain(), "a & b <tag>");
    }

    #[test]
    fn literal_angle_brackets_survive_a_round_trip() {
        // Typed by a user, not markup.
        let typed = "if x < y && y > z";
        let p = ParsedText {
            runs: vec![TextRun { text: typed.into(), style: base(), link: None }],
            align: base().align,
            vertical_align: base().vertical_align,
        };
        let markup = to_markup(&p, &base());
        assert_eq!(parse(&markup, base()).plain(), typed);
    }

    #[test]
    fn newlines_become_breaks() {
        let p = ParsedText {
            runs: vec![TextRun { text: "a\nb".into(), style: base(), link: None }],
            align: base().align,
            vertical_align: base().vertical_align,
        };
        let markup = to_markup(&p, &base());
        assert_eq!(markup, "a<br>b");
        assert_eq!(parse(&markup, base()).plain(), "a\nb");
    }

    #[test]
    fn restyle_applies_to_every_run() {
        let bolded = restyle("normal <i>italic</i>", &base(), |s| s.bold = true);
        assert!(all_runs(&bolded, &base(), |s| s.bold), "got {bolded:?}");
        // And the italic run keeps its italic.
        let parsed = parse(&bolded, base());
        assert!(parsed.runs.iter().any(|r| r.style.italic && r.style.bold));
        assert_eq!(parsed.plain(), "normal italic");
    }

    #[test]
    fn restyle_can_turn_a_property_off_again() {
        let on = restyle("text", &base(), |s| s.bold = true);
        assert!(all_runs(&on, &base(), |s| s.bold));

        let off = restyle(&on, &base(), |s| s.bold = false);
        assert!(!all_runs(&off, &base(), |s| s.bold));
        assert_eq!(off, "text", "turning everything off returns to plain text");
    }

    #[test]
    fn realign_sets_block_attributes() {
        let left = realign("text", &base(), |p| p.align = TextAlign::Left);
        assert_eq!(parse(&left, base()).align, TextAlign::Left);

        let back = realign(&left, &base(), |p| p.align = TextAlign::Center);
        assert_eq!(back, "text", "returning to the default drops the wrapper");
    }

    // ------------------------------------------------------ styling a range

    #[test]
    fn restyling_a_range_leaves_the_rest_alone() {
        let out = restyle_range("hello world", &base(), 0..5, |s| s.bold = true);
        let parsed = parse(&out, base());
        assert_eq!(parsed.plain(), "hello world");

        let bold: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "hello", "got {out:?}");
        assert!(!all_runs(&out, &base(), |s| s.bold));
    }

    #[test]
    fn restyling_a_range_splits_an_existing_run() {
        // The range cuts across the boundary between two styled runs.
        let out = restyle_range("<b>abc</b>def", &base(), 2..4, |s| s.italic = true);
        let parsed = parse(&out, base());
        assert_eq!(parsed.plain(), "abcdef");

        let italic: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.italic)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(italic, "cd", "got {out:?}");

        // And the bold that was already there survives on both halves of it.
        let bold: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "abc", "got {out:?}");
    }

    #[test]
    fn restyling_the_whole_text_matches_restyle() {
        let text = "normal <i>italic</i>";
        let whole = restyle(text, &base(), |s| s.bold = true);
        let plain_len = parse(text, base()).plain().len();
        let ranged = restyle_range(text, &base(), 0..plain_len, |s| s.bold = true);
        assert_eq!(whole, ranged);
    }

    #[test]
    fn restyling_a_range_off_again_returns_to_plain_text() {
        let on = restyle_range("hello world", &base(), 0..5, |s| s.bold = true);
        assert_ne!(on, "hello world");
        let off = restyle_range(&on, &base(), 0..5, |s| s.bold = false);
        assert_eq!(off, "hello world", "the split runs should merge back");
    }

    #[test]
    fn an_empty_range_changes_nothing() {
        let out = restyle_range("hello", &base(), 3..3, |s| s.bold = true);
        assert_eq!(out, "hello");
    }

    #[test]
    fn a_range_past_the_end_is_clamped() {
        let out = restyle_range("abc", &base(), 1..999, |s| s.bold = true);
        let parsed = parse(&out, base());
        assert_eq!(parsed.plain(), "abc");
        let bold: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "bc");
    }

    #[test]
    fn a_range_inside_a_character_does_not_panic() {
        // Offsets 1 and 2 land inside the two-byte "é".
        let out = restyle_range("café", &base(), 1..4, |s| s.bold = true);
        assert_eq!(parse(&out, base()).plain(), "café");

        let emoji = restyle_range("a👍b", &base(), 2..3, |s| s.italic = true);
        assert_eq!(parse(&emoji, base()).plain(), "a👍b");
    }

    #[test]
    fn range_probe_reports_the_selection_not_the_whole_label() {
        let text = "<b>bold</b> plain";
        assert!(all_runs_in_range(text, &base(), 0..4, |s| s.bold));
        assert!(!all_runs_in_range(text, &base(), 0..9, |s| s.bold));
        assert!(!all_runs_in_range(text, &base(), 5..10, |s| s.bold));
        assert!(
            !all_runs_in_range(text, &base(), 2..2, |s| s.bold),
            "an empty range covers nothing"
        );
    }

    #[test]
    fn a_range_can_carry_size_and_color() {
        let out = restyle_range("small BIG", &base(), 6..9, |s| {
            s.font_size_pt = 24.0;
            s.color = Color::rgb(0xff, 0, 0);
        });
        let parsed = parse(&out, base());
        let big = parsed.runs.iter().find(|r| r.text == "BIG").expect("got {out:?}");
        assert_eq!(big.style.font_size_pt, 24.0);
        assert_eq!(big.style.color, Color::rgb(0xff, 0, 0));

        let small = parsed.runs.iter().find(|r| r.text == "small ").unwrap();
        assert_eq!(small.style.font_size_pt, base().font_size_pt);
    }

    #[test]
    fn restyling_a_range_keeps_links_on_their_own_runs() {
        let text = r#"see <a href="https://example.com">the docs</a> now"#;
        let out = restyle_range(text, &base(), 0..12, |s| s.bold = true);
        let parsed = parse(&out, base());
        let link = parsed.runs.iter().find(|r| r.link.is_some()).unwrap();
        assert_eq!(link.link.as_deref(), Some("https://example.com"));
        assert_eq!(parsed.plain(), "see the docs now");
    }

    #[test]
    fn all_runs_reports_false_for_mixed_text() {
        let mixed = "<b>bold</b> plain";
        assert!(!all_runs(mixed, &base(), |s| s.bold));
        assert!(all_runs("<b>all bold</b>", &base(), |s| s.bold));
    }

    #[test]
    fn all_runs_is_false_for_empty_text() {
        assert!(!all_runs("", &base(), |s| s.bold), "nothing is not everything");
    }

    #[test]
    fn font_sizes_serialize_without_trailing_zeros() {
        let bigger = restyle("x", &base(), |s| s.font_size_pt = 18.0);
        assert!(bigger.contains("font-size: 18pt"), "got {bigger:?}");

        let fractional = restyle("x", &base(), |s| s.font_size_pt = 10.5);
        assert!(fractional.contains("font-size: 10.5pt"), "got {fractional:?}");
    }

    #[test]
    fn serialized_size_and_color_parse_back_exactly() {
        let styled = restyle("x", &base(), |s| {
            s.font_size_pt = 22.0;
            s.color = Color::rgb(0x12, 0x34, 0x56);
        });
        let parsed = parse(&styled, base());
        assert_eq!(parsed.runs[0].style.font_size_pt, 22.0);
        assert_eq!(parsed.runs[0].style.color, Color::rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn links_keep_their_href_through_a_round_trip() {
        let p = parse(r#"<a href="https://example.com/a?b=1">x</a>"#, base());
        let markup = to_markup(&p, &base());
        let back = parse(&markup, base());
        assert_eq!(back.runs[0].link.as_deref(), Some("https://example.com/a?b=1"));
    }
}
