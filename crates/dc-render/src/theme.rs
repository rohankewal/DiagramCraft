//! Interface colors and metrics.
//!
//! DiagramCraft uses its own palette rather than imitating another product's.
//! Matching a competitor's exact chrome would add trade-dress risk without
//! making the editor any better, and the suite this is built for has its own
//! visual identity to fit into.

use dc_model::Color;

/// A complete set of interface colors.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// The area around the page.
    pub canvas: Color,
    /// The page sheet itself.
    pub page: Color,
    /// The page's outer edge.
    pub page_border: Color,
    /// Drop shadow beneath the page sheet.
    pub page_shadow: Color,
    /// Minor grid lines.
    pub grid: Color,
    /// Every tenth grid line.
    pub grid_major: Color,

    /// Selection outlines and active affordances.
    pub accent: Color,
    /// Fill for resize handles.
    pub handle_fill: Color,
    /// Outline for resize handles.
    pub handle_stroke: Color,
    /// Alignment guides shown while dragging.
    pub snap_guide: Color,
    /// Marquee selection fill.
    pub marquee_fill: Color,
    /// Marquee selection outline.
    pub marquee_stroke: Color,
    /// Highlight ring on a shape a connector would attach to.
    pub attach_highlight: Color,

    /// Panel and toolbar backgrounds.
    pub panel: Color,
    /// Dividers between panels.
    pub panel_border: Color,
    /// Primary text in the interface.
    pub text: Color,
    /// Secondary interface text.
    pub text_muted: Color,
    /// Hover background for interface controls.
    pub hover: Color,
    /// Background for the active item in a list.
    pub active: Color,

    /// Default fill for a newly created shape.
    pub shape_default_fill: Color,
    /// Default stroke for a newly created shape.
    pub shape_default_stroke: Color,
    /// Stand-in for an image that could not be loaded.
    pub image_placeholder: Color,
    /// Dashed outline used for hotspots, which have no visible fill.
    pub hotspot_outline: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

impl Theme {
    pub fn light() -> Self {
        Theme {
            canvas: Color::rgb(0xf1, 0xf3, 0xf6),
            page: Color::WHITE,
            page_border: Color::rgb(0xd6, 0xda, 0xe0),
            page_shadow: Color::rgba(0x0f, 0x17, 0x2a, 0x1f),
            grid: Color::rgb(0xe9, 0xec, 0xf1),
            grid_major: Color::rgb(0xd8, 0xdd, 0xe5),

            accent: Color::rgb(0x4f, 0x46, 0xe5),
            handle_fill: Color::WHITE,
            handle_stroke: Color::rgb(0x4f, 0x46, 0xe5),
            snap_guide: Color::rgb(0xe1, 0x1d, 0x74),
            marquee_fill: Color::rgba(0x4f, 0x46, 0xe5, 0x1a),
            marquee_stroke: Color::rgb(0x4f, 0x46, 0xe5),
            attach_highlight: Color::rgb(0x10, 0xb9, 0x81),

            panel: Color::rgb(0xff, 0xff, 0xff),
            panel_border: Color::rgb(0xe2, 0xe6, 0xec),
            text: Color::rgb(0x1f, 0x25, 0x37),
            text_muted: Color::rgb(0x6b, 0x72, 0x80),
            hover: Color::rgb(0xf3, 0xf4, 0xf6),
            active: Color::rgb(0xe8, 0xe7, 0xfd),

            shape_default_fill: Color::WHITE,
            shape_default_stroke: Color::rgb(0x33, 0x3a, 0x45),
            image_placeholder: Color::rgb(0xe5, 0xe7, 0xeb),
            hotspot_outline: Color::rgb(0x9c, 0xa3, 0xaf),
        }
    }

    pub fn dark() -> Self {
        Theme {
            canvas: Color::rgb(0x14, 0x17, 0x1f),
            page: Color::rgb(0x1d, 0x21, 0x2b),
            page_border: Color::rgb(0x32, 0x38, 0x45),
            page_shadow: Color::rgba(0x00, 0x00, 0x00, 0x66),
            grid: Color::rgb(0x25, 0x2a, 0x35),
            grid_major: Color::rgb(0x32, 0x38, 0x45),

            accent: Color::rgb(0x81, 0x7a, 0xf6),
            handle_fill: Color::rgb(0x1d, 0x21, 0x2b),
            handle_stroke: Color::rgb(0x81, 0x7a, 0xf6),
            snap_guide: Color::rgb(0xf4, 0x5d, 0x9b),
            marquee_fill: Color::rgba(0x81, 0x7a, 0xf6, 0x26),
            marquee_stroke: Color::rgb(0x81, 0x7a, 0xf6),
            attach_highlight: Color::rgb(0x34, 0xd3, 0x99),

            panel: Color::rgb(0x1a, 0x1e, 0x27),
            panel_border: Color::rgb(0x2b, 0x31, 0x3d),
            text: Color::rgb(0xe8, 0xea, 0xee),
            text_muted: Color::rgb(0x98, 0xa0, 0xae),
            hover: Color::rgb(0x25, 0x2a, 0x35),
            active: Color::rgb(0x2e, 0x2b, 0x4f),

            shape_default_fill: Color::rgb(0x2a, 0x30, 0x3c),
            shape_default_stroke: Color::rgb(0xb8, 0xbf, 0xcc),
            image_placeholder: Color::rgb(0x2b, 0x31, 0x3d),
            hotspot_outline: Color::rgb(0x6b, 0x72, 0x80),
        }
    }

    pub fn is_dark(&self) -> bool {
        self.canvas.luminance() < 0.2
    }
}

/// Fixed interface measurements, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Height of the menu bar across the top of the window.
    pub menubar_height: f64,
    pub left_panel_width: f64,
    pub right_panel_width: f64,
    pub page_tabs_height: f64,
    /// Side length of a selection handle.
    pub handle_size: f64,
    /// How close the pointer must be to a handle to grab it.
    pub handle_hit_slop: f64,
    /// Distance at which a drag snaps to a guide.
    pub snap_threshold: f64,
    pub corner_radius: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            // A menu bar is a single row of titles, so it is shallower than
            // the button toolbar it replaced.
            menubar_height: 30.0,
            left_panel_width: 248.0,
            right_panel_width: 272.0,
            page_tabs_height: 32.0,
            handle_size: 8.0,
            handle_hit_slop: 4.0,
            snap_threshold: 6.0,
            corner_radius: 6.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_are_classified_correctly() {
        assert!(!Theme::light().is_dark());
        assert!(Theme::dark().is_dark());
    }

    #[test]
    fn interface_text_contrasts_with_its_panel() {
        for theme in [Theme::light(), Theme::dark()] {
            let panel = theme.panel.luminance();
            let text = theme.text.luminance();
            // WCAG contrast ratio, with both orderings handled.
            let (hi, lo) = if panel > text { (panel, text) } else { (text, panel) };
            let ratio = (hi + 0.05) / (lo + 0.05);
            assert!(
                ratio >= 7.0,
                "panel/text contrast was only {ratio:.1}:1 for {}",
                if theme.is_dark() { "dark" } else { "light" }
            );
        }
    }

    #[test]
    fn muted_text_still_meets_the_minimum() {
        for theme in [Theme::light(), Theme::dark()] {
            let panel = theme.panel.luminance();
            let muted = theme.text_muted.luminance();
            let (hi, lo) = if panel > muted { (panel, muted) } else { (muted, panel) };
            let ratio = (hi + 0.05) / (lo + 0.05);
            assert!(ratio >= 4.5, "muted contrast was only {ratio:.1}:1");
        }
    }

    #[test]
    fn page_stands_out_from_the_canvas() {
        for theme in [Theme::light(), Theme::dark()] {
            assert_ne!(theme.page, theme.canvas, "the sheet must be distinguishable");
        }
    }

    #[test]
    fn grid_is_subtler_than_the_page_border() {
        let t = Theme::light();
        // A minor grid line should be lighter than the page edge, or the grid
        // competes with the content.
        assert!(t.grid.luminance() > t.page_border.luminance());
    }

    #[test]
    fn default_theme_is_light() {
        assert_eq!(Theme::default(), Theme::light());
    }
}
