//! The shapes and templates someone keeps for themselves.
//!
//! Both live beside the settings file rather than inside it: a saved
//! template carries a whole document, and a preferences file that grows to
//! megabytes because of it would be a poor trade for the one file fewer.
//!
//! Nothing here is required. A missing or unreadable library leaves the
//! editor with the built-in shapes and templates, the same way unreadable
//! settings leave it with the defaults.

use dc_model::{Document, ShapeType};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A template someone saved from a page of their own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedTemplate {
    pub name: String,
    pub document: Document,
}

/// Shapes starred and templates saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserLibrary {
    /// Starred shape types, by their type string so an unknown one survives
    /// a version that does not have it.
    pub favorites: Vec<String>,
    pub templates: Vec<SavedTemplate>,
}

impl UserLibrary {
    /// Reads the library, falling back to an empty one.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return UserLibrary::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("ignoring unreadable library at {}: {e}", path.display());
                UserLibrary::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => UserLibrary::default(),
            Err(e) => {
                log::warn!("could not read library at {}: {e}", path.display());
                UserLibrary::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// Beside the settings file, by the same platform convention.
    pub fn path() -> Option<PathBuf> {
        let mut path = crate::settings::Settings::path()?;
        path.set_file_name("library.json");
        Some(path)
    }

    /// The starred types, as types.
    pub fn favorite_types(&self) -> Vec<ShapeType> {
        self.favorites.iter().map(|s| ShapeType::from(s.as_str())).collect()
    }

    /// Stars a type, or unstars it if it was already there. Returns whether
    /// it is starred afterwards.
    pub fn toggle_favorite(&mut self, shape_type: &ShapeType) -> bool {
        let key = shape_type.as_str().to_string();
        match self.favorites.iter().position(|f| *f == key) {
            Some(i) => {
                self.favorites.remove(i);
                false
            }
            None => {
                self.favorites.push(key);
                true
            }
        }
    }

    /// Saves a document as a template under a name, replacing one of the
    /// same name rather than collecting duplicates.
    pub fn save_template(&mut self, name: &str, document: Document) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let entry = SavedTemplate { name: name.to_string(), document };
        match self
            .templates
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
        {
            Some(i) => self.templates[i] = entry,
            None => self.templates.push(entry),
        }
    }

    /// Forgets a saved template by index.
    pub fn remove_template(&mut self, index: usize) -> bool {
        if index >= self.templates.len() {
            return false;
        }
        self.templates.remove(index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starring_a_shape_is_a_toggle() {
        let mut library = UserLibrary::default();
        let t = ShapeType::Rectangle;

        assert!(library.toggle_favorite(&t), "first press stars it");
        assert_eq!(library.favorites, vec![t.as_str().to_string()]);
        assert!(!library.toggle_favorite(&t), "second press takes it back off");
        assert!(library.favorites.is_empty());
    }

    #[test]
    fn favorites_survive_a_type_this_build_does_not_know() {
        let library = UserLibrary {
            favorites: vec!["somethingFromLater".into()],
            ..Default::default()
        };
        let types = library.favorite_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].as_str(), "somethingFromLater");
    }

    #[test]
    fn saving_a_template_twice_under_one_name_replaces_it() {
        let mut library = UserLibrary::default();
        library.save_template("Mine", Document::default());
        library.save_template("mine", Document::default());
        assert_eq!(library.templates.len(), 1, "names match without regard to case");
        assert_eq!(library.templates[0].name, "mine");
    }

    #[test]
    fn a_template_needs_a_name() {
        let mut library = UserLibrary::default();
        library.save_template("   ", Document::default());
        assert!(library.templates.is_empty());
    }

    #[test]
    fn removing_a_template_out_of_range_does_nothing() {
        let mut library = UserLibrary::default();
        library.save_template("Mine", Document::default());
        assert!(!library.remove_template(5));
        assert!(library.remove_template(0));
        assert!(library.templates.is_empty());
    }

    #[test]
    fn a_library_round_trips_through_json() {
        let mut library = UserLibrary::default();
        library.toggle_favorite(&ShapeType::Decision);
        library.save_template("Mine", Document::default());

        let text = serde_json::to_string(&library).unwrap();
        assert_eq!(serde_json::from_str::<UserLibrary>(&text).unwrap(), library);
    }

    #[test]
    fn an_empty_file_reads_as_an_empty_library() {
        assert_eq!(
            serde_json::from_str::<UserLibrary>("{}").unwrap(),
            UserLibrary::default()
        );
    }
}
