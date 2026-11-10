//! The Open Packaging Conventions container a `.vsdx` file is.
//!
//! A drawing is a ZIP archive whose parts reference each other through
//! relationship files rather than by name, so reaching a page means reading
//! `visio/pages/pages.xml`, finding the relationship id it names, and looking
//! that id up in `visio/pages/_rels/pages.xml.rels`.

use crate::Error;
use std::collections::BTreeMap;
use std::io::{Read, Seek};

/// Every part of the archive, keyed by its normalized path.
pub struct Package {
    parts: BTreeMap<String, Vec<u8>>,
}

impl Package {
    pub fn read<R: Read + Seek>(reader: R) -> Result<Self, Error> {
        let mut archive = zip::ZipArchive::new(reader)?;
        let mut parts = BTreeMap::new();

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            if entry.is_dir() {
                continue;
            }
            // Entry names are attacker-controlled; normalize before storing so
            // a crafted archive cannot reach outside the package.
            let raw = entry.name()?.to_string();
            let Some(name) = normalize(&raw) else {
                log::warn!("ignoring unsafe entry {raw:?}");
                continue;
            };
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            parts.insert(name, bytes);
        }

        Ok(Package { parts })
    }

    pub fn part(&self, path: &str) -> Option<&[u8]> {
        self.parts.get(path).map(Vec::as_slice)
    }

    /// A part decoded as UTF-8 text, which every XML part in the format is.
    pub fn text(&self, path: &str) -> Option<String> {
        let bytes = self.part(path)?;
        // A byte-order mark is legal at the head of an XML part and would
        // otherwise be read as content.
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn has(&self, path: &str) -> bool {
        self.parts.contains_key(path)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }
}

/// Normalizes a ZIP entry name, rejecting anything that escapes the package.
pub fn normalize(name: &str) -> Option<String> {
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
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Resolves a relationship target against the directory of the part that
/// declared it, which is how the packaging convention addresses parts.
pub fn resolve_relative(owner: &str, target: &str) -> Option<String> {
    if let Some(absolute) = target.strip_prefix('/') {
        return normalize(absolute);
    }
    let dir = owner.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let joined = if dir.is_empty() {
        target.to_string()
    } else {
        format!("{dir}/{target}")
    };

    // `..` is meaningful here, so it is resolved rather than rejected, and only
    // a path that climbs above the package root is refused.
    let unified = joined.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for component in unified.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The path of the relationship part belonging to a part.
pub fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_normalized() {
        assert_eq!(normalize("visio/pages/page1.xml").as_deref(), Some("visio/pages/page1.xml"));
        assert_eq!(normalize("./visio//page1.xml").as_deref(), Some("visio/page1.xml"));
        assert_eq!(normalize(r"visio\pages\page1.xml").as_deref(), Some("visio/pages/page1.xml"));
    }

    #[test]
    fn traversal_entry_names_are_refused() {
        assert!(normalize("../../etc/passwd").is_none());
        assert!(normalize("/etc/passwd").is_none());
        assert!(normalize("C:/Windows").is_none());
        assert!(normalize("").is_none());
    }

    #[test]
    fn relationships_resolve_against_their_owner() {
        assert_eq!(
            resolve_relative("visio/pages/pages.xml", "page1.xml").as_deref(),
            Some("visio/pages/page1.xml")
        );
        assert_eq!(
            resolve_relative("visio/document.xml", "pages/pages.xml").as_deref(),
            Some("visio/pages/pages.xml")
        );
        assert_eq!(
            resolve_relative("visio/pages/pages.xml", "../masters/masters.xml").as_deref(),
            Some("visio/masters/masters.xml")
        );
        assert_eq!(
            resolve_relative("visio/pages/pages.xml", "/visio/document.xml").as_deref(),
            Some("visio/document.xml")
        );
    }

    #[test]
    fn a_relationship_cannot_climb_out_of_the_package() {
        assert!(resolve_relative("visio/document.xml", "../../escape.xml").is_none());
    }

    #[test]
    fn relationship_parts_sit_beside_what_they_describe() {
        assert_eq!(rels_path("visio/pages/pages.xml"), "visio/pages/_rels/pages.xml.rels");
        assert_eq!(rels_path("visio/document.xml"), "visio/_rels/document.xml.rels");
        assert_eq!(rels_path("document.xml"), "_rels/document.xml.rels");
    }
}
