//! Reader and writer for the `.lucid` interchange package.
//!
//! The package is a ZIP archive whose layout the format publishes as:
//!
//! ```text
//! import.lucid
//! ├── data
//! │   └── records.csv
//! ├── images
//! │   └── logo.png
//! └── document.json
//! ```
//!
//! `document.json` is required and capped at 2 MB. The `data` and `images`
//! directories are optional.
//!
//! Written from the published specification at
//! <https://developer.lucid.co/docs/overview-si>.

use dc_model::Document;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

pub mod data;
pub mod validate;

pub use data::DataSource;
pub use validate::{Severity, ValidationIssue};

/// The entry name of the document, fixed by the format.
pub const DOCUMENT_ENTRY: &str = "document.json";
/// The directory holding image assets.
pub const IMAGES_DIR: &str = "images";
/// The directory holding CSV data sources.
pub const DATA_DIR: &str = "data";
/// The format's stated size ceiling for `document.json`.
pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a valid zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("{DOCUMENT_ENTRY} is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("package has no {DOCUMENT_ENTRY}; it may not be a .lucid file")]
    MissingDocument,

    #[error(
        "{DOCUMENT_ENTRY} is {size} bytes, over the {MAX_DOCUMENT_BYTES} byte limit the format allows"
    )]
    DocumentTooLarge { size: usize },

    #[error("refusing entry {0:?}: path escapes the package")]
    UnsafeEntryPath(String),
}

/// A loaded `.lucid` package: the document plus its bundled assets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LucidPackage {
    pub document: Document,
    /// Image assets keyed by file name as referenced from the document, so
    /// `images/logo.png` in the archive is keyed `logo.png`.
    pub images: BTreeMap<String, Vec<u8>>,
    /// CSV data sources, keyed the same way relative to `data/`.
    pub data: BTreeMap<String, Vec<u8>>,
}

impl LucidPackage {
    pub fn new(document: Document) -> Self {
        LucidPackage { document, images: BTreeMap::new(), data: BTreeMap::new() }
    }

    pub fn validate(&self) -> Vec<ValidationIssue> {
        validate::validate(self)
    }

    /// The package's CSV data sources, in name order.
    ///
    /// A file that holds no header row is not a source and is skipped; it is
    /// still carried in `data` and still written back out, since round-trip
    /// fidelity does not depend on this build understanding a file.
    pub fn data_sources(&self) -> Vec<DataSource> {
        self.data
            .iter()
            .filter_map(|(name, bytes)| DataSource::parse(name, bytes))
            .collect()
    }

    /// One data source by entry name.
    pub fn data_source(&self, name: &str) -> Option<DataSource> {
        DataSource::parse(name, self.data.get(name)?)
    }

    /// Reads a package from a path.
    pub fn read_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let file = File::open(path)?;
        Self::read(file)
    }

    /// Reads a package from any seekable reader.
    pub fn read<R: Read + Seek>(reader: R) -> Result<Self, Error> {
        let mut archive = zip::ZipArchive::new(reader)?;

        let mut document: Option<Document> = None;
        let mut images = BTreeMap::new();
        let mut data = BTreeMap::new();

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            // Directory markers carry no payload.
            if entry.is_dir() {
                continue;
            }
            // `name` is attacker-controlled. Normalize before matching so a
            // crafted archive cannot reach outside the package.
            let raw_name = entry.name()?.to_string();
            let name = normalize_entry_path(&raw_name)
                .ok_or_else(|| Error::UnsafeEntryPath(raw_name.clone()))?;

            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;

            if name == DOCUMENT_ENTRY {
                if bytes.len() > MAX_DOCUMENT_BYTES {
                    // The ceiling binds writers; a reader that rejected an
                    // oversized document would be less useful than one that
                    // loads it and says so.
                    log::warn!(
                        "{DOCUMENT_ENTRY} is {} bytes, over the {MAX_DOCUMENT_BYTES} byte limit",
                        bytes.len()
                    );
                }
                document = Some(serde_json::from_slice(&bytes)?);
            } else if let Some(rest) = strip_dir(&name, IMAGES_DIR) {
                images.insert(rest.to_string(), bytes);
            } else if let Some(rest) = strip_dir(&name, DATA_DIR) {
                data.insert(rest.to_string(), bytes);
            } else {
                log::debug!("ignoring unrecognized package entry {name:?}");
            }
        }

        Ok(LucidPackage {
            document: document.ok_or(Error::MissingDocument)?,
            images,
            data,
        })
    }

    /// Writes the package to a path.
    pub fn write_path(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let file = File::create(path)?;
        self.write(file)
    }

    /// Writes the package to any seekable writer.
    pub fn write<W: Write + Seek>(&self, writer: W) -> Result<(), Error> {
        let json = serde_json::to_vec(&self.document)?;
        if json.len() > MAX_DOCUMENT_BYTES {
            return Err(Error::DocumentTooLarge { size: json.len() });
        }

        let mut zip = zip::ZipWriter::new(writer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file(DOCUMENT_ENTRY, options)?;
        zip.write_all(&json)?;

        for (name, bytes) in &self.images {
            zip.start_file(format!("{IMAGES_DIR}/{name}"), options)?;
            zip.write_all(bytes)?;
        }
        for (name, bytes) in &self.data {
            zip.start_file(format!("{DATA_DIR}/{name}"), options)?;
            zip.write_all(bytes)?;
        }

        zip.finish()?;
        Ok(())
    }

    /// Serializes to an in-memory package. Useful for tests and clipboards.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut buf = Cursor::new(Vec::new());
        self.write(&mut buf)?;
        Ok(buf.into_inner())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Self::read(Cursor::new(bytes))
    }
}

/// Normalizes a ZIP entry name, rejecting anything that escapes the package.
///
/// Returns `None` for absolute paths, drive-prefixed paths, and any component
/// that is `..`. Leading `./` and redundant separators are removed.
fn normalize_entry_path(name: &str) -> Option<String> {
    let unified = name.replace('\\', "/");
    if unified.starts_with('/') || unified.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// Returns the portion of `name` inside `dir`, or `None` if it is elsewhere.
fn strip_dir<'a>(name: &'a str, dir: &str) -> Option<&'a str> {
    name.strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{
        BoundingBox, Document, Endpoint, EndpointStyle, Line, LineType, Shape, ShapeType,
        Style,
    };
    use std::io::Cursor;

    fn sample_document() -> Document {
        let mut doc = Document::new();
        doc.set_units(dc_model::Units::Pt);
        let page = &mut doc.pages[0];
        page.add_shape(
            Shape::new("block1", ShapeType::Terminator, BoundingBox::new(40.0, 40.0, 120.0, 50.0))
                .with_text("Start")
                .with_style(Style::default()),
        );
        page.add_shape(
            Shape::new("block2", ShapeType::Decision, BoundingBox::new(40.0, 160.0, 120.0, 100.0))
                .with_text("Approved?"),
        );
        page.add_line(
            Line::new(
                "line1",
                Endpoint::at_shape("block1", EndpointStyle::None),
                Endpoint::at_shape("block2", EndpointStyle::Arrow),
            )
            .with_line_type(LineType::Elbow)
            .with_label("yes"),
        );
        doc
    }

    #[test]
    fn package_round_trips_through_zip() {
        let mut pkg = LucidPackage::new(sample_document());
        pkg.images.insert("logo.png".into(), vec![0x89, b'P', b'N', b'G']);
        pkg.data.insert("records.csv".into(), b"id,name\n1,ops\n".to_vec());

        let bytes = pkg.to_bytes().unwrap();
        let back = LucidPackage::from_bytes(&bytes).unwrap();

        assert_eq!(back, pkg, "document and assets both survive");
        assert_eq!(back.images["logo.png"], vec![0x89, b'P', b'N', b'G']);
        assert_eq!(back.data["records.csv"], b"id,name\n1,ops\n");
    }

    #[test]
    fn written_archive_uses_the_published_layout() {
        let mut pkg = LucidPackage::new(sample_document());
        pkg.images.insert("logo.png".into(), vec![1, 2, 3]);
        pkg.data.insert("records.csv".into(), b"a\n".to_vec());

        let bytes = pkg.to_bytes().unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();

        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["data/records.csv", "document.json", "images/logo.png"]);
    }

    #[test]
    fn missing_document_is_reported_clearly() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file("images/logo.png", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(&[1, 2, 3]).unwrap();
            zip.finish().unwrap();
        }
        let err = LucidPackage::from_bytes(&buf.into_inner()).unwrap_err();
        assert!(matches!(err, Error::MissingDocument), "got {err:?}");
    }

    #[test]
    fn non_zip_input_is_reported_as_such() {
        let err = LucidPackage::from_bytes(b"this is not a zip file at all").unwrap_err();
        assert!(matches!(err, Error::Zip(_)), "got {err:?}");
    }

    #[test]
    fn malformed_document_json_is_reported_as_such() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file(DOCUMENT_ENTRY, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"{ not json").unwrap();
            zip.finish().unwrap();
        }
        let err = LucidPackage::from_bytes(&buf.into_inner()).unwrap_err();
        assert!(matches!(err, Error::Json(_)), "got {err:?}");
    }

    #[test]
    fn oversized_document_is_refused_on_write() {
        let mut doc = Document::new();
        // Inflate past the 2 MB ceiling with a long note.
        let page = &mut doc.pages[0];
        let mut s = Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        s.note = Some("x".repeat(MAX_DOCUMENT_BYTES + 1));
        page.add_shape(s);

        let err = LucidPackage::new(doc).to_bytes().unwrap_err();
        match err {
            Error::DocumentTooLarge { size } => assert!(size > MAX_DOCUMENT_BYTES),
            other => panic!("expected DocumentTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn traversal_entry_names_are_rejected() {
        assert_eq!(normalize_entry_path("images/logo.png").as_deref(), Some("images/logo.png"));
        assert_eq!(normalize_entry_path("./document.json").as_deref(), Some("document.json"));
        assert_eq!(normalize_entry_path("images//logo.png").as_deref(), Some("images/logo.png"));

        assert!(normalize_entry_path("../../etc/passwd").is_none());
        assert!(normalize_entry_path("images/../../secret").is_none());
        assert!(normalize_entry_path("/etc/passwd").is_none());
        assert!(normalize_entry_path("C:/Windows/system32").is_none());
        assert!(normalize_entry_path("").is_none());
    }

    #[test]
    fn traversal_archive_is_refused() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file("../escape.png", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(&[1]).unwrap();
            zip.finish().unwrap();
        }
        let err = LucidPackage::from_bytes(&buf.into_inner()).unwrap_err();
        assert!(matches!(err, Error::UnsafeEntryPath(_)), "got {err:?}");
    }

    #[test]
    fn strip_dir_matches_only_that_directory() {
        assert_eq!(strip_dir("images/a.png", IMAGES_DIR), Some("a.png"));
        assert_eq!(strip_dir("images/nested/a.png", IMAGES_DIR), Some("nested/a.png"));
        assert_eq!(strip_dir("data/a.csv", IMAGES_DIR), None);
        assert_eq!(strip_dir("imagesfoo/a.png", IMAGES_DIR), None);
        assert_eq!(strip_dir("images", IMAGES_DIR), None);
    }

    #[test]
    fn unknown_entries_are_ignored_not_fatal() {
        let mut pkg = LucidPackage::new(sample_document());
        pkg.images.insert("logo.png".into(), vec![7]);
        let bytes = pkg.to_bytes().unwrap();

        // Re-pack with an extra stray entry.
        let mut out = Cursor::new(Vec::new());
        {
            let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let mut zip = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default();
            for i in 0..src.len() {
                let mut e = src.by_index(i).unwrap();
                let name = e.name().unwrap().to_string();
                let mut b = Vec::new();
                e.read_to_end(&mut b).unwrap();
                zip.start_file(name, opts).unwrap();
                zip.write_all(&b).unwrap();
            }
            zip.start_file("thumbnail.png", opts).unwrap();
            zip.write_all(&[9, 9]).unwrap();
            zip.finish().unwrap();
        }

        let back = LucidPackage::from_bytes(&out.into_inner()).unwrap();
        assert_eq!(back.document, pkg.document);
        assert_eq!(back.images["logo.png"], vec![7]);
    }

    #[test]
    fn file_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagram.lucid");

        let pkg = LucidPackage::new(sample_document());
        pkg.write_path(&path).unwrap();
        assert!(path.exists());

        let back = LucidPackage::read_path(&path).unwrap();
        assert_eq!(back.document, pkg.document);
    }
}
