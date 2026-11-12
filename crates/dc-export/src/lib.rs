//! Vector export: SVG and PDF.
//!
//! Both formats are written from one traversal of the document, in
//! [`scene`], so they cannot drift apart. That traversal reads the same
//! geometry and resolved styles the on-screen renderer reads, and lays text
//! out through the same Parley engine, so an exported file breaks its lines
//! where the canvas breaks them.
//!
//! Text is written as real text in both formats rather than as outlines, so
//! it stays selectable and searchable. PDF needs no font embedded to do
//! that: the three families the editor offers map onto the base fonts every
//! reader is required to have.

use dc_model::{Document, Page};
use std::path::Path;

pub mod pdf;
pub mod scene;
pub mod svg;

pub use scene::{Drawing, FontFamily, Item, Paint, Stroke};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("document has no pages")]
    NoPages,
}

/// Choices that apply to a vector export.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    /// Leave the sheet unpainted so whatever the file is placed on shows
    /// through. A PDF page still has a size; only the fill is skipped.
    pub transparent: bool,
}

/// Renders one page to an SVG file.
pub fn write_svg(
    document: &Document,
    page_index: usize,
    options: Options,
    path: impl AsRef<Path>,
) -> Result<(), Error> {
    let drawing = drawing_for(document, page_index)?;
    std::fs::write(path, svg::render(&drawing, options))?;
    Ok(())
}

/// Renders one page to a PDF file.
pub fn write_pdf(
    document: &Document,
    page_index: usize,
    options: Options,
    path: impl AsRef<Path>,
) -> Result<(), Error> {
    let drawing = drawing_for(document, page_index)?;
    std::fs::write(path, pdf::render(&drawing, options))?;
    Ok(())
}

/// Builds the drawing list for a page, falling back to the first page when
/// the index is past the end — the same forgiveness the PNG export shows.
pub fn drawing_for(document: &Document, page_index: usize) -> Result<Drawing, Error> {
    let page: &Page = document
        .pages
        .get(page_index)
        .or_else(|| document.pages.first())
        .ok_or(Error::NoPages)?;
    let mut text = dc_render::text::TextEngine::new();
    Ok(scene::build(page, &mut text))
}
