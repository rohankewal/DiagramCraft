//! Revision history, kept locally.
//!
//! Every save puts a copy of the document aside, so a change that turns out
//! to be wrong can be undone after the editor has been closed and reopened —
//! which is exactly when the undo stack is no longer any help.
//!
//! Revisions live in the application's own directory rather than beside the
//! document or inside it. Beside it would litter the person's folder with
//! files they did not make; inside it would multiply the size of a document
//! by however many times it had been saved. The cost of keeping them apart
//! is that they do not travel with the file, which is stated plainly in the
//! README rather than hidden.
//!
//! Nothing here is required. A directory that cannot be written leaves the
//! editor working exactly as it did before, with no history.

use dc_model::Document;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many revisions are kept for one document before the oldest go.
pub const DEFAULT_LIMIT: usize = 50;

/// One saved state, as listed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    /// Seconds since the Unix epoch, which is enough to order and to show.
    pub saved_at: u64,
    /// The file the revision is stored in, relative to the document's own
    /// history directory.
    pub file: String,
    /// Shapes and connectors at the time, for a line of description that
    /// says something without opening the file.
    pub shapes: usize,
    pub lines: usize,
    pub pages: usize,
}

impl Revision {
    /// A short description, for a list.
    pub fn summary(&self) -> String {
        let pages = if self.pages == 1 {
            String::new()
        } else {
            format!("{} pages, ", self.pages)
        };
        format!("{pages}{} shapes, {} connectors", self.shapes, self.lines)
    }

    /// The time, as a local date and time.
    ///
    /// Written out by hand rather than with a calendar library: a civil date
    /// from a Unix timestamp is a dozen lines, and the alternative is a
    /// dependency carried for one label.
    pub fn when(&self) -> String {
        let (y, m, d, hh, mm) = civil_from_unix(self.saved_at);
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let month = MONTHS[(m as usize).clamp(1, 12) - 1];
        format!("{d} {month} {y}, {hh:02}:{mm:02}")
    }
}

/// Splits a Unix timestamp into a civil date and time, in UTC.
///
/// The usual days-from-epoch algorithm, shifted so the leap day falls at the
/// end of the cycle and the arithmetic has no special cases.
fn civil_from_unix(seconds: u64) -> (i64, u32, u32, u32, u32) {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (hh, mm) = ((rem / 3600) as u32, ((rem % 3600) / 60) as u32);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm)
}

/// The revisions kept for one document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Index {
    /// Newest first, which is the order they are shown in.
    pub revisions: Vec<Revision>,
    /// The number the next revision file is named with.
    ///
    /// Only ever increases. Numbering by how many revisions are held would
    /// start repeating as soon as the oldest were dropped, and a new
    /// revision would quietly overwrite one still listed.
    pub next: u64,
}

/// Where histories live by default: beside the settings.
///
/// Taken as an argument everywhere below rather than read inside each
/// function, so the history can be pointed somewhere else — which is what a
/// test needs, and what a future preference would set.
pub fn default_root() -> Option<PathBuf> {
    let mut dir = crate::settings::Settings::path()?;
    dir.set_file_name("history");
    Some(dir)
}

/// The history directory for one document, under `root`.
///
/// Keyed by a hash of the document's full path, so two files with the same
/// name in different folders do not share a history.
pub fn directory_for(root: &Path, document_path: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    document_path.hash(&mut hasher);
    root.join(format!("{:016x}", hasher.finish()))
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

/// Reads the index for a document, which is empty when there is none.
pub fn load(root: &Path, document_path: &Path) -> Index {
    let dir = directory_for(root, document_path);
    match std::fs::read_to_string(index_path(&dir)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Index::default(),
    }
}

/// Puts a copy of the document aside.
///
/// Returns the revision recorded, or `None` when nothing was: an unchanged
/// document is not worth a second copy, and a history directory that cannot
/// be written is not worth refusing a save over.
pub fn record(
    root: &Path,
    document_path: &Path,
    document: &Document,
    limit: usize,
) -> std::io::Result<Option<Revision>> {
    let dir = directory_for(root, document_path);
    std::fs::create_dir_all(&dir)?;

    let body = serde_json::to_string(document)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let mut index = load(root, document_path);

    // A save that changed nothing adds nothing. Comparing the text is exact
    // and costs less than deciding what "the same" means field by field.
    if let Some(newest) = index.revisions.first() {
        if let Ok(previous) = std::fs::read_to_string(dir.join(&newest.file)) {
            if previous == body {
                return Ok(None);
            }
        }
    }

    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // The counter keeps two saves in the same second from sharing a file,
    // and keeps a file name from being reused once older ones are dropped.
    let file = format!("{saved_at}-{}.json", index.next);
    index.next += 1;
    std::fs::write(dir.join(&file), &body)?;

    let revision = Revision {
        saved_at,
        file,
        shapes: document.pages.iter().map(|p| p.shapes().len()).sum(),
        lines: document.pages.iter().map(|p| p.lines().len()).sum(),
        pages: document.pages.len(),
    };
    index.revisions.insert(0, revision.clone());

    // Oldest first out, and their files with them — an index that listed
    // revisions whose files had gone would be worse than no index.
    while index.revisions.len() > limit.max(1) {
        if let Some(dropped) = index.revisions.pop() {
            let _ = std::fs::remove_file(dir.join(&dropped.file));
        }
    }

    std::fs::write(
        index_path(&dir),
        serde_json::to_string_pretty(&index)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
    )?;
    Ok(Some(revision))
}

/// Reads one revision back.
pub fn read(root: &Path, document_path: &Path, revision: &Revision) -> Option<Document> {
    let dir = directory_for(root, document_path);
    let text = std::fs::read_to_string(dir.join(&revision.file)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forgets every revision of a document.
pub fn clear(root: &Path, document_path: &Path) -> std::io::Result<()> {
    let dir = directory_for(root, document_path);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Shape, ShapeType};

    /// A history root of its own, so tests neither touch the real one nor
    /// each other — which a global would not allow, since they run at once.
    struct Sandbox {
        dir: tempfile::TempDir,
        file: PathBuf,
    }

    impl Sandbox {
        fn new() -> Sandbox {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("diagram.lucid");
            Sandbox { dir, file }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }
    }

    fn document_with(shapes: usize) -> Document {
        let mut d = Document::new();
        for i in 0..shapes {
            d.pages[0].add_shape(Shape::new(
                format!("s{i}"),
                ShapeType::Rectangle,
                BoundingBox::new(i as f64, 0.0, 10.0, 10.0),
            ));
        }
        d
    }

    #[test]
    fn a_save_is_recorded_and_can_be_read_back() {
        let s = Sandbox::new();
        let document = document_with(3);

        let revision = record(s.root(), &s.file, &document, DEFAULT_LIMIT).unwrap().unwrap();
        assert_eq!(revision.shapes, 3);
        assert_eq!(revision.pages, 1);

        let back = read(s.root(), &s.file, &revision).expect("the revision should read back");
        assert_eq!(back.pages[0].shapes().len(), 3);
    }

    #[test]
    fn an_unchanged_save_is_not_recorded_twice() {
        let s = Sandbox::new();
        let document = document_with(2);

        assert!(record(s.root(), &s.file, &document, DEFAULT_LIMIT).unwrap().is_some());
        assert!(
            record(s.root(), &s.file, &document, DEFAULT_LIMIT).unwrap().is_none(),
            "saving again without changing anything should add nothing"
        );
        assert_eq!(load(s.root(), &s.file).revisions.len(), 1);
    }

    #[test]
    fn revisions_are_listed_newest_first() {
        let s = Sandbox::new();
        for n in 1..=3 {
            record(s.root(), &s.file, &document_with(n), DEFAULT_LIMIT).unwrap();
        }
        let index = load(s.root(), &s.file);
        assert_eq!(index.revisions.len(), 3);
        assert_eq!(index.revisions[0].shapes, 3, "the newest should be first");
        assert_eq!(index.revisions[2].shapes, 1);
    }

    #[test]
    fn the_oldest_revisions_are_dropped_past_the_limit() {
        let s = Sandbox::new();
        for n in 1..=6 {
            record(s.root(), &s.file, &document_with(n), 3).unwrap();
        }
        let index = load(s.root(), &s.file);
        assert_eq!(index.revisions.len(), 3);
        assert_eq!(index.revisions[0].shapes, 6);
        assert_eq!(index.revisions[2].shapes, 4, "the first three went");
    }

    #[test]
    fn a_dropped_revisions_file_goes_with_it() {
        let s = Sandbox::new();
        for n in 1..=4 {
            record(s.root(), &s.file, &document_with(n), 2).unwrap();
        }
        let dir = directory_for(s.root(), &s.file);
        let files = std::fs::read_dir(&dir).unwrap().count();
        // Two revisions plus the index.
        assert_eq!(files, 3, "orphaned revision files should not pile up");
    }

    #[test]
    fn two_documents_keep_separate_histories() {
        let s = Sandbox::new();
        let other = s.file.with_file_name("other.lucid");

        record(s.root(), &s.file, &document_with(1), DEFAULT_LIMIT).unwrap();
        record(s.root(), &other, &document_with(5), DEFAULT_LIMIT).unwrap();

        assert_eq!(load(s.root(), &s.file).revisions[0].shapes, 1);
        assert_eq!(load(s.root(), &other).revisions[0].shapes, 5);
    }

    #[test]
    fn clearing_forgets_everything_about_a_document() {
        let s = Sandbox::new();
        record(s.root(), &s.file, &document_with(2), DEFAULT_LIMIT).unwrap();
        assert_eq!(load(s.root(), &s.file).revisions.len(), 1);

        clear(s.root(), &s.file).unwrap();
        assert!(load(s.root(), &s.file).revisions.is_empty());
    }

    #[test]
    fn a_document_with_no_history_lists_nothing() {
        let s = Sandbox::new();
        assert!(load(s.root(), &s.file).revisions.is_empty());
    }

    #[test]
    fn a_summary_says_what_is_in_a_revision() {
        let one = Revision { saved_at: 0, file: "f".into(), shapes: 4, lines: 3, pages: 1 };
        assert_eq!(one.summary(), "4 shapes, 3 connectors");

        let many = Revision { pages: 3, ..one };
        assert_eq!(many.summary(), "3 pages, 4 shapes, 3 connectors");
    }

    #[test]
    fn a_timestamp_reads_as_a_date() {
        // 2024-02-29T13:45:00Z — a leap day, which is where date arithmetic
        // usually goes wrong.
        let r = Revision { saved_at: 1_709_214_300, file: "f".into(), shapes: 0, lines: 0, pages: 1 };
        assert_eq!(r.when(), "29 Feb 2024, 13:45");
    }

    #[test]
    fn the_epoch_itself_reads_correctly() {
        let r = Revision { saved_at: 0, file: "f".into(), shapes: 0, lines: 0, pages: 1 };
        assert_eq!(r.when(), "1 Jan 1970, 00:00");
    }
}
