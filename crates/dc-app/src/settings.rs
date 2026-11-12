//! Application preferences.
//!
//! These are the choices that outlive a document: how the grid looks, what
//! snaps to what, which way new connectors route, and what a new document
//! starts as. They are written to a file beside the user's other application
//! data and read back at launch, so the editor comes up the way it was left.
//!
//! The grouping follows the one Lucidchart's document settings use — canvas,
//! grid and snapping, lines, zoom — because those are the headings someone
//! arriving from that product will look under. Anything in that product's
//! settings this build cannot honour is absent rather than present and inert.
//!
//! Settings this program does not recognize are kept and written back, so a
//! file written by a newer build survives being opened by an older one.

use dc_model::{LineType, Units};
use dc_render::GridStyle;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;

/// The grid pitches offered, in points. Nine points is an eighth of an inch.
pub const GRID_PITCHES: [f64; 4] = [4.5, 9.0, 18.0, 36.0];
/// How close a drag must come before it snaps, in screen pixels.
pub const SNAP_DISTANCES: [f64; 4] = [3.0, 6.0, 10.0, 16.0];
/// Zoom ratios per step.
pub const ZOOM_STEPS: [f64; 3] = [1.1, 1.25, 1.5];
/// The page sizes offered to new documents.
pub const PAGE_SIZES: [&str; 5] = ["letter", "legal", "tabloid", "a3", "a4"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    // ------------------------------------------------------- appearance
    pub dark_theme: bool,

    // -------------------------------------------------- grid and snapping
    pub show_grid: bool,
    pub grid_style: GridStyle,
    /// Grid pitch in points.
    pub grid_spacing: f64,
    /// Whether a drag aligns to the edges and centres of other shapes.
    pub snap_to_objects: bool,
    /// Whether a drag with nothing to align to falls back to the grid.
    pub snap_to_grid: bool,
    /// How close a drag must come before it snaps, in screen pixels.
    pub snap_distance: f64,

    // ------------------------------------------------------------ canvas
    pub show_page_bounds: bool,
    /// Whether rulers are drawn along the top and left of the canvas.
    pub show_rulers: bool,
    /// Zoom ratio applied per zoom step.
    pub zoom_step: f64,

    // ----------------------------------------------------------- lines
    /// The routing a newly drawn connector takes.
    pub default_line_type: LineType,
    /// Whether connectors steer around unrelated shapes.
    pub route_around_shapes: bool,

    // -------------------------------------------------- new documents
    pub default_units: Units,
    pub default_page_size: String,

    /// Settings this build does not recognize, kept so a file written by a
    /// newer one is not quietly stripped.
    #[serde(flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark_theme: false,
            show_grid: true,
            grid_style: GridStyle::Graph,
            grid_spacing: 9.0,
            snap_to_objects: true,
            snap_to_grid: true,
            snap_distance: 6.0,
            show_page_bounds: true,
            show_rulers: false,
            zoom_step: 1.25,
            default_line_type: LineType::Elbow,
            route_around_shapes: true,
            default_units: Units::In,
            default_page_size: "letter".to_string(),
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// Reads the saved settings, falling back to the defaults.
    ///
    /// A missing file is the ordinary first-run case. A corrupt one is worth
    /// saying something about, but not worth refusing to start over: the
    /// defaults are always usable.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Settings::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(settings) => settings.sanitized(),
                Err(e) => {
                    log::warn!("ignoring unreadable settings at {}: {e}", path.display());
                    Settings::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                log::warn!("could not read settings at {}: {e}", path.display());
                Settings::default()
            }
        }
    }

    /// Writes the settings out, creating the directory if it is not there.
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

    /// Where the settings file lives, by the platform's convention.
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);

        if cfg!(target_os = "macos") {
            return Some(
                home?
                    .join("Library/Application Support/DiagramCraft/settings.json"),
            );
        }
        if cfg!(target_os = "windows") {
            return Some(
                PathBuf::from(std::env::var_os("APPDATA")?)
                    .join("DiagramCraft")
                    .join("settings.json"),
            );
        }
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => home?.join(".config"),
        };
        Some(base.join("diagramcraft").join("settings.json"))
    }

    /// Clamps anything a hand-edited file could have put out of range.
    ///
    /// A zero grid pitch or a zoom step of one would leave the editor stuck,
    /// so a nonsensical value falls back to the default rather than being
    /// honoured.
    pub fn sanitized(mut self) -> Self {
        let defaults = Settings::default();
        if !(0.5..=720.0).contains(&self.grid_spacing) {
            self.grid_spacing = defaults.grid_spacing;
        }
        if !(0.0..=64.0).contains(&self.snap_distance) {
            self.snap_distance = defaults.snap_distance;
        }
        if !(1.01..=4.0).contains(&self.zoom_step) {
            self.zoom_step = defaults.zoom_step;
        }
        if self.default_page_size.trim().is_empty() {
            self.default_page_size = defaults.default_page_size;
        }
        self
    }
}

/// The display name of a page size.
pub fn page_size_label(name: &str) -> String {
    match name {
        "letter" => "Letter".into(),
        "legal" => "Legal".into(),
        "tabloid" => "Tabloid".into(),
        "executive" => "Executive".into(),
        "folio" => "Folio".into(),
        other => {
            // Anything else is shown as written, with its first letter raised.
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + chars.as_str()
                }
                None => String::new(),
            }
        }
    }
}

/// A readable name for a measurement unit.
pub fn units_label(units: Units) -> &'static str {
    match units {
        Units::In => "Inches",
        Units::Cm => "Centimetres",
        Units::Pt => "Points",
        Units::Px => "Pixels",
    }
}

/// A readable name for a connector routing.
pub fn line_type_label(line_type: LineType) -> &'static str {
    match line_type {
        LineType::Elbow => "Elbow",
        LineType::Straight => "Straight",
        LineType::Curved => "Curved",
    }
}

/// Formats a length in points for a settings label.
pub fn pitch_label(points: f64) -> String {
    if (points - points.round()).abs() < 0.01 {
        format!("{} pt", points.round() as i64)
    } else {
        format!("{points} pt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_ones_the_editor_shipped_with() {
        let s = Settings::default();
        assert!(s.show_grid);
        assert!(s.snap_to_objects);
        assert!(s.snap_to_grid);
        assert!(!s.dark_theme);
        assert_eq!(s.grid_spacing, 9.0, "an eighth of an inch");
        assert_eq!(s.snap_distance, 6.0);
        assert_eq!(s.zoom_step, 1.25);
        assert_eq!(s.default_line_type, LineType::Elbow);
        assert_eq!(s.default_units, Units::In);
        assert_eq!(s.default_page_size, "letter");
    }

    #[test]
    fn every_offered_value_is_one_the_defaults_could_take() {
        assert!(GRID_PITCHES.contains(&Settings::default().grid_spacing));
        assert!(SNAP_DISTANCES.contains(&Settings::default().snap_distance));
        assert!(ZOOM_STEPS.contains(&Settings::default().zoom_step));
        assert!(PAGE_SIZES.contains(&Settings::default().default_page_size.as_str()));
    }

    #[test]
    fn settings_round_trip_through_json() {
        let s = Settings {
            dark_theme: true,
            grid_style: GridStyle::Dots,
            grid_spacing: 18.0,
            default_line_type: LineType::Curved,
            default_units: Units::Cm,
            default_page_size: "a4".into(),
            ..Default::default()
        };

        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn a_partial_file_fills_in_the_rest_from_the_defaults() {
        let s: Settings = serde_json::from_str(r#"{"darkTheme":true}"#).unwrap();
        assert!(s.dark_theme);
        assert_eq!(s.grid_spacing, Settings::default().grid_spacing);
        assert!(s.show_grid);
    }

    #[test]
    fn an_empty_file_is_just_the_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn settings_from_a_newer_build_are_kept_not_stripped() {
        let s: Settings =
            serde_json::from_str(r#"{"darkTheme":true,"somethingNewer":{"a":1}}"#).unwrap();
        assert!(s.extra.contains_key("somethingNewer"));

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["somethingNewer"]["a"], 1);
    }

    #[test]
    fn nonsense_values_fall_back_rather_than_wedging_the_editor() {
        let hostile: Settings = serde_json::from_str(
            r#"{"gridSpacing":0,"snapDistance":-5,"zoomStep":1,"defaultPageSize":"  "}"#,
        )
        .unwrap();
        let s = hostile.sanitized();

        assert_eq!(s.grid_spacing, 9.0, "a zero pitch would divide by zero");
        assert_eq!(s.snap_distance, 6.0);
        assert_eq!(s.zoom_step, 1.25, "a step of one would never zoom");
        assert_eq!(s.default_page_size, "letter");
    }

    #[test]
    fn sane_values_are_left_alone_by_sanitizing() {
        let s = Settings {
            grid_spacing: 36.0,
            snap_distance: 16.0,
            zoom_step: 1.5,
            ..Default::default()
        };
        assert_eq!(s.clone().sanitized(), s);
    }

    #[test]
    fn the_settings_path_is_under_the_users_own_directory() {
        let Some(path) = Settings::path() else {
            return;
        };
        assert!(path.ends_with("settings.json"), "{}", path.display());
        assert!(path.is_absolute(), "{}", path.display());
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(units_label(Units::In), "Inches");
        assert_eq!(line_type_label(LineType::Elbow), "Elbow");
        assert_eq!(page_size_label("letter"), "Letter");
        assert_eq!(page_size_label("a4"), "A4");
        assert_eq!(page_size_label(""), "");
        assert_eq!(pitch_label(9.0), "9 pt");
        assert_eq!(pitch_label(4.5), "4.5 pt");
    }

    #[test]
    fn every_offered_page_size_has_a_label() {
        for name in PAGE_SIZES {
            assert!(!page_size_label(name).is_empty(), "{name}");
        }
    }
}
