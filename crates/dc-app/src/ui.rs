//! Interface chrome: menu bar, shape palette, properties panel, page tabs.
//!
//! The chrome is rebuilt from editor state each frame as a flat list of
//! [`Region`]s. Drawing and hit testing both read that list, so a control can
//! never be painted in one place and clicked in another.
//!
//! Commands live in the menu bar rather than on a toolbar row; see
//! [`crate::menu`] for the structure and why it is the one it is. What is left
//! in the panels is the things you reach for continuously while drawing — the
//! shape palette on the left, the properties of the selection on the right,
//! and the page tabs along the bottom.

use crate::editor::{Editor, TextToggle, Tool};
use crate::menu::{self, ItemKind, Menu, ViewState};
use dc_model::{BoundingBox, Color, LineType, Shape, ShapeLibrary, ShapeType};
use dc_render::theme::{Metrics, Theme};
use dc_render::TextEngine;
use kurbo::{Affine, Rect, Vec2};
use peniko::Fill as FillRule;
use vello::Scene;

/// What activating a control does.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    /// A separator, or anything else that is painted but not pressed.
    NoOp,

    /// Opens a menu by index, or closes it when it is already open.
    OpenMenu(usize),

    SelectTool(Tool),

    // File
    NewDocument,
    OpenTemplateGallery,
    CloseTemplateGallery,
    /// Replaces the document with a template, by index into `templates::ALL`.
    NewFromTemplate(usize),
    Open,
    ImportVsdx,
    Save,
    SaveAs,
    ExportPng,
    SetPageSize(String),
    ToggleInfiniteCanvas,
    Quit,

    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    EditLabel,

    // Select
    SelectAll,
    SelectNone,
    SelectAllShapes,
    SelectAllLines,
    SelectMatching(crate::editor::SelectBy),

    // Insert
    InsertImage,
    SelectLibrary(ShapeLibrary),
    /// Puts keystrokes into the palette's search field.
    FocusPaletteSearch,
    /// Puts keystrokes into the selected shape's link field.
    FocusLinkField,
    /// Removes the selected shape's link.
    ClearLink,
    /// Opens the selected shape's link: a URL in the browser, or a jump to
    /// another page.
    OpenLink,
    /// Appends an empty custom-data entry to the selected shape.
    AddCustomData,
    /// Removes a custom-data entry, by index.
    RemoveCustomData(usize),
    /// Puts keystrokes into a custom-data entry's key or value field.
    FocusCustomDataKey(usize),
    FocusCustomDataValue(usize),
    /// Shows or hides the layers panel.
    ToggleLayersPanel,
    AddLayer,
    DeleteLayer(String),
    ToggleLayerVisibility(String),
    ToggleLayerLock(String),
    /// Moves the current selection into a layer.
    MoveSelectionToLayer(String),
    /// Shows or hides the find bar.
    ToggleFind,
    /// Puts keystrokes into the find query or the replacement field.
    FocusFindQuery,
    FocusFindReplacement,
    FindNext,
    FindPrevious,
    ReplaceCurrentMatch,

    // Format
    SetFill(Color),
    SetStroke(Color),
    SetStrokeWidth(f64),
    SetStrokeStyle(dc_model::StrokeStyle),
    SetLineType(LineType),
    ToggleText(TextToggle),
    ScaleTextSize(f64),
    SetTextAlign(dc_model::TextAlign),
    SetVerticalAlign(dc_model::VerticalAlign),
    SetTextColor(Color),
    SetFontFamily(&'static str),

    // Lines
    SetEndpointStyle(crate::editor::LineEnd, dc_model::EndpointStyle),
    SwapEndpoints,

    // Shape geometry
    SetOpacity(f64),
    SetCornerRounding(f64),
    Rotate(f64),
    Flip(bool),
    ChangeShapeType(ShapeType),
    CopyStyle,
    PasteStyle,

    // Arrange
    BringToFront,
    BringForward,
    SendBackward,
    SendToBack,
    Align(crate::editor::Align),
    Distribute(bool),
    MatchSize(bool, bool),
    Group,
    Ungroup,

    // View
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomToSelection,
    ActualSize,
    ToggleGrid,
    TogglePageBounds,
    ToggleSnap,
    ToggleRouting,
    ToggleTheme,

    // Pages
    SelectPage(usize),
    AddPage,
    DeletePage(usize),
    DuplicatePage(usize),
    RenamePage(usize),
    MovePage { from: usize, to: usize },

    // Settings
    OpenSettings,
    CloseSettings,
    ChangeSetting(SettingChange),

    // Help
    ShowShortcuts,
    ShowAbout,
}

/// One preference being set to one value.
///
/// Gathered into a single action rather than a variant per preference, so
/// adding a setting is one line here and one row in the panel rather than a
/// new arm in every match in the program.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingChange {
    DarkTheme(bool),
    ShowGrid(bool),
    GridStyle(dc_render::GridStyle),
    GridSpacing(f64),
    SnapToObjects(bool),
    SnapToGrid(bool),
    SnapDistance(f64),
    ShowPageBounds(bool),
    RouteAroundShapes(bool),
    DefaultLineType(LineType),
    ZoomStep(f64),
    DefaultUnits(dc_model::Units),
    DefaultPageSize(String),
    RestoreDefaults,
}

impl SettingChange {
    /// Applies this change to a settings record.
    pub fn apply(&self, settings: &mut crate::settings::Settings) {
        match self {
            SettingChange::DarkTheme(on) => settings.dark_theme = *on,
            SettingChange::ShowGrid(on) => settings.show_grid = *on,
            SettingChange::GridStyle(style) => settings.grid_style = *style,
            SettingChange::GridSpacing(v) => settings.grid_spacing = *v,
            SettingChange::SnapToObjects(on) => settings.snap_to_objects = *on,
            SettingChange::SnapToGrid(on) => settings.snap_to_grid = *on,
            SettingChange::SnapDistance(v) => settings.snap_distance = *v,
            SettingChange::ShowPageBounds(on) => settings.show_page_bounds = *on,
            SettingChange::RouteAroundShapes(on) => settings.route_around_shapes = *on,
            SettingChange::DefaultLineType(t) => settings.default_line_type = *t,
            SettingChange::ZoomStep(v) => settings.zoom_step = *v,
            SettingChange::DefaultUnits(u) => settings.default_units = *u,
            SettingChange::DefaultPageSize(name) => {
                settings.default_page_size = name.clone();
            }
            SettingChange::RestoreDefaults => {
                // Anything a newer build wrote is left alone; this resets the
                // preferences this build knows about.
                let extra = std::mem::take(&mut settings.extra);
                *settings = crate::settings::Settings { extra, ..Default::default() };
            }
        }
    }
}

/// How a region is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum RegionKind {
    /// A labeled button.
    Button,
    /// A color chip.
    Swatch(Color),
    /// A shape preview with its name beneath.
    ShapeTile(ShapeType),
    /// A page tab.
    Tab { active: bool },
    /// A non-interactive section label.
    Heading,
    /// A title in the menu bar.
    MenuTitle { open: bool },
    /// One line of an open menu.
    MenuEntry {
        kind: ItemKind,
        shortcut: String,
        /// Whether the label is drawn in full strength.
        enabled: bool,
        /// Whether pressing it does anything, which a rule never does.
        clickable: bool,
    },
    /// The panel behind an open menu, drawn before its entries.
    MenuBackdrop,
    /// A library selector in the palette.
    LibraryTab { active: bool },
    /// A plain single-line field: the palette search, or a shape's link
    /// URL. The live value lives in `Region.label`; `placeholder` is shown
    /// in its place, muted, while the value is empty.
    TextField { focused: bool, placeholder: &'static str },
    /// A floating panel's background, clickable so a click on empty space
    /// inside it does not fall through to whatever is behind it.
    Panel,
    /// Where a dragged page tab would land.
    DropMarker,
    /// The dimmed sheet behind the settings screen.
    Scrim,
    /// The surface of the settings screen.
    Sheet,
    /// A choice in a row of mutually exclusive ones.
    Choice { active: bool },
    /// A choice that shows what it does rather than naming it.
    PreviewChoice { active: bool, preview: Preview },
}

/// Emphasis applied to a control's own label.
///
/// A control whose job is to apply a style demonstrates it: the bold button
/// is bold, the italic one italic. That is what the button means, and saying
/// it in the letterform is clearer than any label could be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emphasis {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Emphasis {
    fn of(toggle: TextToggle) -> Self {
        let mut e = Emphasis::default();
        match toggle {
            TextToggle::Bold => e.bold = true,
            TextToggle::Italic => e.italic = true,
            TextToggle::Underline => e.underline = true,
            TextToggle::Strikethrough => e.strikethrough = true,
        }
        e
    }

    fn applied_to(self, mut style: dc_model::TextStyle) -> dc_model::TextStyle {
        style.bold |= self.bold;
        style.italic |= self.italic;
        style.underline |= self.underline;
        style.strikethrough |= self.strikethrough;
        style
    }
}

/// What a control draws in place of a label, so the choice reads at a glance.
///
/// Lucidchart's line-width and line-style menus show the lines themselves
/// rather than naming them, and its alignment buttons show stacked rules.
/// Copying that is the whole point: a row of the word "Dashed" tells you less
/// than a dashed line does.
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    /// A rule of the given weight, in document points.
    Weight(f64),
    /// A rule in the given dash pattern.
    Dash(dc_model::StrokeStyle),
    /// A miniature connector following the given routing.
    Route(LineType),
    /// Stacked rules, set the way the alignment would set text.
    Align(dc_model::TextAlign),
    /// Rules pushed to the top, middle, or bottom of the chip.
    VerticalAlign(dc_model::VerticalAlign),
    /// A line ending in the given terminator, pointing the given way.
    Endpoint { style: dc_model::EndpointStyle, pointing_right: bool },
    /// A shape's outline, for the change-shape picker.
    Outline(ShapeType),
    /// A rounded corner at the given radius.
    Corner(f64),
    /// A square at the given opacity.
    Opacity(f64),
}

#[derive(Clone, Debug)]
pub struct Region {
    pub rect: Rect,
    pub action: UiAction,
    pub label: String,
    pub kind: RegionKind,
    /// Shown when the control is active, for tools and toggles.
    pub selected: bool,
    /// How the label itself is set.
    pub emphasis: Emphasis,
    /// Overrides the label's point size, for a control whose size is its
    /// meaning — the two type-size buttons.
    pub label_size: Option<f64>,
}

impl UiAction {
    /// Whether this command means something to a label that is already open,
    /// and so must not close it on its way in.
    ///
    /// Pressing the bold button is the case that matters: it is meant to
    /// style the characters the user has highlighted, so finishing the label
    /// first would throw away the very selection it is supposed to act on.
    /// Everything else is the user moving on, and closing the label is right.
    pub fn keeps_label_open(&self) -> bool {
        matches!(
            self,
            // Formatting acts on the selected characters.
            UiAction::ToggleText(_)
                | UiAction::ScaleTextSize(_)
                | UiAction::SetTextAlign(_)
                | UiAction::SetVerticalAlign(_)
                | UiAction::SetTextColor(_)
                | UiAction::SetFontFamily(_)
                // So do the text operations.
                | UiAction::Cut
                | UiAction::Copy
                | UiAction::Paste
                | UiAction::SelectAll
                // And opening a menu is not an edit at all.
                | UiAction::OpenMenu(_)
                | UiAction::NoOp
        )
    }
}

impl Region {
    /// Whether a click on this region does anything.
    fn is_clickable(&self) -> bool {
        match &self.kind {
            RegionKind::Heading
            | RegionKind::MenuBackdrop
            | RegionKind::DropMarker
            | RegionKind::Sheet => false,
            RegionKind::MenuEntry { clickable, .. } => *clickable,
            // The scrim is clickable: catching the click is what keeps it
            // from reaching the editor behind the settings screen.
            _ => true,
        }
    }
}

/// Panel rectangles for the current window size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub viewport: Vec2,
    pub menubar: Rect,
    pub left_panel: Rect,
    pub right_panel: Rect,
    /// The drawing area. Pointer coordinates must be made relative to this.
    pub canvas: Rect,
    pub tabs: Rect,
}

impl Layout {
    pub fn compute(viewport: Vec2, metrics: &Metrics) -> Self {
        let w = viewport.x.max(1.0);
        let h = viewport.y.max(1.0);

        let menubar = Rect::new(0.0, 0.0, w, metrics.menubar_height.min(h));
        let tabs_top = (h - metrics.page_tabs_height).max(menubar.y1);
        let tabs = Rect::new(0.0, tabs_top, w, h);

        // Panels collapse before the canvas does on a narrow window.
        let body_top = menubar.y1;
        let body_bottom = tabs.y0;
        let left_width = metrics.left_panel_width.min(w * 0.3);
        let right_width = metrics.right_panel_width.min(w * 0.3);

        let left_panel = Rect::new(0.0, body_top, left_width, body_bottom);
        let right_panel = Rect::new(w - right_width, body_top, w, body_bottom);
        let canvas = Rect::new(left_panel.x1, body_top, right_panel.x0, body_bottom);

        Layout { viewport, menubar, left_panel, right_panel, canvas, tabs }
    }

    pub fn canvas_size(&self) -> Vec2 {
        Vec2::new(self.canvas.width().max(1.0), self.canvas.height().max(1.0))
    }
}

/// A page tab being dragged to a new position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDrag {
    pub from: usize,
    /// Where the pointer is, in window coordinates.
    pub pointer_x: f64,
}

/// What a right click landed on, which decides which commands the context
/// menu offers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContextTarget {
    /// The canvas, or a shape on it — the editor's selection at the time of
    /// the click decides which of the two this is.
    Canvas,
    /// A specific page tab, by index, regardless of which page is active.
    Page(usize),
}

/// Which custom-data field is being typed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomDataFocus {
    Key(usize),
    Value(usize),
}

/// Chrome built for one frame.
pub struct Chrome {
    pub layout: Layout,
    pub regions: Vec<Region>,
    /// The index of the open menu, and the rectangle it covers.
    pub open_menu: Option<(usize, Rect)>,
    /// The rectangle a right-click context menu covers, when one is up.
    pub context_menu: Option<Rect>,
    /// The settings screen's sheet, when it is up.
    pub settings_sheet: Option<Rect>,
    /// The "New from Template" gallery's sheet, when it is up.
    pub template_gallery: Option<Rect>,
    /// The layers panel, when it is open.
    pub layers_panel: Option<Rect>,
    /// The find bar, when it is open.
    pub find_bar: Option<Rect>,
    /// Where each page tab sits, so a drag can be resolved to an index.
    pub tab_rects: Vec<Rect>,
}

/// Fill colors offered in the properties panel.
const FILL_SWATCHES: [Color; 10] = [
    Color::WHITE,
    Color::rgb(0xf3, 0xf4, 0xf6),
    Color::rgb(0xfe, 0xe2, 0xe2),
    Color::rgb(0xff, 0xed, 0xd5),
    Color::rgb(0xfe, 0xf3, 0xc7),
    Color::rgb(0xd1, 0xfa, 0xe5),
    Color::rgb(0xdb, 0xea, 0xfe),
    Color::rgb(0xe0, 0xe7, 0xff),
    Color::rgb(0xf3, 0xe8, 0xff),
    Color::rgb(0x1f, 0x25, 0x37),
];

/// Stroke colors offered in the properties panel.
const STROKE_SWATCHES: [Color; 8] = [
    Color::rgb(0x1f, 0x25, 0x37),
    Color::rgb(0x6b, 0x72, 0x80),
    Color::rgb(0xdc, 0x26, 0x26),
    Color::rgb(0xea, 0x58, 0x0c),
    Color::rgb(0xca, 0x8a, 0x04),
    Color::rgb(0x05, 0x96, 0x69),
    Color::rgb(0x25, 0x63, 0xeb),
    Color::rgb(0x7c, 0x3a, 0xed),
];

/// Font families offered in the properties panel, each a real face name
/// `dc-render` can request: the format's default, then a serif and a
/// monospace alternative. Freeform entry is left for later — a short list
/// covers the common cases without a font-chooser dialog.
const FONT_FAMILIES: [(&str, &str); 3] =
    [("Sans", "Liberation Sans"), ("Serif", "Georgia"), ("Mono", "Courier New")];

const PAD: f64 = 10.0;
const ROW_HEIGHT: f64 = 26.0;
const TILE_WIDTH: f64 = 70.0;
const TILE_HEIGHT: f64 = 58.0;
const SWATCH: f64 = 22.0;
/// Height of one line of an open menu.
const MENU_ROW: f64 = 24.0;
/// Width reserved at the right of a menu line for its shortcut.
const SHORTCUT_COLUMN: f64 = 72.0;
const MENU_SEPARATOR: f64 = 9.0;
const MENU_PAD: f64 = 6.0;
/// Width of a page tab.
const TAB_WIDTH: f64 = 96.0;

impl Chrome {
    /// Builds chrome for a window size.
    ///
    /// The window size is passed explicitly rather than read from the camera,
    /// because the camera's viewport is the canvas rectangle, which is smaller
    /// than the window by the width of the panels.
    pub fn build_for(
        window: Vec2,
        editor: &Editor,
        metrics: &Metrics,
        view: &ViewState,
        open_menu: Option<usize>,
        tab_drag: Option<TabDrag>,
        context_menu: Option<(ContextTarget, kurbo::Point)>,
    ) -> Chrome {
        let layout = Layout::compute(window, metrics);
        let mut regions = Vec::new();
        let menus = menu::menus(editor, view);

        let title_rects = build_menubar(&layout, &menus, open_menu, &mut regions);
        build_tool_strip(&layout, editor, &mut regions);
        build_palette(&layout, editor, view, &mut regions);
        build_properties(&layout, editor, view, &mut regions);
        let tab_rects = build_tabs(&layout, editor, tab_drag, &mut regions);

        // An open menu is built last so it paints over the panels and wins hit
        // testing against whatever it covers.
        let open = open_menu
            .and_then(|index| Some((index, *title_rects.get(index)?)))
            .map(|(index, title)| {
                let rect = build_open_menu(&layout, &menus[index], title, &mut regions);
                (index, rect)
            });

        // A context menu paints over everything else the same way, including
        // a menu-bar menu that happened to still be open.
        let context_menu = context_menu.map(|(target, at)| {
            let menu = menu::context_menu(editor, target);
            build_context_menu(&layout, &menu, at, &mut regions)
        });

        // The layers panel floats over the canvas rather than resizing it,
        // the same way a context menu does, so opening it does not disturb
        // the camera's framing.
        let layers_panel =
            view.layers_panel_open.then(|| build_layers_panel(&layout, editor, &mut regions));

        // The find bar floats the same way.
        let find_bar = view.find_open.then(|| build_find_bar(&layout, editor, view, &mut regions));

        // The template gallery is modal, the same way settings is.
        let template_gallery = view
            .template_gallery_open
            .then(|| build_template_gallery(&layout, &mut regions));

        // The settings screen covers everything, menus included.
        let settings_sheet = view
            .settings_open
            .then(|| build_settings(&layout, view, &mut regions));

        Chrome {
            layout,
            regions,
            open_menu: open,
            context_menu,
            settings_sheet,
            template_gallery,
            layers_panel,
            find_bar,
            tab_rects,
        }
    }

    /// The control under a window-space point.
    ///
    /// Non-interactive regions are skipped: they occupy space but are not
    /// clickable, and treating them as hits would swallow clicks meant for the
    /// panel behind them.
    pub fn hit(&self, p: kurbo::Point) -> Option<usize> {
        // The settings screen needs no special case: its regions are appended
        // after the rest of the chrome and its scrim covers the whole window,
        // so the topmost-wins rule below already stops a click reaching
        // anything behind it.

        // A context menu is opaque in the same way an open menu is, and it
        // paints on top of one if both somehow landed open at once.
        if self.covered_by_context_menu(p) {
            return self.regions.iter().rposition(|r| {
                matches!(r.kind, RegionKind::MenuEntry { .. })
                    && r.is_clickable()
                    && r.rect.contains(p)
            });
        }

        // An open menu is opaque: a point inside its panel can only reach that
        // menu's own entries, never a control the panel happens to cover. A
        // dimmed command or a rule therefore swallows the click rather than
        // letting it fall through to the palette underneath.
        if self.covered_by_open_menu(p) {
            return self.regions.iter().rposition(|r| {
                matches!(r.kind, RegionKind::MenuEntry { .. })
                    && r.is_clickable()
                    && r.rect.contains(p)
            });
        }

        // Reverse order so later regions, drawn on top, win.
        self.regions
            .iter()
            .rposition(|r| r.is_clickable() && r.rect.contains(p))
    }

    /// Whether a window-space point is over the drawing area.
    pub fn is_over_canvas(&self, p: kurbo::Point) -> bool {
        // The settings screen and the template gallery are both modal, so
        // nothing is over the canvas while either is up.
        if self.settings_sheet.is_some()
            || self.template_gallery.is_some()
            || self.covered_by_open_menu(p)
            || self.covered_by_context_menu(p)
        {
            return false;
        }
        self.layout.canvas.contains(p) && self.hit(p).is_none()
    }

    /// Whether a point falls inside the open menu's panel.
    pub fn covered_by_open_menu(&self, p: kurbo::Point) -> bool {
        self.open_menu.is_some_and(|(_, rect)| rect.contains(p))
    }

    /// Whether a point falls inside the context menu's panel.
    pub fn covered_by_context_menu(&self, p: kurbo::Point) -> bool {
        self.context_menu.is_some_and(|rect| rect.contains(p))
    }

    /// The page tab index a pointer at `x` is over or nearest to.
    ///
    /// Used while dragging a tab: the answer is the slot the tab would drop
    /// into, so it ranges over every existing index.
    pub fn tab_index_at(&self, x: f64) -> Option<usize> {
        tab_index_in(&self.tab_rects, x)
    }

    pub fn draw(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        metrics: &Metrics,
        text: &mut TextEngine,
        hovered: Option<usize>,
        status: &str,
    ) {
        let l = &self.layout;

        for panel in [l.menubar, l.left_panel, l.right_panel, l.tabs] {
            scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.panel), None, &panel);
        }

        // Hairline dividers between the chrome and the canvas.
        let divider = theme.panel_border;
        for edge in [
            Rect::new(l.menubar.x0, l.menubar.y1 - 1.0, l.menubar.x1, l.menubar.y1),
            Rect::new(l.left_panel.x1 - 1.0, l.left_panel.y0, l.left_panel.x1, l.left_panel.y1),
            Rect::new(l.right_panel.x0, l.right_panel.y0, l.right_panel.x0 + 1.0, l.right_panel.y1),
            Rect::new(l.tabs.x0, l.tabs.y0, l.tabs.x1, l.tabs.y0 + 1.0),
        ] {
            scene.fill(FillRule::NonZero, Affine::IDENTITY, px(divider), None, &edge);
        }

        for (i, region) in self.regions.iter().enumerate() {
            let is_hovered = hovered == Some(i);
            self.draw_region(scene, theme, metrics, text, region, is_hovered);
        }

        if !status.is_empty() {
            let style = label_style(theme.text_muted, 11.0);
            let block = text.layout_plain(status, &style, Some(l.tabs.width() - 220.0));
            let area = Rect::new(
                l.tabs.x1 - 8.0 - block.width(),
                l.tabs.y0 + (l.tabs.height() - block.height()) / 2.0,
                l.tabs.x1 - 8.0,
                l.tabs.y1,
            );
            dc_render::text::draw(scene, &block, area, Affine::IDENTITY);
        }
    }

    fn draw_region(
        &self,
        scene: &mut Scene,
        theme: &Theme,
        metrics: &Metrics,
        text: &mut TextEngine,
        region: &Region,
        hovered: bool,
    ) {
        let radius = metrics.corner_radius.min(region.rect.height() / 2.0);
        let rounded = kurbo::RoundedRect::from_rect(region.rect, radius);

        match &region.kind {
            RegionKind::Swatch(color) => {
                if region.selected {
                    // The selection is drawn as a ring standing off the chip,
                    // with the panel showing through the gap. A plain heavier
                    // outline would disappear against a swatch whose own
                    // colour is close to the accent.
                    let ring = kurbo::RoundedRect::from_rect(
                        region.rect,
                        metrics.corner_radius.min(region.rect.height() / 2.0),
                    );
                    scene.stroke(
                        &kurbo::Stroke::new(2.0),
                        Affine::IDENTITY,
                        px(theme.accent),
                        None,
                        &ring,
                    );
                    let inset = region.rect.inflate(-3.0, -3.0);
                    let chip = kurbo::RoundedRect::from_rect(inset, (metrics.corner_radius - 2.0).max(1.0));
                    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(*color), None, &chip);
                    scene.stroke(
                        &kurbo::Stroke::new(1.0),
                        Affine::IDENTITY,
                        px(theme.panel_border),
                        None,
                        &chip,
                    );
                    return;
                }

                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(*color), None, &rounded);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(if hovered { theme.accent } else { theme.panel_border }),
                    None,
                    &rounded,
                );
                return;
            }

            RegionKind::ShapeTile(shape_type) => {
                if hovered || region.selected {
                    let bg = if region.selected { theme.active } else { theme.hover };
                    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(bg), None, &rounded);
                }
                // The preview is the real outline, so the palette cannot drift
                // from what the canvas draws.
                let preview = Rect::new(
                    region.rect.x0 + 10.0,
                    region.rect.y0 + 6.0,
                    region.rect.x1 - 10.0,
                    region.rect.y0 + 6.0 + 26.0,
                );
                let probe = Shape::new(
                    "preview",
                    shape_type.clone(),
                    BoundingBox::new(
                        preview.x0,
                        preview.y0,
                        preview.width(),
                        preview.height(),
                    ),
                );
                let geo = dc_geom::geometry(&probe);
                if geo.filled {
                    scene.fill(
                        FillRule::NonZero,
                        Affine::IDENTITY,
                        px(theme.page),
                        None,
                        &geo.outline,
                    );
                }
                if geo.stroked {
                    scene.stroke(
                        &kurbo::Stroke::new(1.2),
                        Affine::IDENTITY,
                        px(theme.text_muted),
                        None,
                        &geo.outline,
                    );
                }
                if let Some(decoration) = &geo.decoration {
                    scene.stroke(
                        &kurbo::Stroke::new(1.0),
                        Affine::IDENTITY,
                        px(theme.text_muted),
                        None,
                        decoration,
                    );
                }

                let style = label_style(theme.text_muted, 9.5);
                let block = text.layout_plain(&region.label, &style, Some(region.rect.width() - 4.0));
                let area = Rect::new(
                    region.rect.x0 + 2.0,
                    preview.y1 + 2.0,
                    region.rect.x1 - 2.0,
                    region.rect.y1,
                );
                dc_render::text::draw(scene, &block, area, Affine::IDENTITY);
                return;
            }

            RegionKind::Heading => {
                // A section label: left-aligned, muted, no surface.
                let style = dc_model::TextStyle {
                    color: theme.text_muted,
                    font_size_pt: 10.0,
                    bold: true,
                    align: dc_model::TextAlign::Left,
                    vertical_align: dc_model::VerticalAlign::Center,
                    ..Default::default()
                };
                let block = text.layout_plain(&region.label, &style, Some(region.rect.width()));
                dc_render::text::draw(scene, &block, region.rect, Affine::IDENTITY);
                return;
            }

            RegionKind::TextField { focused, placeholder } => {
                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.panel), None, &rounded);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(if *focused { theme.accent } else { theme.panel_border }),
                    None,
                    &rounded,
                );

                let empty = region.label.is_empty();
                let shown: &str = if empty { placeholder } else { &region.label };
                let style = dc_model::TextStyle {
                    color: if empty { theme.text_muted } else { theme.text },
                    font_size_pt: 11.0,
                    align: dc_model::TextAlign::Left,
                    vertical_align: dc_model::VerticalAlign::Center,
                    ..Default::default()
                };
                let area = Rect::new(
                    region.rect.x0 + 8.0,
                    region.rect.y0,
                    region.rect.x1 - 8.0,
                    region.rect.y1,
                );
                let block = text.layout_plain(shown, &style, Some(area.width()));
                dc_render::text::draw(scene, &block, area, Affine::IDENTITY);
                return;
            }

            RegionKind::MenuBackdrop => {
                // A shadow first, so the panel reads as floating over the
                // canvas rather than cut into it.
                let shadow = region.rect.inflate(0.0, 0.0) + kurbo::Vec2::new(1.0, 3.0);
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(theme.page_shadow),
                    None,
                    &kurbo::RoundedRect::from_rect(shadow, radius),
                );
                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.panel), None, &rounded);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(theme.panel_border),
                    None,
                    &rounded,
                );
                return;
            }

            RegionKind::MenuEntry { kind, shortcut, enabled, .. } => {
                if *kind == ItemKind::Separator {
                    let y = region.rect.center().y;
                    scene.fill(
                        FillRule::NonZero,
                        Affine::IDENTITY,
                        px(theme.panel_border),
                        None,
                        &Rect::new(region.rect.x0 + 8.0, y, region.rect.x1 - 8.0, y + 1.0),
                    );
                    return;
                }

                if hovered && *enabled {
                    scene.fill(
                        FillRule::NonZero,
                        Affine::IDENTITY,
                        px(theme.active),
                        None,
                        &rounded,
                    );
                }

                let color = if *enabled { theme.text } else { theme.text_muted };
                // A check mark in a reserved gutter, so labels line up whether
                // or not the item has state. It is drawn rather than typeset:
                // the bundled font has no tick, and a missing glyph box would
                // be worse than no mark at all.
                if let ItemKind::Check(true) = kind {
                    let (cx, cy) = (region.rect.x0 + 12.0, region.rect.center().y);
                    let mut tick = kurbo::BezPath::new();
                    tick.move_to((cx - 4.5, cy));
                    tick.line_to((cx - 1.5, cy + 3.5));
                    tick.line_to((cx + 4.5, cy - 3.5));
                    scene.stroke(
                        &kurbo::Stroke::new(1.6).with_caps(kurbo::Cap::Round),
                        Affine::IDENTITY,
                        px(theme.accent),
                        None,
                        &tick,
                    );
                }

                let style = dc_model::TextStyle {
                    color,
                    font_size_pt: 11.5,
                    align: dc_model::TextAlign::Left,
                    vertical_align: dc_model::VerticalAlign::Center,
                    ..Default::default()
                };
                let label_area = Rect::new(
                    region.rect.x0 + 22.0,
                    region.rect.y0,
                    region.rect.x1 - SHORTCUT_COLUMN,
                    region.rect.y1,
                );
                let block =
                    text.layout_plain(&region.label, &style, Some(label_area.width().max(1.0)));
                dc_render::text::draw(scene, &block, label_area, Affine::IDENTITY);

                if !shortcut.is_empty() {
                    let hint = dc_model::TextStyle {
                        color: theme.text_muted,
                        font_size_pt: 10.5,
                        align: dc_model::TextAlign::Right,
                        vertical_align: dc_model::VerticalAlign::Center,
                        ..Default::default()
                    };
                    let area = Rect::new(
                        region.rect.x1 - SHORTCUT_COLUMN + 2.0,
                        region.rect.y0,
                        region.rect.x1 - 8.0,
                        region.rect.y1,
                    );
                    let block = text.layout_plain(shortcut, &hint, Some(area.width().max(1.0)));
                    dc_render::text::draw(scene, &block, area, Affine::IDENTITY);
                }
                return;
            }

            RegionKind::MenuTitle { open } => {
                if *open || hovered {
                    let bg = if *open { theme.active } else { theme.hover };
                    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(bg), None, &rounded);
                }
                let color = if *open { theme.accent } else { theme.text };
                let style = label_style(color, 12.0);
                let block = text.layout_plain(&region.label, &style, Some(region.rect.width()));
                dc_render::text::draw(scene, &block, region.rect, Affine::IDENTITY);
                return;
            }

            RegionKind::DropMarker => {
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(theme.accent),
                    None,
                    &region.rect,
                );
                return;
            }

            RegionKind::Scrim => {
                // Dimming what is behind says the rest of the window is not
                // listening, which is the whole point of a modal screen.
                let shade = if theme.is_dark() {
                    peniko::Color::from_rgba8(0, 0, 0, 0x99)
                } else {
                    peniko::Color::from_rgba8(0x0f, 0x17, 0x2a, 0x66)
                };
                scene.fill(FillRule::NonZero, Affine::IDENTITY, shade, None, &region.rect);
                return;
            }

            RegionKind::Sheet => {
                let shadow = region.rect + kurbo::Vec2::new(0.0, 6.0);
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(theme.page_shadow),
                    None,
                    &kurbo::RoundedRect::from_rect(shadow, 12.0),
                );
                let surface = kurbo::RoundedRect::from_rect(region.rect, 12.0);
                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.panel), None, &surface);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(theme.panel_border),
                    None,
                    &surface,
                );
                return;
            }

            RegionKind::Panel => {
                let shadow = region.rect + kurbo::Vec2::new(0.0, 6.0);
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(theme.page_shadow),
                    None,
                    &kurbo::RoundedRect::from_rect(shadow, 12.0),
                );
                let surface = kurbo::RoundedRect::from_rect(region.rect, 12.0);
                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.panel), None, &surface);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(theme.panel_border),
                    None,
                    &surface,
                );
                return;
            }

            RegionKind::Choice { active } => {
                draw_chip(scene, theme, &rounded, *active, hovered);

                let color = if *active { theme.accent } else { theme.text };
                let size = region.label_size.unwrap_or(11.0);
                let style = region.emphasis.applied_to(label_style(color, size));
                let block =
                    text.layout_plain(&region.label, &style, Some(region.rect.width() - 6.0));
                dc_render::text::draw(scene, &block, region.rect, Affine::IDENTITY);
                return;
            }

            RegionKind::PreviewChoice { active, preview } => {
                draw_chip(scene, theme, &rounded, *active, hovered);
                let ink = if *active { theme.accent } else { theme.text };
                draw_preview(scene, region.rect, preview, ink, theme);
                return;
            }

            RegionKind::LibraryTab { active } => {
                if *active || hovered {
                    let bg = if *active { theme.active } else { theme.hover };
                    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(bg), None, &rounded);
                }
                let color = if *active { theme.accent } else { theme.text_muted };
                let style = label_style(color, 10.0);
                let block = text.layout_plain(&region.label, &style, Some(region.rect.width()));
                dc_render::text::draw(scene, &block, region.rect, Affine::IDENTITY);
                return;
            }

            RegionKind::Tab { active } => {
                if *active || hovered {
                    let bg = if *active { theme.active } else { theme.hover };
                    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(bg), None, &rounded);
                }
            }

            RegionKind::Button => {
                // A resting surface so the control reads as pressable rather
                // than as a line of text.
                let surface = if hovered { theme.hover } else { theme.panel };
                scene.fill(FillRule::NonZero, Affine::IDENTITY, px(surface), None, &rounded);
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(if hovered { theme.accent } else { theme.panel_border }),
                    None,
                    &rounded,
                );
            }

        }

        if region.selected {
            scene.fill(FillRule::NonZero, Affine::IDENTITY, px(theme.active), None, &rounded);
            scene.stroke(
                &kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                px(theme.accent),
                None,
                &rounded,
            );
        }

        let color = if region.selected { theme.accent } else { theme.text };
        let size = region.label_size.unwrap_or(11.5);
        let style = region.emphasis.applied_to(label_style(color, size));
        let block = text.layout_plain(&region.label, &style, Some(region.rect.width() - 8.0));
        let area = Rect::new(
            region.rect.x0 + 4.0,
            region.rect.y0,
            region.rect.x1 - 4.0,
            region.rect.y1,
        );
        dc_render::text::draw(scene, &block, area, Affine::IDENTITY);
    }
}

/// The outward vector for an endpoint preview pointing one way or the other.
///
/// `endpoint_marker` takes the direction pointing *away from* the shape the
/// endpoint attaches to, and draws the glyph extending back along it. The
/// outward vector is therefore the opposite of the way the glyph points, and
/// getting the sign wrong turns every terminator inward.
fn endpoint_outward(pointing_right: bool) -> kurbo::Vec2 {
    if pointing_right {
        kurbo::Vec2::new(-1.0, 0.0)
    } else {
        kurbo::Vec2::new(1.0, 0.0)
    }
}

/// The surface of a segmented choice: a flat chip that lights up when it is
/// the current value.
fn draw_chip(
    scene: &mut Scene,
    theme: &Theme,
    shape: &kurbo::RoundedRect,
    active: bool,
    hovered: bool,
) {
    let surface = if active {
        theme.active
    } else if hovered {
        theme.hover
    } else {
        theme.panel
    };
    scene.fill(FillRule::NonZero, Affine::IDENTITY, px(surface), None, shape);
    scene.stroke(
        &kurbo::Stroke::new(if active { 1.5 } else { 1.0 }),
        Affine::IDENTITY,
        px(if active || hovered { theme.accent } else { theme.panel_border }),
        None,
        shape,
    );
}

/// Draws what a choice does, inside its chip.
fn draw_preview(scene: &mut Scene, rect: Rect, preview: &Preview, ink: Color, theme: &Theme) {
    let pad = 8.0;
    let inner = Rect::new(rect.x0 + pad, rect.y0 + 6.0, rect.x1 - pad, rect.y1 - 6.0);
    let mid_y = inner.center().y;

    match *preview {
        Preview::Weight(width) => {
            // The real weight, capped so a heavy line still fits the chip.
            let w = width.clamp(0.5, inner.height());
            scene.fill(
                FillRule::NonZero,
                Affine::IDENTITY,
                px(ink),
                None,
                &Rect::new(inner.x0, mid_y - w / 2.0, inner.x1, mid_y + w / 2.0),
            );
        }

        Preview::Dash(style) => {
            let mut line = kurbo::BezPath::new();
            line.move_to((inner.x0, mid_y));
            line.line_to((inner.x1, mid_y));
            let width = 1.6;
            let stroke = match style.dash_pattern(width) {
                Some(pattern) => kurbo::Stroke::new(width).with_dashes(0.0, pattern),
                None => kurbo::Stroke::new(width),
            };
            scene.stroke(&stroke, Affine::IDENTITY, px(ink), None, &line);
        }

        Preview::Route(line_type) => {
            // A connector from the lower left to the upper right, bent the way
            // this routing would bend it.
            let (x0, y0) = (inner.x0, inner.y1);
            let (x1, y1) = (inner.x1, inner.y0);
            let mut path = kurbo::BezPath::new();
            path.move_to((x0, y0));
            match line_type {
                LineType::Straight => path.line_to((x1, y1)),
                LineType::Elbow => {
                    let mid = (x0 + x1) / 2.0;
                    path.line_to((mid, y0));
                    path.line_to((mid, y1));
                    path.line_to((x1, y1));
                }
                LineType::Curved => {
                    path.curve_to((x0 + (x1 - x0) * 0.6, y0), (x1 - (x1 - x0) * 0.6, y1), (x1, y1));
                }
            }
            scene.stroke(
                &kurbo::Stroke::new(1.6).with_caps(kurbo::Cap::Round),
                Affine::IDENTITY,
                px(ink),
                None,
                &path,
            );
        }

        Preview::Align(align) => {
            // Four rules set the way this alignment would set a paragraph.
            let count = 4;
            let gap = inner.height() / count as f64;
            let thickness = (gap * 0.32).clamp(1.0, 2.0);
            for i in 0..count {
                // A ragged edge is what makes left and right tell apart, so
                // alternate lines are short.
                let fraction = if i % 2 == 1 { 0.62 } else { 1.0 };
                let w = inner.width() * fraction;
                let x0 = match align {
                    dc_model::TextAlign::Left | dc_model::TextAlign::Justify => inner.x0,
                    dc_model::TextAlign::Center => inner.center().x - w / 2.0,
                    dc_model::TextAlign::Right => inner.x1 - w,
                };
                let y = inner.y0 + gap * i as f64 + (gap - thickness) / 2.0;
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(ink),
                    None,
                    &Rect::new(x0, y, x0 + w, y + thickness),
                );
            }
        }

        Preview::VerticalAlign(align) => {
            // A frame with the text block pushed to one end of it. The frame
            // is what makes top, middle, and bottom tell apart, so it takes
            // the whole chip rather than the padded interior.
            let frame = Rect::new(rect.x0 + 6.0, rect.y0 + 3.0, rect.x1 - 6.0, rect.y1 - 3.0);
            scene.stroke(
                &kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                px(theme.text_muted),
                None,
                &frame,
            );

            let thickness = 2.0;
            let block_height = thickness * 3.0;
            let y0 = match align {
                dc_model::VerticalAlign::Top => frame.y0 + 2.5,
                dc_model::VerticalAlign::Center => frame.center().y - block_height / 2.0,
                dc_model::VerticalAlign::Bottom => frame.y1 - block_height - 2.5,
            };
            for i in 0..2 {
                let y = y0 + i as f64 * (block_height - thickness);
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(ink),
                    None,
                    &Rect::new(frame.x0 + 3.0, y, frame.x1 - 3.0, y + thickness),
                );
            }
        }

        Preview::Endpoint { ref style, pointing_right } => {
            // A stub of line with the terminator on it, drawn by the same
            // code the canvas uses, so the picker cannot show a glyph that
            // differs from what gets drawn.
            let width = 1.4;
            let outward = endpoint_outward(pointing_right);
            let (tail, tip) = if pointing_right {
                (inner.x0, inner.x1 - 1.0)
            } else {
                (inner.x1, inner.x0 + 1.0)
            };

            let mut shaft = kurbo::BezPath::new();
            shaft.move_to((tail, mid_y));
            shaft.line_to((tip, mid_y));
            scene.stroke(
                &kurbo::Stroke::new(width),
                Affine::IDENTITY,
                px(ink),
                None,
                &shaft,
            );

            let anchor = dc_model::Point::new(tip, mid_y);
            if let Some(marker) = dc_geom::endpoint_marker(style, anchor, outward, width) {
                match marker.fill {
                    dc_geom::MarkerFill::Solid => {
                        scene.fill(
                            FillRule::NonZero,
                            Affine::IDENTITY,
                            px(ink),
                            None,
                            &marker.path,
                        );
                    }
                    dc_geom::MarkerFill::Hollow => {
                        scene.fill(
                            FillRule::NonZero,
                            Affine::IDENTITY,
                            px(theme.panel),
                            None,
                            &marker.path,
                        );
                        scene.stroke(
                            &kurbo::Stroke::new(width),
                            Affine::IDENTITY,
                            px(ink),
                            None,
                            &marker.path,
                        );
                    }
                    dc_geom::MarkerFill::Open => {
                        scene.stroke(
                            &kurbo::Stroke::new(width),
                            Affine::IDENTITY,
                            px(ink),
                            None,
                            &marker.path,
                        );
                    }
                }
            }
        }

        Preview::Outline(ref shape_type) => {
            // The real outline, so the change-shape picker cannot drift from
            // what the canvas draws.
            let box_ = Rect::new(
                rect.x0 + 6.0,
                rect.y0 + 5.0,
                rect.x1 - 6.0,
                rect.y1 - 5.0,
            );
            let probe = Shape::new(
                "preview",
                shape_type.clone(),
                BoundingBox::new(box_.x0, box_.y0, box_.width(), box_.height()),
            );
            let geo = dc_geom::geometry(&probe);
            if geo.stroked {
                scene.stroke(
                    &kurbo::Stroke::new(1.2),
                    Affine::IDENTITY,
                    px(ink),
                    None,
                    &geo.outline,
                );
            }
            if let Some(decoration) = &geo.decoration {
                scene.stroke(
                    &kurbo::Stroke::new(1.0),
                    Affine::IDENTITY,
                    px(ink),
                    None,
                    decoration,
                );
            }
        }

        Preview::Corner(radius) => {
            // A box drawn with the radius it would apply.
            let box_ = Rect::new(
                rect.x0 + 7.0,
                rect.y0 + 5.0,
                rect.x1 - 7.0,
                rect.y1 - 5.0,
            );
            let r = radius.min(box_.width().min(box_.height()) / 2.0);
            scene.stroke(
                &kurbo::Stroke::new(1.3),
                Affine::IDENTITY,
                px(ink),
                None,
                &kurbo::RoundedRect::from_rect(box_, r),
            );
        }

        Preview::Opacity(percent) => {
            // The chip filled at the opacity it would set, over a rule that
            // shows through it.
            let box_ = Rect::new(
                rect.x0 + 7.0,
                rect.y0 + 5.0,
                rect.x1 - 7.0,
                rect.y1 - 5.0,
            );
            scene.fill(
                FillRule::NonZero,
                Affine::IDENTITY,
                px(theme.text_muted),
                None,
                &Rect::new(box_.x0, mid_y - 0.5, box_.x1, mid_y + 0.5),
            );
            let alpha = (percent / 100.0 * 255.0).clamp(0.0, 255.0) as u8;
            scene.fill(
                FillRule::NonZero,
                Affine::IDENTITY,
                px(ink.with_alpha(alpha)),
                None,
                &box_,
            );
            scene.stroke(
                &kurbo::Stroke::new(1.0),
                Affine::IDENTITY,
                px(theme.panel_border),
                None,
                &box_,
            );
        }
    }
}

fn label_style(color: Color, size: f64) -> dc_model::TextStyle {
    dc_model::TextStyle {
        color,
        font_size_pt: size,
        align: dc_model::TextAlign::Center,
        vertical_align: dc_model::VerticalAlign::Center,
        ..Default::default()
    }
}

fn button(rect: Rect, label: &str, action: UiAction) -> Region {
    Region {
        rect,
        action,
        label: label.to_string(),
        kind: RegionKind::Button,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    }
}

// ---------------------------------------------------------------- menu bar

/// Lays the menu titles out and returns where each one landed.
fn build_menubar(
    layout: &Layout,
    menus: &[Menu],
    open: Option<usize>,
    out: &mut Vec<Region>,
) -> Vec<Rect> {
    let bar = layout.menubar;
    // A platform with a system menu bar gives the strip no height, and the
    // commands live up there instead.
    if bar.height() < 1.0 {
        return Vec::new();
    }
    let height = (bar.height() - 8.0).max(16.0);
    let y = bar.y0 + (bar.height() - height) / 2.0;
    let mut x = PAD;
    let mut rects = Vec::with_capacity(menus.len());

    for (index, menu) in menus.iter().enumerate() {
        // Roughly a character of width each, plus padding, which is close
        // enough for titles this short and avoids laying text out twice.
        let width = (menu.title.chars().count() as f64 * 7.5 + 20.0).max(40.0);
        let rect = Rect::new(x, y, x + width, y + height);
        rects.push(rect);

        out.push(Region {
            rect,
            action: UiAction::OpenMenu(index),
            label: menu.title.clone(),
            kind: RegionKind::MenuTitle { open: open == Some(index) },
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        x += width + 2.0;
    }

    rects
}

/// Lays an open menu out beneath its title, and returns the panel rectangle.
/// The width and height a menu's panel needs: wide enough for the longest
/// label plus the shortcut column, tall enough for every row, neither ever
/// wider or taller than the window itself.
fn menu_panel_extent(layout: &Layout, menu: &Menu) -> (f64, f64) {
    let height: f64 = menu
        .items
        .iter()
        .map(|i| if i.is_separator() { MENU_SEPARATOR } else { MENU_ROW })
        .sum::<f64>()
        + MENU_PAD * 2.0;

    let widest = menu
        .items
        .iter()
        .map(|i| i.label.chars().count() as f64 * 6.6)
        .fold(0.0_f64, f64::max);
    let width = (widest + 30.0 + SHORTCUT_COLUMN).clamp(180.0, layout.viewport.x.max(180.0));

    (width, height)
}

/// Lays a menu's items out inside an already-placed panel.
fn fill_menu_panel(panel: Rect, menu: &Menu, out: &mut Vec<Region>) {
    out.push(Region {
        rect: panel,
        action: UiAction::NoOp,
        label: String::new(),
        kind: RegionKind::MenuBackdrop,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let mut y = panel.y0 + MENU_PAD;
    for item in &menu.items {
        let row = if item.is_separator() { MENU_SEPARATOR } else { MENU_ROW };
        // A menu taller than the window is clipped rather than drawn off the
        // bottom edge where it could never be clicked.
        if y + row > panel.y1 {
            break;
        }
        out.push(Region {
            rect: Rect::new(panel.x0 + 2.0, y, panel.x1 - 2.0, y + row),
            action: item.action.clone(),
            label: item.label.clone(),
            kind: RegionKind::MenuEntry {
                kind: item.kind.clone(),
                shortcut: item.shortcut.clone(),
                enabled: item.enabled,
                clickable: item.is_clickable(),
            },
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        y += row;
    }
}

fn build_open_menu(layout: &Layout, menu: &Menu, title: Rect, out: &mut Vec<Region>) -> Rect {
    let (width, height) = menu_panel_extent(layout, menu);

    // Flip to the left of the title rather than overflow the window.
    let x0 = if title.x0 + width <= layout.viewport.x {
        title.x0
    } else {
        (layout.viewport.x - width).max(0.0)
    };
    let y0 = title.y1 + 2.0;
    let panel = Rect::new(x0, y0, x0 + width, (y0 + height).min(layout.viewport.y));

    fill_menu_panel(panel, menu, out);
    panel
}

/// Lays a context menu out anchored at the point that was clicked, the way a
/// native right-click menu does: it opens below and to the right of the
/// click, but flips to whichever side still has room rather than running off
/// a window edge.
fn build_context_menu(layout: &Layout, menu: &Menu, at: kurbo::Point, out: &mut Vec<Region>) -> Rect {
    let (width, height) = menu_panel_extent(layout, menu);

    let x0 = if at.x + width <= layout.viewport.x {
        at.x
    } else {
        (layout.viewport.x - width).max(0.0)
    };
    let y0 = if at.y + height <= layout.viewport.y {
        at.y
    } else {
        (at.y - height).max(0.0)
    };
    let panel = Rect::new(x0, y0, x0 + width, (y0 + height).min(layout.viewport.y));

    fill_menu_panel(panel, menu, out);
    panel
}

// ------------------------------------------------------------- left panel

/// The three pointer tools, above the palette.
///
/// These stay out of the menus because they are modes you switch between
/// constantly while drawing, not commands you run.
fn build_tool_strip(layout: &Layout, editor: &Editor, out: &mut Vec<Region>) {
    let panel = layout.left_panel;
    if panel.width() < 120.0 {
        return;
    }
    let x0 = panel.x0 + PAD;
    let x1 = panel.x1 - PAD;
    let y = panel.y0 + PAD;
    let width = ((x1 - x0) - 3.0 * 4.0) / 4.0;

    for (i, (label, tool)) in [
        ("Select", Tool::Select),
        ("Pan", Tool::Pan),
        ("Connect", Tool::Connector),
        ("Pen", Tool::Freehand),
    ]
    .into_iter()
    .enumerate()
    {
        let x = x0 + i as f64 * (width + 4.0);
        let mut region = button(
            Rect::new(x, y, x + width, y + ROW_HEIGHT),
            label,
            UiAction::SelectTool(tool.clone()),
        );
        region.selected = editor.tool == tool;
        out.push(region);
    }
}

/// The vertical offset at which the shape tiles start, below the tool strip
/// and the library selector.
fn palette_top(layout: &Layout) -> f64 {
    layout.left_panel.y0 + PAD + ROW_HEIGHT + 8.0
}

fn build_palette(layout: &Layout, editor: &Editor, view: &ViewState, out: &mut Vec<Region>) {
    let panel = layout.left_panel;
    if panel.width() < TILE_WIDTH + PAD * 2.0 {
        return;
    }
    let x0 = panel.x0 + PAD;
    let x1 = panel.x1 - PAD;
    let mut y = palette_top(layout);

    out.push(Region {
        rect: Rect::new(x0, y, x1, y + 22.0),
        action: UiAction::FocusPaletteSearch,
        label: view.palette_search.clone(),
        kind: RegionKind::TextField {
            focused: view.palette_search_focused,
            placeholder: "Search shapes…",
        },
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 22.0 + 8.0;

    let columns = ((panel.width() - PAD) / (TILE_WIDTH + 4.0)).floor().max(1.0) as usize;
    let query = view.palette_search.trim().to_ascii_lowercase();
    if !query.is_empty() {
        // A search spans every library at once, since the point is not
        // having to already know which one a shape lives in.
        let matches = ShapeLibrary::ALL.iter().flat_map(|lib| lib.types()).filter(|t| {
            t.display_name().to_ascii_lowercase().contains(&query)
        });
        build_shape_tiles(matches, x0, panel.y1, y, columns, editor, out);
        return;
    }

    // A row of library tabs, because sixty-eight shapes do not fit at once.
    let per_row = 4usize;
    let tab_width = ((x1 - x0) - (per_row - 1) as f64 * 3.0) / per_row as f64;
    for (i, library) in ShapeLibrary::ALL.iter().enumerate() {
        let col = i % per_row;
        let row = i / per_row;
        let tx = x0 + col as f64 * (tab_width + 3.0);
        let ty = y + row as f64 * 21.0;
        out.push(Region {
            rect: Rect::new(tx, ty, tx + tab_width, ty + 19.0),
            action: UiAction::SelectLibrary(*library),
            label: library.short_name().to_string(),
            kind: RegionKind::LibraryTab { active: view.library == *library },
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
    }
    y += ShapeLibrary::ALL.len().div_ceil(per_row) as f64 * 21.0 + 6.0;

    out.push(Region {
        rect: Rect::new(x0, y, x1, y + 18.0),
        action: UiAction::NoOp,
        label: view.library.display_name().to_string(),
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 22.0;

    build_shape_tiles(view.library.types().into_iter(), x0, panel.y1, y, columns, editor, out);
}

/// Lays shape tiles out in a grid, stopping once they would run past
/// `bottom` — shared between browsing one library and a cross-library
/// search, which differ only in which shapes they offer.
fn build_shape_tiles(
    shapes: impl Iterator<Item = ShapeType>,
    x0: f64,
    bottom: f64,
    y: f64,
    columns: usize,
    editor: &Editor,
    out: &mut Vec<Region>,
) {
    for (i, shape_type) in shapes.enumerate() {
        let col = i % columns;
        let row = i / columns;
        let x = x0 + col as f64 * (TILE_WIDTH + 4.0);
        let top = y + row as f64 * (TILE_HEIGHT + 4.0);
        if top + TILE_HEIGHT > bottom {
            break;
        }

        let selected = editor.tool == Tool::Shape(shape_type.clone());
        out.push(Region {
            rect: Rect::new(x, top, x + TILE_WIDTH, top + TILE_HEIGHT),
            action: UiAction::SelectTool(Tool::Shape(shape_type.clone())),
            label: shape_type.display_name().to_string(),
            kind: RegionKind::ShapeTile(shape_type.clone()),
            selected,
            emphasis: Emphasis::default(),
            label_size: None,
        });
    }
}

// ------------------------------------------------------------ right panel

/// The properties of whatever is selected.
///
/// Laid out the way Lucidchart's right panel is: colour, then the line's
/// weight and style, then the connector's routing, then text. Controls show
/// what they do rather than naming it — the weight picker draws the weights,
/// the style picker draws the dashes, the bold button is bold — and every row
/// marks the value the selection currently has.
fn build_properties(layout: &Layout, editor: &Editor, view: &ViewState, out: &mut Vec<Region>) {
    use dc_model::{StrokeStyle, TextAlign, VerticalAlign};

    let panel = layout.right_panel;
    if panel.width() < 120.0 {
        return;
    }
    let x0 = panel.x0 + PAD;
    let x1 = panel.x1 - PAD;
    let half = x0 + (x1 - x0) / 2.0;
    let mut y = panel.y0 + PAD;

    let heading = |label: &str, y: &mut f64, out: &mut Vec<Region>| {
        out.push(Region {
            rect: Rect::new(x0, *y, x1, *y + 16.0),
            action: UiAction::NoOp,
            label: label.to_string(),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        *y += 20.0;
    };

    // ------------------------------------------------------------- colour

    heading("Fill", &mut y, out);
    swatches(&FILL_SWATCHES, x0, x1, editor.selection_fill(), UiAction::SetFill, &mut y, out);

    heading("Line", &mut y, out);
    swatches(
        &STROKE_SWATCHES,
        x0,
        x1,
        editor.selection_stroke_color(),
        UiAction::SetStroke,
        &mut y,
        out,
    );

    // --------------------------------------------------- weight and dashes

    heading("Weight", &mut y, out);
    let width = editor.selection_stroke_width();
    segmented(
        &[1.0_f64, 2.0, 3.0, 5.0].map(|w| {
            Segment::preview(
                Preview::Weight(w),
                UiAction::SetStrokeWidth(w),
                width.is_some_and(|c| (c - w).abs() < 0.01),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + PAD;

    heading("Style", &mut y, out);
    let dash = editor.selection_stroke_style();
    segmented(
        &[StrokeStyle::Solid, StrokeStyle::Dashed, StrokeStyle::Dotted].map(|style| {
            Segment::preview(
                Preview::Dash(style),
                UiAction::SetStrokeStyle(style),
                dash == Some(style),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + PAD;

    // Lucidchart's editing bar shows only the controls that apply to what is
    // selected. The same rule here: connector controls when a connector is
    // selected, shape controls when a shape is, and everything when nothing
    // is, so the panel is never an empty box.
    let nothing_selected = editor.selection.is_empty();
    let show_lines = nothing_selected || editor.selection_has_lines();
    let show_shapes = nothing_selected || editor.selection_has_shapes();

    // ---------------------------------------------------------- connectors

    if show_lines {
        heading("Connector", &mut y, out);
        let route = editor.selection_line_type();
        segmented(
            &[LineType::Elbow, LineType::Straight, LineType::Curved].map(|t| {
                Segment::preview(Preview::Route(t), UiAction::SetLineType(t), route == Some(t))
            }),
            x0,
            x1,
            y,
            out,
        );
        y += ROW_HEIGHT + 4.0;

        // Terminators: one picker per end, and a button that turns the
        // connector around. Lucidchart's panel is laid out the same way.
        endpoint_rows(editor, x0, x1, &mut y, out);
        y += PAD;
    }

    // ------------------------------------------------------------- object

    if !show_shapes {
        finish_properties(editor, x0, x1, &mut y, out);
        return;
    }

    heading("Object", &mut y, out);
    let rounding = editor.selection_corner_rounding();
    segmented(
        &[0.0_f64, 4.0, 10.0, 20.0].map(|r| {
            Segment::preview(
                Preview::Corner(r),
                UiAction::SetCornerRounding(r),
                rounding.is_some_and(|c| (c - r).abs() < 0.01),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    let opacity = editor.selection_opacity();
    segmented(
        &[25.0_f64, 50.0, 75.0, 100.0].map(|percent| {
            Segment::preview(
                Preview::Opacity(percent),
                UiAction::SetOpacity(percent),
                opacity.is_some_and(|c| (c - percent).abs() < 0.5),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    // Rotation and mirroring, which the format expresses through the box
    // rather than through a flag, so these are plain commands.
    segmented(
        &[
            Segment::letter("Turn L", Emphasis::default(), UiAction::Rotate(-90.0), false),
            Segment::letter("Turn R", Emphasis::default(), UiAction::Rotate(90.0), false),
            Segment::letter("Flip H", Emphasis::default(), UiAction::Flip(true), false),
            Segment::letter("Flip V", Emphasis::default(), UiAction::Flip(false), false),
        ],
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    // Swapping one shape for another, keeping its box, style, and text. The
    // alternatives offered are its own library's, which is where a shape's
    // natural substitutes are.
    change_shape_row(editor, x0, x1, &mut y, out);
    y += PAD;

    // ---------------------------------------------------------------- text

    heading("Text", &mut y, out);
    segmented(
        &[
            TextToggle::Bold,
            TextToggle::Italic,
            TextToggle::Underline,
            TextToggle::Strikethrough,
        ]
        .map(|toggle| {
            // The letter is set in the style the button applies, which says
            // what the button is for better than any label could.
            Segment::letter(
                match toggle {
                    TextToggle::Bold => "B",
                    TextToggle::Italic => "I",
                    TextToggle::Underline => "U",
                    TextToggle::Strikethrough => "S",
                },
                Emphasis::of(toggle),
                UiAction::ToggleText(toggle),
                editor.text_style_is_on(toggle),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    let family = editor.selection_font_family();
    segmented(
        &FONT_FAMILIES.map(|(label, name)| {
            Segment::letter(
                label,
                Emphasis::default(),
                UiAction::SetFontFamily(name),
                // The markup layer lowercases font-family values on its way
                // through CSS, so the comparison has to ignore case too.
                family.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(name)),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    // Type size on the left, paragraph alignment on the right: two short rows
    // sharing a line, the way the editing bar packs them.
    segmented(
        &[
            Segment::letter("A", Emphasis::default(), UiAction::ScaleTextSize(1.0 / 1.15), false)
                .at_size(9.5),
            Segment::letter("A", Emphasis::default(), UiAction::ScaleTextSize(1.15), false)
                .at_size(14.0),
        ],
        x0,
        half - 3.0,
        y,
        out,
    );
    let align = editor.text_align();
    segmented(
        &[TextAlign::Left, TextAlign::Center, TextAlign::Right].map(|a| {
            Segment::preview(Preview::Align(a), UiAction::SetTextAlign(a), align == Some(a))
        }),
        half + 3.0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    let vertical = editor.vertical_align();
    segmented(
        &[VerticalAlign::Top, VerticalAlign::Center, VerticalAlign::Bottom].map(|a| {
            Segment::preview(
                Preview::VerticalAlign(a),
                UiAction::SetVerticalAlign(a),
                vertical == Some(a),
            )
        }),
        x0,
        x1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    swatches(
        &STROKE_SWATCHES,
        x0,
        x1,
        editor.selection_text_color(),
        UiAction::SetTextColor,
        &mut y,
        out,
    );

    out.push(button(
        Rect::new(x0, y, x1, y + ROW_HEIGHT),
        "Edit label",
        UiAction::EditLabel,
    ));
    y += ROW_HEIGHT + PAD;

    // --------------------------------------------------------------- link

    // Actions are a documented shape property, not a line one, so this is
    // offered only for a single selected shape.
    if editor.selection.len() == 1 && editor.page().shape(&editor.selection[0]).is_some() {
        heading("Link", &mut y, out);

        let current = editor.selection_link();
        let shown = if view.link_edit_focused {
            view.link_edit.clone()
        } else {
            match &current {
                Some(dc_model::Action::Url { url, .. }) => url.clone(),
                Some(dc_model::Action::GotoPage { page_id }) => format!("Page: {page_id}"),
                _ => String::new(),
            }
        };
        out.push(Region {
            rect: Rect::new(x0, y, x1, y + ROW_HEIGHT),
            action: UiAction::FocusLinkField,
            label: shown,
            kind: RegionKind::TextField { focused: view.link_edit_focused, placeholder: "https://…" },
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        y += ROW_HEIGHT + 4.0;

        if current.is_some() {
            out.push(button(
                Rect::new(x0, y, x1, y + ROW_HEIGHT),
                "Remove Link",
                UiAction::ClearLink,
            ));
            y += ROW_HEIGHT + 4.0;
        }
        y += PAD;

        // ----------------------------------------------------- custom data

        heading("Custom Data", &mut y, out);

        for (i, entry) in editor.selection_custom_data().iter().enumerate() {
            let key_focused = view.custom_data_focus == Some(CustomDataFocus::Key(i));
            let value_focused = view.custom_data_focus == Some(CustomDataFocus::Value(i));
            let key_shown = if key_focused { view.custom_data_edit.clone() } else { entry.key.clone() };
            let value_shown =
                if value_focused { view.custom_data_edit.clone() } else { entry.value.clone() };

            out.push(Region {
                rect: Rect::new(x0, y, half - 3.0, y + ROW_HEIGHT),
                action: UiAction::FocusCustomDataKey(i),
                label: key_shown,
                kind: RegionKind::TextField { focused: key_focused, placeholder: "key" },
                selected: false,
                emphasis: Emphasis::default(),
                label_size: None,
            });
            out.push(Region {
                rect: Rect::new(half + 3.0, y, x1, y + ROW_HEIGHT),
                action: UiAction::FocusCustomDataValue(i),
                label: value_shown,
                kind: RegionKind::TextField { focused: value_focused, placeholder: "value" },
                selected: false,
                emphasis: Emphasis::default(),
                label_size: None,
            });
            y += ROW_HEIGHT + 4.0;

            out.push(button(
                Rect::new(x0, y, x1, y + ROW_HEIGHT),
                "Remove Field",
                UiAction::RemoveCustomData(i),
            ));
            y += ROW_HEIGHT + 4.0;
        }

        out.push(button(Rect::new(x0, y, x1, y + ROW_HEIGHT), "Add Field", UiAction::AddCustomData));
        y += ROW_HEIGHT + PAD;
    }

    finish_properties(editor, x0, x1, &mut y, out);
}

/// The rows every selection gets: ordering, and what is selected.
fn finish_properties(
    editor: &Editor,
    x0: f64,
    x1: f64,
    y: &mut f64,
    out: &mut Vec<Region>,
) {
    out.push(Region {
        rect: Rect::new(x0, *y, x1, *y + 16.0),
        action: UiAction::NoOp,
        label: "Arrange".to_string(),
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    *y += 20.0;

    // All four of Lucidchart's ordering commands: the two that move one step
    // and the two that go the whole way.
    segmented(
        &[
            Segment::letter("Back", Emphasis::default(), UiAction::SendToBack, false),
            Segment::letter("−", Emphasis::default(), UiAction::SendBackward, false),
            Segment::letter("+", Emphasis::default(), UiAction::BringForward, false),
            Segment::letter("Front", Emphasis::default(), UiAction::BringToFront, false),
        ],
        x0,
        x1,
        *y,
        out,
    );
    *y += ROW_HEIGHT + 4.0;

    let button_width = ((x1 - x0) - 4.0) / 2.0;
    out.push(button(
        Rect::new(x0, *y, x0 + button_width, *y + ROW_HEIGHT),
        "Copy style",
        UiAction::CopyStyle,
    ));
    out.push(button(
        Rect::new(x0 + button_width + 4.0, *y, x1, *y + ROW_HEIGHT),
        "Paste style",
        UiAction::PasteStyle,
    ));
    *y += ROW_HEIGHT + PAD;

    // A short readout of what is selected.
    let summary = match editor.selection.len() {
        0 => "Nothing selected".to_string(),
        1 => {
            let id = &editor.selection[0];
            match editor.page().shape(id) {
                Some(s) => s.shape_type.display_name().to_string(),
                None => "Connector".to_string(),
            }
        }
        n => format!("{n} items selected"),
    };
    out.push(Region {
        rect: Rect::new(x0, *y, x1, *y + 18.0),
        action: UiAction::NoOp,
        label: summary,
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
}

/// Alternatives to the selected shape, drawn as their own outlines.
///
/// Only when a single shape is selected: with several, "change shape" has no
/// obvious starting point to offer substitutes for.
fn change_shape_row(editor: &Editor, x0: f64, x1: f64, y: &mut f64, out: &mut Vec<Region>) {
    let Some(current) = editor
        .selection
        .first()
        .filter(|_| editor.selection.len() == 1)
        .and_then(|id| editor.page().shape(id))
        .map(|s| s.shape_type.clone())
    else {
        return;
    };

    // Its own library's types are where a shape's natural substitutes are,
    // and the ones nearest it are the ones worth offering first.
    let family = current.library().types();
    let at = family.iter().position(|t| *t == current).unwrap_or(0);
    let alternatives: Vec<ShapeType> = family
        .iter()
        .cycle()
        .skip(at + 1)
        .take(4)
        .cloned()
        .collect();
    if alternatives.is_empty() {
        return;
    }

    segmented(
        &alternatives
            .into_iter()
            .map(|t| {
                Segment::preview(
                    Preview::Outline(t.clone()),
                    UiAction::ChangeShapeType(t),
                    false,
                )
            })
            .collect::<Vec<_>>(),
        x0,
        x1,
        *y,
        out,
    );
    *y += ROW_HEIGHT + 4.0;
}

/// The terminators offered in the panel.
///
/// The format publishes twenty-two, which is more than a panel row can hold;
/// these are the ones a diagram actually reaches for, and the rest stay
/// reachable by loading a document that uses them and are preserved on save.
const ENDPOINT_CHOICES: [dc_model::EndpointStyle; 5] = [
    dc_model::EndpointStyle::None,
    dc_model::EndpointStyle::Arrow,
    dc_model::EndpointStyle::OpenArrow,
    dc_model::EndpointStyle::HollowArrow,
    dc_model::EndpointStyle::ClosedCircle,
];

/// The two endpoint pickers and the button that turns a connector around.
fn endpoint_rows(editor: &Editor, x0: f64, x1: f64, y: &mut f64, out: &mut Vec<Region>) {
    use crate::editor::LineEnd;

    for end in [LineEnd::Start, LineEnd::End] {
        let current = editor.selection_endpoint_style(end);
        // The start terminator points back along the line and the end one
        // points forward, which is how they are drawn on the canvas.
        let pointing_right = end == LineEnd::End;
        segmented(
            &ENDPOINT_CHOICES.map(|style| {
                Segment::preview(
                    Preview::Endpoint { style: style.clone(), pointing_right },
                    UiAction::SetEndpointStyle(end, style.clone()),
                    current.as_ref() == Some(&style),
                )
            }),
            x0,
            x1,
            *y,
            out,
        );
        *y += ROW_HEIGHT + 4.0;
    }

    out.push(button(
        Rect::new(x0, *y, x1, *y + ROW_HEIGHT),
        "Reverse direction",
        UiAction::SwapEndpoints,
    ));
    *y += ROW_HEIGHT + 4.0;
}

/// One choice in a segmented row.
struct Segment {
    label: String,
    /// Drawn in place of the label, when the choice shows what it does.
    preview: Option<Preview>,
    emphasis: Emphasis,
    size: Option<f64>,
    action: UiAction,
    active: bool,
}

impl Segment {
    fn preview(preview: Preview, action: UiAction, active: bool) -> Self {
        Segment {
            label: String::new(),
            preview: Some(preview),
            emphasis: Emphasis::default(),
            size: None,
            action,
            active,
        }
    }

    fn letter(label: &str, emphasis: Emphasis, action: UiAction, active: bool) -> Self {
        Segment {
            label: label.to_string(),
            preview: None,
            emphasis,
            size: None,
            action,
            active,
        }
    }

    fn at_size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }
}

/// A row of equal-width choices spanning `x0` to `x1`, at `y`.
///
/// The row does not advance `y`: two half-width rows share a line by being
/// given the same one.
fn segmented(segments: &[Segment], x0: f64, x1: f64, y: f64, out: &mut Vec<Region>) {
    if segments.is_empty() {
        return;
    }
    let gap = 3.0;
    let each = (((x1 - x0) - gap * (segments.len() - 1) as f64) / segments.len() as f64).max(1.0);

    for (i, segment) in segments.iter().enumerate() {
        let sx = x0 + i as f64 * (each + gap);
        out.push(Region {
            rect: Rect::new(sx, y, sx + each, y + ROW_HEIGHT),
            action: segment.action.clone(),
            label: segment.label.clone(),
            kind: match &segment.preview {
                Some(preview) => RegionKind::PreviewChoice {
                    active: segment.active,
                    preview: preview.clone(),
                },
                None => RegionKind::Choice { active: segment.active },
            },
            selected: false,
            emphasis: segment.emphasis,
            label_size: segment.size,
        });
    }
}

/// A grid of colour chips, with the selection's own colour ringed.
#[allow(clippy::too_many_arguments)]
fn swatches(
    colors: &[Color],
    x0: f64,
    x1: f64,
    current: Option<Color>,
    action: impl Fn(Color) -> UiAction,
    y: &mut f64,
    out: &mut Vec<Region>,
) {
    let per_row = (((x1 - x0) + 4.0) / (SWATCH + 4.0)).floor().max(1.0) as usize;
    for (i, color) in colors.iter().enumerate() {
        let col = i % per_row;
        let row = i / per_row;
        let sx = x0 + col as f64 * (SWATCH + 4.0);
        let sy = *y + row as f64 * (SWATCH + 4.0);
        out.push(Region {
            rect: Rect::new(sx, sy, sx + SWATCH, sy + SWATCH),
            action: action(*color),
            label: String::new(),
            kind: RegionKind::Swatch(*color),
            selected: current == Some(*color),
            emphasis: Emphasis::default(),
            label_size: None,
        });
    }
    *y += colors.len().div_ceil(per_row) as f64 * (SWATCH + 4.0) + PAD;
}

// -------------------------------------------------------------- page tabs

/// Lays the page tabs out and returns where each landed, so a drag can be
/// resolved to a drop index.
fn build_tabs(
    layout: &Layout,
    editor: &Editor,
    drag: Option<TabDrag>,
    out: &mut Vec<Region>,
) -> Vec<Rect> {
    let bar = layout.tabs;
    let height = (bar.height() - 6.0).max(16.0);
    let y = bar.y0 + 3.0;
    let mut x = PAD;
    let mut rects = Vec::new();

    for (i, page) in editor.document().pages.iter().enumerate() {
        if x + TAB_WIDTH > bar.x1 - 40.0 {
            break;
        }
        let rect = Rect::new(x, y, x + TAB_WIDTH, y + height);
        rects.push(rect);

        // The tab being dragged is dimmed in place; the drop marker shows
        // where it would land.
        let dragging = drag.is_some_and(|d| d.from == i);
        out.push(Region {
            rect,
            action: UiAction::SelectPage(i),
            label: page.title.clone(),
            kind: RegionKind::Tab { active: i == editor.page_index && !dragging },
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        x += TAB_WIDTH + 4.0;
    }

    out.push(button(Rect::new(x, y, x + 28.0, y + height), "+", UiAction::AddPage));
    x += 32.0;

    // Page operations act on the current page.
    let current = editor.page_index;
    out.push(button(
        Rect::new(x, y, x + 76.0, y + height),
        "Duplicate",
        UiAction::DuplicatePage(current),
    ));
    x += 80.0;

    // Removing the only page would leave a document the format cannot
    // represent, so the control is simply absent then.
    if editor.document().pages.len() > 1 {
        out.push(button(
            Rect::new(x, y, x + 60.0, y + height),
            "Delete",
            UiAction::DeletePage(current),
        ));
    }

    // A bar at the insertion point, drawn last so it sits over the tabs.
    if let Some(drag) = drag {
        if let Some(target) = drop_marker_x(&rects, drag) {
            out.push(Region {
                rect: Rect::new(target - 1.5, y, target + 1.5, y + height),
                action: UiAction::NoOp,
                label: String::new(),
                kind: RegionKind::DropMarker,
                selected: false,
                emphasis: Emphasis::default(),
                label_size: None,
            });
        }
    }

    rects
}

/// Which page tab a pointer at `x` is over, or the nearest one.
fn tab_index_in(rects: &[Rect], x: f64) -> Option<usize> {
    let first = rects.first()?;
    // Directly over a tab is that tab.
    if let Some(index) = rects.iter().position(|r| x >= r.x0 && x < r.x1) {
        return Some(index);
    }
    // Otherwise the nearest end of the row; a gap between two tabs rounds to
    // whichever one follows it.
    if x < first.x0 {
        return Some(0);
    }
    rects
        .iter()
        .position(|r| x < r.x0)
        .or(Some(rects.len() - 1))
}

/// Where the drop marker goes for a drag in progress.
///
/// The marker has to show where the tab will actually land, which means
/// resolving the pointer the same way the drop does and then translating that
/// final index back into a position among the tabs as they are drawn now —
/// a list that still contains the tab being dragged.
fn drop_marker_x(rects: &[Rect], drag: TabDrag) -> Option<f64> {
    if rects.len() < 2 {
        return None;
    }
    let to = tab_index_in(rects, drag.pointer_x)?;
    if to == drag.from {
        return None;
    }

    // Dropping at `to` means landing before whatever is at `to` once the
    // dragged tab is lifted out, which is one tab further right when the tab
    // came from the left of it.
    let before = if to < drag.from { to } else { to + 1 };
    Some(match rects.get(before) {
        Some(rect) => rect.x0 - 2.0,
        None => rects[rects.len() - 1].x1 + 2.0,
    })
}

// -------------------------------------------------------------- find bar

/// A floating bar for finding and replacing text in labels: a query field
/// with Previous/Next, and a replacement field with a Replace button.
/// Floats over the canvas near the top, out of the layers panel's way at
/// the top right.
fn build_find_bar(layout: &Layout, editor: &Editor, view: &ViewState, out: &mut Vec<Region>) -> Rect {
    let width = 420.0_f64.min(layout.viewport.x - 24.0).max(240.0);
    let x0 = ((layout.viewport.x - width) / 2.0).max(12.0);
    let x1 = x0 + width;
    let y0 = layout.menubar.y1 + 12.0;
    let y1 = y0 + PAD * 2.0 + ROW_HEIGHT * 2.0 + 18.0 + 12.0;
    let panel = Rect::new(x0, y0, x1, y1);

    out.push(Region {
        rect: panel,
        action: UiAction::NoOp,
        label: String::new(),
        kind: RegionKind::Panel,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let px0 = x0 + PAD;
    let px1 = x1 - PAD;
    let mut y = y0 + PAD;

    // Query field, with Previous/Next sharing its row.
    let nav_width = 70.0;
    out.push(Region {
        rect: Rect::new(px0, y, px1 - nav_width * 2.0 - 8.0, y + ROW_HEIGHT),
        action: UiAction::FocusFindQuery,
        label: view.find_query.clone(),
        kind: RegionKind::TextField { focused: view.find_query_focused, placeholder: "Find in labels" },
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    segmented(
        &[
            Segment::letter("Prev", Emphasis::default(), UiAction::FindPrevious, false),
            Segment::letter("Next", Emphasis::default(), UiAction::FindNext, false),
        ],
        px1 - nav_width * 2.0 - 4.0,
        px1,
        y,
        out,
    );
    y += ROW_HEIGHT + 4.0;

    let matches = editor.find_matches(&view.find_query);
    let count_label = if view.find_query.trim().is_empty() {
        String::new()
    } else if matches.is_empty() {
        "No matches".to_string()
    } else {
        format!("Match {} of {}", view.find_match_index.min(matches.len() - 1) + 1, matches.len())
    };
    out.push(Region {
        rect: Rect::new(px0, y, px1, y + 18.0),
        action: UiAction::NoOp,
        label: count_label,
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 18.0 + 8.0;

    // Replacement field, with Replace sharing its row.
    let replace_width = 80.0;
    out.push(Region {
        rect: Rect::new(px0, y, px1 - replace_width - 4.0, y + ROW_HEIGHT),
        action: UiAction::FocusFindReplacement,
        label: view.find_replacement.clone(),
        kind: RegionKind::TextField { focused: view.find_replacement_focused, placeholder: "Replace with" },
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    out.push(button(
        Rect::new(px1 - replace_width, y, px1, y + ROW_HEIGHT),
        "Replace",
        UiAction::ReplaceCurrentMatch,
    ));

    panel
}

// ----------------------------------------------------------- layers panel

/// A floating panel listing the page's layers: visibility, lock, delete, and
/// moving the current selection into one. Floats over the canvas rather than
/// claiming a `Layout` region, so opening it never disturbs the camera.
fn build_layers_panel(layout: &Layout, editor: &Editor, out: &mut Vec<Region>) -> Rect {
    let width = 240.0_f64.min(layout.viewport.x - 24.0).max(160.0);
    let x1 = layout.viewport.x - 12.0;
    let x0 = (x1 - width).max(12.0);
    let y0 = layout.menubar.y1 + 12.0;
    let max_y1 = (layout.tabs.y0 - 12.0).max(y0 + 40.0);

    let layers = editor.layers();
    let per_entry = ROW_HEIGHT + 4.0 + ROW_HEIGHT + 8.0;
    let content_height = PAD * 2.0 + 22.0 + layers.len() as f64 * per_entry + ROW_HEIGHT;
    let y1 = (y0 + content_height).min(max_y1);
    let panel = Rect::new(x0, y0, x1, y1);

    out.push(Region {
        rect: panel,
        action: UiAction::NoOp,
        label: String::new(),
        kind: RegionKind::Panel,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let px0 = x0 + PAD;
    let px1 = x1 - PAD;
    let mut y = y0 + PAD;

    out.push(Region {
        rect: Rect::new(px0, y, px1, y + 18.0),
        action: UiAction::NoOp,
        label: "Layers".to_string(),
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 22.0;

    let has_selection = !editor.selection.is_empty();
    for layer in layers {
        if y + ROW_HEIGHT > panel.y1 {
            break;
        }
        out.push(Region {
            rect: Rect::new(px0, y, px1, y + ROW_HEIGHT),
            action: UiAction::NoOp,
            label: layer.title.clone().unwrap_or_else(|| layer.id.clone()),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        y += ROW_HEIGHT + 4.0;
        if y + ROW_HEIGHT > panel.y1 {
            break;
        }

        let mut segments = vec![
            Segment::letter(
                "Vis",
                Emphasis::default(),
                UiAction::ToggleLayerVisibility(layer.id.clone()),
                layer.is_visible(),
            ),
            Segment::letter(
                "Lock",
                Emphasis::default(),
                UiAction::ToggleLayerLock(layer.id.clone()),
                layer.is_locked(),
            ),
            Segment::letter("Del", Emphasis::default(), UiAction::DeleteLayer(layer.id.clone()), false),
        ];
        if has_selection {
            segments.insert(
                0,
                Segment::letter(
                    "Use",
                    Emphasis::default(),
                    UiAction::MoveSelectionToLayer(layer.id.clone()),
                    false,
                ),
            );
        }
        segmented(&segments, px0, px1, y, out);
        y += ROW_HEIGHT + 8.0;
    }

    if y + ROW_HEIGHT <= panel.y1 {
        out.push(button(Rect::new(px0, y, px1, y + ROW_HEIGHT), "Add Layer", UiAction::AddLayer));
    }

    panel
}

// --------------------------------------------------------- settings screen

/// Height of one settings row.
// ------------------------------------------------------- template gallery

/// Height of one template's row: its name and description stacked.
const TEMPLATE_ROW: f64 = 56.0;
const TEMPLATE_GALLERY_WIDTH: f64 = 440.0;

/// Lays out the "New from Template" gallery as a modal sheet, the same
/// construction `build_settings` uses: a scrim that closes it on a miss,
/// and a sheet sized to its content and clamped to the window.
fn build_template_gallery(layout: &Layout, out: &mut Vec<Region>) -> Rect {
    let window = Rect::new(0.0, 0.0, layout.viewport.x, layout.viewport.y);
    out.push(Region {
        rect: window,
        action: UiAction::CloseTemplateGallery,
        label: String::new(),
        kind: RegionKind::Scrim,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let width = TEMPLATE_GALLERY_WIDTH.clamp(1.0, layout.viewport.x - 40.0);
    let content_height =
        16.0 + 24.0 + 10.0 + crate::templates::ALL.len() as f64 * (TEMPLATE_ROW + 8.0) + 16.0 + ROW_HEIGHT + 16.0;
    let height = content_height.clamp(1.0, layout.viewport.y - 40.0);
    let sheet = Rect::from_center_size(
        kurbo::Point::new(layout.viewport.x / 2.0, layout.viewport.y / 2.0),
        (width, height),
    );
    out.push(Region {
        rect: sheet,
        action: UiAction::NoOp,
        label: String::new(),
        kind: RegionKind::Sheet,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let x0 = sheet.x0 + 20.0;
    let x1 = sheet.x1 - 20.0;
    let mut y = sheet.y0 + 16.0;

    out.push(Region {
        rect: Rect::new(x0, y, x1, y + 24.0),
        action: UiAction::NoOp,
        label: "New from Template".into(),
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 34.0;

    let content_limit = sheet.y1 - 16.0 - ROW_HEIGHT - 12.0;
    for (i, t) in crate::templates::ALL.iter().enumerate() {
        if y + TEMPLATE_ROW > content_limit {
            break;
        }
        // The whole row is one clickable surface; the name and description
        // are drawn over it as plain, non-clickable text, the same way the
        // layers panel draws a title over its own backdrop.
        out.push(Region {
            rect: Rect::new(x0, y, x1, y + TEMPLATE_ROW),
            action: UiAction::NewFromTemplate(i),
            label: String::new(),
            kind: RegionKind::Button,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        out.push(Region {
            rect: Rect::new(x0 + 12.0, y + 8.0, x1 - 12.0, y + 28.0),
            action: UiAction::NoOp,
            label: t.name.to_string(),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        out.push(Region {
            rect: Rect::new(x0 + 12.0, y + 30.0, x1 - 12.0, y + 48.0),
            action: UiAction::NoOp,
            label: t.description.to_string(),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        y += TEMPLATE_ROW + 8.0;
    }

    let footer = sheet.y1 - 16.0 - ROW_HEIGHT;
    out.push(button(
        Rect::new(x1 - 90.0, footer, x1, footer + ROW_HEIGHT),
        "Cancel",
        UiAction::CloseTemplateGallery,
    ));

    sheet
}

// --------------------------------------------------------- settings screen

/// Height of one settings row.
const SETTING_ROW: f64 = 30.0;
/// How wide the labels column is, so every control lines up.
const SETTING_LABEL: f64 = 150.0;
/// The settings sheet at its full size, which is what its content comes to.
const SETTINGS_WIDTH: f64 = 520.0;
const SETTINGS_HEIGHT: f64 = 700.0;

/// Lays out the settings screen over the window, and returns the sheet.
///
/// The sections follow the ones Lucidchart's document settings use, so
/// someone arriving from that product finds each preference under the heading
/// they expect. Every control here changes something the editor honours.
fn build_settings(layout: &Layout, view: &ViewState, out: &mut Vec<Region>) -> Rect {
    use crate::settings::{self, GRID_PITCHES, PAGE_SIZES, SNAP_DISTANCES, ZOOM_STEPS};
    use dc_render::GridStyle;

    let s = &view.settings;
    let window = Rect::new(0.0, 0.0, layout.viewport.x, layout.viewport.y);

    // The scrim catches every click that misses the sheet, which is what stops
    // the editor underneath from reacting to a stray press.
    out.push(Region {
        rect: window,
        action: UiAction::CloseSettings,
        label: String::new(),
        kind: RegionKind::Scrim,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    // Sized to its content, and never larger than the window it sits in. A
    // window too small for the whole screen gets as much of it as fits; the
    // rows below stop rather than spilling past the footer.
    let width = (layout.viewport.x - 40.0).clamp(1.0, SETTINGS_WIDTH);
    let height = (layout.viewport.y - 40.0).clamp(1.0, SETTINGS_HEIGHT);
    let sheet = Rect::from_center_size(
        kurbo::Point::new(layout.viewport.x / 2.0, layout.viewport.y / 2.0),
        (width, height),
    );
    // Rows stop here so they cannot run under the footer buttons.
    let content_limit = sheet.y1 - 20.0 - ROW_HEIGHT;
    out.push(Region {
        rect: sheet,
        action: UiAction::NoOp,
        label: String::new(),
        kind: RegionKind::Sheet,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });

    let x0 = sheet.x0 + 20.0;
    let x1 = sheet.x1 - 20.0;
    let mut y = sheet.y0 + 16.0;

    // Title.
    out.push(Region {
        rect: Rect::new(x0, y, x1, y + 24.0),
        action: UiAction::NoOp,
        label: "Settings".into(),
        kind: RegionKind::Heading,
        selected: false,
        emphasis: Emphasis::default(),
        label_size: None,
    });
    y += 34.0;

    let section = |title: &str, y: &mut f64, out: &mut Vec<Region>| {
        if *y + 18.0 > content_limit {
            return;
        }
        out.push(Region {
            rect: Rect::new(x0, *y, x1, *y + 18.0),
            action: UiAction::NoOp,
            label: title.to_string(),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });
        *y += 22.0;
    };

    /// One row: a label on the left, a run of choices on the right.
    ///
    /// A row that would reach past `limit` is left out rather than drawn over
    /// the footer, the same way the shape palette stops at the panel edge.
    #[allow(clippy::too_many_arguments)]
    fn row(
        label: &str,
        choices: &[(String, UiAction, bool)],
        x0: f64,
        x1: f64,
        limit: f64,
        y: &mut f64,
        out: &mut Vec<Region>,
    ) {
        if *y + SETTING_ROW > limit {
            return;
        }
        out.push(Region {
            rect: Rect::new(x0, *y, x0 + SETTING_LABEL, *y + SETTING_ROW),
            action: UiAction::NoOp,
            label: label.to_string(),
            kind: RegionKind::Heading,
            selected: false,
            emphasis: Emphasis::default(),
            label_size: None,
        });

        let left = x0 + SETTING_LABEL;
        let span = (x1 - left).max(1.0);
        let gap = 4.0;
        let each = ((span - gap * (choices.len().max(1) - 1) as f64)
            / choices.len().max(1) as f64)
            .max(1.0);

        for (i, (text, action, active)) in choices.iter().enumerate() {
            let cx = left + i as f64 * (each + gap);
            out.push(Region {
                rect: Rect::new(cx, *y + 2.0, cx + each, *y + SETTING_ROW - 2.0),
                action: action.clone(),
                label: text.clone(),
                kind: RegionKind::Choice { active: *active },
                selected: false,
                emphasis: Emphasis::default(),
                label_size: None,
            });
        }
        *y += SETTING_ROW + 4.0;
    }

    /// An on/off row, written as two choices so it reads like the rest.
    #[allow(clippy::too_many_arguments)]
    fn switch(
        label: &str,
        on: bool,
        change: impl Fn(bool) -> SettingChange,
        x0: f64,
        x1: f64,
        limit: f64,
        y: &mut f64,
        out: &mut Vec<Region>,
    ) {
        row(
            label,
            &[
                ("On".into(), UiAction::ChangeSetting(change(true)), on),
                ("Off".into(), UiAction::ChangeSetting(change(false)), !on),
            ],
            x0,
            x1,
            limit,
            y,
            out,
        );
    }

    section("Appearance", &mut y, out);
    row(
        "Theme",
        &[
            (
                "Light".into(),
                UiAction::ChangeSetting(SettingChange::DarkTheme(false)),
                !s.dark_theme,
            ),
            (
                "Dark".into(),
                UiAction::ChangeSetting(SettingChange::DarkTheme(true)),
                s.dark_theme,
            ),
        ],
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    switch(
        "Show page bounds",
        s.show_page_bounds,
        SettingChange::ShowPageBounds,
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    y += 6.0;

    section("Grid and snapping", &mut y, out);
    switch("Show grid", s.show_grid, SettingChange::ShowGrid, x0, x1, content_limit, &mut y, out);
    row(
        "Grid style",
        [GridStyle::Graph, GridStyle::Lines, GridStyle::Dots]
            .map(|style| {
                (
                    style.label().to_string(),
                    UiAction::ChangeSetting(SettingChange::GridStyle(style)),
                    s.grid_style == style,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    row(
        "Grid size",
        GRID_PITCHES
            .map(|pitch| {
                (
                    settings::pitch_label(pitch),
                    UiAction::ChangeSetting(SettingChange::GridSpacing(pitch)),
                    (s.grid_spacing - pitch).abs() < 0.01,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    switch(
        "Snap to objects",
        s.snap_to_objects,
        SettingChange::SnapToObjects,
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    switch(
        "Snap to grid",
        s.snap_to_grid,
        SettingChange::SnapToGrid,
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    row(
        "Snap distance",
        SNAP_DISTANCES
            .map(|d| {
                (
                    format!("{d:.0}"),
                    UiAction::ChangeSetting(SettingChange::SnapDistance(d)),
                    (s.snap_distance - d).abs() < 0.01,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    y += 6.0;

    section("Lines", &mut y, out);
    row(
        "Default routing",
        [LineType::Elbow, LineType::Straight, LineType::Curved]
            .map(|t| {
                (
                    settings::line_type_label(t).to_string(),
                    UiAction::ChangeSetting(SettingChange::DefaultLineType(t)),
                    s.default_line_type == t,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    switch(
        "Route around shapes",
        s.route_around_shapes,
        SettingChange::RouteAroundShapes,
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    y += 6.0;

    section("Zoom", &mut y, out);
    row(
        "Step size",
        ZOOM_STEPS
            .map(|step| {
                (
                    format!("{:.0}%", (step - 1.0) * 100.0),
                    UiAction::ChangeSetting(SettingChange::ZoomStep(step)),
                    (s.zoom_step - step).abs() < 0.001,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    y += 6.0;

    section("New documents", &mut y, out);
    row(
        "Units",
        [
            dc_model::Units::In,
            dc_model::Units::Cm,
            dc_model::Units::Pt,
            dc_model::Units::Px,
        ]
        .map(|u| {
            (
                settings::units_label(u).to_string(),
                UiAction::ChangeSetting(SettingChange::DefaultUnits(u)),
                s.default_units == u,
            )
        })
        .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );
    row(
        "Page size",
        PAGE_SIZES
            .map(|name| {
                (
                    settings::page_size_label(name),
                    UiAction::ChangeSetting(SettingChange::DefaultPageSize(name.to_string())),
                    s.default_page_size == name,
                )
            })
            .as_slice(),
        x0,
        x1,
        content_limit,
        &mut y,
        out,
    );

    // The footer sits on the bottom edge whatever the sections above came to.
    let footer = sheet.y1 - 16.0 - ROW_HEIGHT;
    out.push(button(
        Rect::new(x0, footer, x0 + 130.0, footer + ROW_HEIGHT),
        "Restore defaults",
        UiAction::ChangeSetting(SettingChange::RestoreDefaults),
    ));
    out.push(button(
        Rect::new(x1 - 70.0, footer, x1, footer + ROW_HEIGHT),
        "Done",
        UiAction::CloseSettings,
    ));

    sheet
}

fn px(c: Color) -> peniko::Color {
    peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::Point;

    fn window() -> Vec2 {
        Vec2::new(1440.0, 900.0)
    }

    fn editor() -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(900.0, 800.0);
        e
    }

    fn chrome(e: &Editor) -> Chrome {
        Chrome::build_for(window(), e, &Metrics::default(), &ViewState::default(), None, None, None)
    }

    fn chrome_with_menu(e: &Editor, menu: usize) -> Chrome {
        Chrome::build_for(
            window(),
            e,
            &Metrics::default(),
            &ViewState::default(),
            Some(menu),
            None,
            None,
        )
    }

    fn chrome_with_library(e: &Editor, library: ShapeLibrary) -> Chrome {
        let view = ViewState { library, ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    fn chrome_with_search(e: &Editor, query: &str) -> Chrome {
        let view = ViewState { palette_search: query.to_string(), ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    fn shape_tiles(c: &Chrome) -> Vec<&ShapeType> {
        c.regions
            .iter()
            .filter_map(|r| match &r.kind {
                RegionKind::ShapeTile(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_search_field_is_always_in_the_palette() {
        let c = chrome(&editor());
        let field = c
            .regions
            .iter()
            .find(|r| matches!(r.kind, RegionKind::TextField { .. }))
            .expect("a search field");
        assert_eq!(field.action, UiAction::FocusPaletteSearch);
    }

    #[test]
    fn searching_finds_shapes_across_every_library() {
        // "Process" lives in Flowchart, "Terminator" in Flowchart too, but
        // the point is finding a shape without knowing which library it is
        // in — search a term and get hits regardless of library.
        let c = chrome_with_search(&editor(), "star");
        let tiles = shape_tiles(&c);
        assert!(!tiles.is_empty(), "expected at least one match for \"star\"");
        for t in &tiles {
            assert!(
                t.display_name().to_ascii_lowercase().contains("star"),
                "{} does not match the query",
                t.display_name()
            );
        }

        // With a query active, the library tabs are not shown at all —
        // there is nothing to browse by library while searching.
        assert!(!c.regions.iter().any(|r| matches!(r.kind, RegionKind::LibraryTab { .. })));
    }

    #[test]
    fn an_empty_search_shows_the_current_library_as_usual() {
        let e = editor();
        let with_query = chrome_with_search(&e, "");
        let without = chrome(&e);
        assert_eq!(shape_tiles(&with_query).len(), shape_tiles(&without).len());
    }

    #[test]
    fn a_search_with_no_matches_offers_no_tiles() {
        let c = chrome_with_search(&editor(), "zzzznotashape");
        assert!(shape_tiles(&c).is_empty());
    }

    #[test]
    fn layout_partitions_the_window_without_overlap() {
        let l = Layout::compute(window(), &Metrics::default());

        assert!(l.canvas.x0 >= l.left_panel.x1 - 0.01);
        assert!(l.canvas.x1 <= l.right_panel.x0 + 0.01);
        assert!(l.canvas.y0 >= l.menubar.y1 - 0.01);
        assert!(l.canvas.y1 <= l.tabs.y0 + 0.01);
        assert!(l.canvas.width() > 0.0 && l.canvas.height() > 0.0);
    }

    #[test]
    fn layout_survives_a_tiny_window() {
        let l = Layout::compute(Vec2::new(60.0, 40.0), &Metrics::default());
        assert!(l.canvas.width() >= 0.0);
        assert!(l.canvas.height() >= 0.0);
        assert!(l.menubar.height() <= 40.0);
    }

    #[test]
    fn panels_do_not_consume_the_whole_window() {
        let l = Layout::compute(Vec2::new(400.0, 400.0), &Metrics::default());
        assert!(
            l.canvas.width() > 100.0,
            "canvas collapsed to {}",
            l.canvas.width()
        );
    }

    // ------------------------------------------------------------ menu bar

    #[test]
    fn the_menu_bar_offers_every_menu() {
        let c = chrome(&editor());
        let titles: Vec<&str> = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::MenuTitle { .. }))
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["File", "Edit", "Select", "Insert", "Format", "Arrange", "View", "Help"]
        );
    }

    #[test]
    fn menu_titles_do_not_overlap_each_other() {
        let c = chrome(&editor());
        let rects: Vec<Rect> = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::MenuTitle { .. }))
            .map(|r| r.rect)
            .collect();
        for pair in rects.windows(2) {
            assert!(pair[0].x1 <= pair[1].x0, "{:?} overlaps {:?}", pair[0], pair[1]);
        }
        assert!(rects.last().unwrap().x1 < window().x, "the bar overflows");
    }

    #[test]
    fn menu_titles_fit_inside_the_bar() {
        let c = chrome(&editor());
        for region in &c.regions {
            if matches!(region.kind, RegionKind::MenuTitle { .. }) {
                assert!(
                    region.rect.y0 >= c.layout.menubar.y0 && region.rect.y1 <= c.layout.menubar.y1,
                    "{:?} escapes the menu bar",
                    region.label
                );
            }
        }
    }

    #[test]
    fn no_menu_is_open_by_default() {
        let c = chrome(&editor());
        assert!(c.open_menu.is_none());
        assert!(
            !c.regions.iter().any(|r| matches!(r.kind, RegionKind::MenuEntry { .. })),
            "a closed menu should contribute no entries"
        );
    }

    #[test]
    fn opening_a_menu_lays_out_its_entries() {
        let c = chrome_with_menu(&editor(), 0);
        let (index, panel) = c.open_menu.expect("a menu should be open");
        assert_eq!(index, 0);

        let entries: Vec<&Region> = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::MenuEntry { .. }))
            .collect();
        assert!(entries.len() > 5, "got {}", entries.len());
        for entry in entries {
            assert!(
                panel.union(entry.rect) == panel,
                "{:?} escapes its panel",
                entry.label
            );
        }
    }

    #[test]
    fn an_open_menu_hangs_below_its_title() {
        let c = chrome_with_menu(&editor(), 2);
        let (_, panel) = c.open_menu.unwrap();
        let title = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::MenuTitle { .. }))
            .nth(2)
            .unwrap();
        assert!(panel.y0 >= title.rect.y1, "the panel should not cover its own title");
        assert!((panel.x0 - title.rect.x0).abs() < 1.0, "and should line up with it");
    }

    #[test]
    fn a_menu_near_the_right_edge_flips_to_stay_on_screen() {
        let e = editor();
        let narrow = Vec2::new(520.0, 700.0);
        let c = Chrome::build_for(
            narrow,
            &e,
            &Metrics::default(),
            &ViewState::default(),
            Some(7),
            None,
            None,
        );
        let (_, panel) = c.open_menu.unwrap();
        assert!(panel.x1 <= narrow.x + 0.01, "the panel ran off at {}", panel.x1);
        assert!(panel.x0 >= 0.0);
    }

    // ------------------------------------------------------- context menu

    fn chrome_with_context_menu(e: &Editor, target: ContextTarget, at: kurbo::Point) -> Chrome {
        Chrome::build_for(
            window(),
            e,
            &Metrics::default(),
            &ViewState::default(),
            None,
            None,
            Some((target, at)),
        )
    }

    #[test]
    fn a_context_menu_opens_at_the_click_point() {
        let e = editor();
        let at = kurbo::Point::new(200.0, 150.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Canvas, at);
        let panel = c.context_menu.expect("a context menu should be up");
        assert!((panel.x0 - at.x).abs() < 0.5);
        assert!((panel.y0 - at.y).abs() < 0.5);
    }

    #[test]
    fn a_context_menu_near_the_bottom_flips_upward_rather_than_clip() {
        let e = editor();
        let w = window();
        let at = kurbo::Point::new(200.0, w.y - 10.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Canvas, at);
        let panel = c.context_menu.unwrap();
        assert!(panel.y1 <= w.y + 0.01, "the panel ran off the bottom at {}", panel.y1);
        assert!(panel.y1 <= at.y + 0.01, "it should open above the click, not below it");
    }

    #[test]
    fn a_context_menu_near_the_right_edge_flips_left() {
        let e = editor();
        let w = window();
        let at = kurbo::Point::new(w.x - 5.0, 200.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Canvas, at);
        let panel = c.context_menu.unwrap();
        assert!(panel.x1 <= w.x + 0.01, "the panel ran off the right edge at {}", panel.x1);
        assert!(panel.x0 >= 0.0);
    }

    #[test]
    fn a_context_menu_is_opaque_for_hit_testing() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        let at = kurbo::Point::new(200.0, 150.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Canvas, at);
        let panel = c.context_menu.unwrap();

        let hit = c.hit(panel.center());
        let kind = hit.map(|i| c.regions[i].kind.clone());
        assert!(
            matches!(kind, Some(RegionKind::MenuEntry { .. })),
            "a point inside the panel should only ever reach the menu's own entries"
        );
    }

    #[test]
    fn a_point_inside_the_context_menu_is_not_canvas() {
        let e = editor();
        let at = kurbo::Point::new(200.0, 150.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Canvas, at);
        let panel = c.context_menu.unwrap();
        assert!(!c.is_over_canvas(panel.center()));
    }

    #[test]
    fn a_page_tab_context_menu_offers_page_commands() {
        let mut e = editor();
        e.add_page();
        let at = kurbo::Point::new(200.0, 150.0);
        let c = chrome_with_context_menu(&e, ContextTarget::Page(0), at);
        let labels: Vec<&str> = c
            .regions
            .iter()
            .filter_map(|r| matches!(r.kind, RegionKind::MenuEntry { .. }).then_some(r.label.as_str()))
            .collect();
        assert!(labels.contains(&"Delete Page"));
        assert!(labels.contains(&"Rename Page…"));
    }

    #[test]
    fn an_open_menu_is_marked_open_in_the_bar() {
        let c = chrome_with_menu(&editor(), 1);
        let titles: Vec<bool> = c
            .regions
            .iter()
            .filter_map(|r| match r.kind {
                RegionKind::MenuTitle { open } => Some(open),
                _ => None,
            })
            .collect();
        assert!(titles[1], "the open menu's title should be marked open");
        assert!(!titles[0], "and no other title should be");
    }

    #[test]
    fn every_menu_opens_without_escaping_the_window() {
        let e = editor();
        for index in 0..8 {
            let c = chrome_with_menu(&e, index);
            let (_, panel) = c.open_menu.expect("a menu");
            assert!(panel.x0 >= 0.0 && panel.x1 <= window().x + 0.01, "menu {index}: {panel:?}");
            assert!(panel.y1 <= window().y + 0.01, "menu {index} ran off the bottom");
        }
    }

    #[test]
    fn menu_separators_cannot_be_clicked() {
        let c = chrome_with_menu(&editor(), 0);
        let separator = c
            .regions
            .iter()
            .find(|r| matches!(&r.kind, RegionKind::MenuEntry { kind, .. } if *kind == ItemKind::Separator))
            .expect("the File menu has rules");
        assert_eq!(c.hit(separator.rect.center()), None);
    }

    #[test]
    fn disabled_menu_entries_cannot_be_clicked() {
        // Undo is unavailable in a fresh document.
        let c = chrome_with_menu(&editor(), 1);
        let undo = c
            .regions
            .iter()
            .find(|r| r.label == "Undo")
            .expect("an Undo entry");
        assert_eq!(c.hit(undo.rect.center()), None, "a dimmed command must not fire");
    }

    #[test]
    fn an_open_menu_wins_hit_testing_over_what_it_covers() {
        let mut e = editor();
        e.add_page();
        let c = chrome_with_menu(&e, 3);
        let (_, panel) = c.open_menu.unwrap();

        // A point inside the panel is not canvas, whatever is behind it.
        let inside = kurbo::Point::new(panel.x0 + 10.0, panel.y0 + 10.0);
        assert!(c.covered_by_open_menu(inside));
        assert!(!c.is_over_canvas(inside));
    }

    #[test]
    fn menu_entries_carry_their_shortcut_text() {
        let c = chrome_with_menu(&editor(), 0);
        let save = c.regions.iter().find(|r| r.label == "Save").unwrap();
        match &save.kind {
            RegionKind::MenuEntry { shortcut, .. } => assert_eq!(shortcut, "⌘S"),
            other => panic!("Save is {other:?}"),
        }
    }

    // ------------------------------------------------------------- palette

    #[test]
    fn the_palette_offers_every_shape_in_the_selected_library() {
        let e = editor();
        for library in ShapeLibrary::ALL {
            let c = chrome_with_library(&e, *library);
            let offered: Vec<String> = c
                .regions
                .iter()
                .filter_map(|r| match &r.kind {
                    RegionKind::ShapeTile(t) => Some(t.as_str().to_string()),
                    _ => None,
                })
                .collect();

            let expected = library.types();
            assert_eq!(
                offered.len(),
                expected.len(),
                "{} offered {} of {}",
                library.display_name(),
                offered.len(),
                expected.len()
            );
            for shape_type in expected {
                assert!(
                    offered.contains(&shape_type.as_str().to_string()),
                    "{} is missing from {}",
                    shape_type.as_str(),
                    library.display_name()
                );
            }
        }
    }

    #[test]
    fn every_published_shape_type_is_reachable_from_some_library() {
        let e = editor();
        let mut reachable: Vec<String> = Vec::new();
        for library in ShapeLibrary::ALL {
            for region in chrome_with_library(&e, *library).regions {
                if let RegionKind::ShapeTile(t) = region.kind {
                    reachable.push(t.as_str().to_string());
                }
            }
        }
        assert_eq!(reachable.len(), ShapeType::KNOWN.len());
        for name in ShapeType::KNOWN {
            assert!(reachable.contains(&name.to_string()), "{name} is unreachable");
        }
    }

    #[test]
    fn the_library_selector_marks_the_current_library() {
        let c = chrome_with_library(&editor(), ShapeLibrary::Bpmn);
        let active: Vec<&str> = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::LibraryTab { active: true }))
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(active, vec![ShapeLibrary::Bpmn.short_name()]);
    }

    #[test]
    fn every_library_has_a_selector() {
        let c = chrome(&editor());
        let offered: Vec<&str> = c
            .regions
            .iter()
            .filter(|r| matches!(r.kind, RegionKind::LibraryTab { .. }))
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(offered.len(), ShapeLibrary::ALL.len());
    }

    #[test]
    fn the_tool_strip_marks_the_active_tool() {
        let mut e = editor();
        e.tool = Tool::Connector;
        let c = chrome(&e);
        let connect = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::SelectTool(Tool::Connector))
            .unwrap();
        assert!(connect.selected);

        let select = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::SelectTool(Tool::Select))
            .unwrap();
        assert!(!select.selected);
    }

    #[test]
    fn the_pen_tool_is_in_the_tool_strip() {
        let mut e = editor();
        e.tool = Tool::Freehand;
        let c = chrome(&e);
        let pen = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::SelectTool(Tool::Freehand))
            .expect("a pen tool button");
        assert!(pen.selected);
    }

    // ----------------------------------------------------------------- link

    #[test]
    fn the_link_field_is_offered_for_a_single_shape_only() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        let c = chrome(&e);
        assert!(c.regions.iter().any(|r| r.action == UiAction::FocusLinkField));
    }

    #[test]
    fn the_link_field_is_absent_with_nothing_selected() {
        let c = chrome(&editor());
        assert!(!c.regions.iter().any(|r| r.action == UiAction::FocusLinkField));
    }

    #[test]
    fn the_link_field_shows_the_shapes_current_link() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        e.set_link_url("https://example.com");
        let c = chrome(&e);
        let field = c.regions.iter().find(|r| r.action == UiAction::FocusLinkField).unwrap();
        assert_eq!(field.label, "https://example.com");

        let remove = c.regions.iter().find(|r| r.action == UiAction::ClearLink);
        assert!(remove.is_some(), "a link that exists should offer a way to remove it");
    }

    #[test]
    fn the_link_field_shows_the_live_edit_while_focused() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        e.set_link_url("https://example.com");

        let view = ViewState { link_edit_focused: true, link_edit: "https://typing".into(), ..Default::default() };
        let c = Chrome::build_for(window(), &e, &Metrics::default(), &view, None, None, None);
        let field = c.regions.iter().find(|r| r.action == UiAction::FocusLinkField).unwrap();
        assert_eq!(field.label, "https://typing", "the field shows what is being typed, not the saved value");
        assert_eq!(field.kind, RegionKind::TextField { focused: true, placeholder: "https://…" });
    }

    // --------------------------------------------------------- custom data

    #[test]
    fn custom_data_rows_are_offered_for_a_single_shape() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        e.add_custom_data();
        e.set_custom_data_key(0, "owner");
        e.set_custom_data_value(0, "ops");

        let c = chrome(&e);
        let key = c.regions.iter().find(|r| r.action == UiAction::FocusCustomDataKey(0)).unwrap();
        assert_eq!(key.label, "owner");
        let value = c.regions.iter().find(|r| r.action == UiAction::FocusCustomDataValue(0)).unwrap();
        assert_eq!(value.label, "ops");
        assert!(c.regions.iter().any(|r| r.action == UiAction::RemoveCustomData(0)));
        assert!(c.regions.iter().any(|r| r.action == UiAction::AddCustomData));
    }

    #[test]
    fn custom_data_is_absent_with_nothing_selected() {
        let c = chrome(&editor());
        assert!(!c.regions.iter().any(|r| r.action == UiAction::AddCustomData));
    }

    #[test]
    fn a_focused_custom_data_field_shows_its_live_edit() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        e.add_custom_data();
        e.set_custom_data_key(0, "owner");

        let view = ViewState {
            custom_data_focus: Some(CustomDataFocus::Key(0)),
            custom_data_edit: "typing".into(),
            ..Default::default()
        };
        let c = Chrome::build_for(window(), &e, &Metrics::default(), &view, None, None, None);
        let key = c.regions.iter().find(|r| r.action == UiAction::FocusCustomDataKey(0)).unwrap();
        assert_eq!(key.label, "typing");
        assert_eq!(key.kind, RegionKind::TextField { focused: true, placeholder: "key" });
    }

    // ------------------------------------------------------------- layers

    fn chrome_with_layers_panel(e: &Editor) -> Chrome {
        let view = ViewState { layers_panel_open: true, ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    #[test]
    fn the_layers_panel_is_absent_until_opened() {
        let c = chrome(&editor());
        assert!(c.layers_panel.is_none());
        assert!(!c.regions.iter().any(|r| r.action == UiAction::AddLayer));
    }

    #[test]
    fn opening_the_layers_panel_shows_its_controls() {
        let mut e = editor();
        let id = e.add_layer();
        let c = chrome_with_layers_panel(&e);

        assert!(c.layers_panel.is_some());
        assert!(c.regions.iter().any(|r| r.action == UiAction::AddLayer));
        assert!(c.regions.iter().any(|r| r.action == UiAction::ToggleLayerVisibility(id.clone())));
        assert!(c.regions.iter().any(|r| r.action == UiAction::ToggleLayerLock(id.clone())));
        assert!(c.regions.iter().any(|r| r.action == UiAction::DeleteLayer(id.clone())));
    }

    #[test]
    fn the_move_selection_action_is_offered_only_with_a_selection() {
        let mut e = editor();
        let id = e.add_layer();
        let c = chrome_with_layers_panel(&e);
        assert!(!c.regions.iter().any(|r| r.action == UiAction::MoveSelectionToLayer(id.clone())));

        let shape = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.selection = vec![shape];
        let c = chrome_with_layers_panel(&e);
        assert!(c.regions.iter().any(|r| r.action == UiAction::MoveSelectionToLayer(id.clone())));
    }

    #[test]
    fn the_layers_panel_is_opaque_so_it_does_not_leak_clicks_to_the_canvas() {
        let e = editor();
        let c = chrome_with_layers_panel(&e);
        let panel = c.layers_panel.unwrap();
        assert!(!c.is_over_canvas(panel.center()));
    }

    #[test]
    fn visibility_and_lock_buttons_reflect_the_layers_state() {
        let mut e = editor();
        let id = e.add_layer();
        e.toggle_layer_lock(&id);
        let c = chrome_with_layers_panel(&e);

        let vis = c.regions.iter().find(|r| r.action == UiAction::ToggleLayerVisibility(id.clone())).unwrap();
        assert_eq!(vis.kind, RegionKind::Choice { active: true }, "visible by default");

        let lock = c.regions.iter().find(|r| r.action == UiAction::ToggleLayerLock(id.clone())).unwrap();
        assert_eq!(lock.kind, RegionKind::Choice { active: true }, "locked just now");
    }

    // --------------------------------------------------------- find and replace

    fn chrome_with_find(e: &Editor, query: &str) -> Chrome {
        let view = ViewState { find_open: true, find_query: query.to_string(), ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    #[test]
    fn the_find_bar_is_absent_until_opened() {
        let c = chrome(&editor());
        assert!(c.find_bar.is_none());
        assert!(!c.regions.iter().any(|r| r.action == UiAction::FocusFindQuery));
    }

    #[test]
    fn opening_find_shows_its_controls() {
        let c = chrome_with_find(&editor(), "");
        assert!(c.find_bar.is_some());
        assert!(c.regions.iter().any(|r| r.action == UiAction::FocusFindQuery));
        assert!(c.regions.iter().any(|r| r.action == UiAction::FocusFindReplacement));
        assert!(c.regions.iter().any(|r| r.action == UiAction::FindNext));
        assert!(c.regions.iter().any(|r| r.action == UiAction::FindPrevious));
        assert!(c.regions.iter().any(|r| r.action == UiAction::ReplaceCurrentMatch));
    }

    #[test]
    fn the_match_count_reflects_the_query() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("Approval"));
        }

        let empty = chrome_with_find(&e, "");
        let heading = |c: &Chrome| {
            c.regions
                .iter()
                .find(|r| {
                    matches!(r.kind, RegionKind::Heading)
                        && (r.label.starts_with("Match") || r.label == "No matches")
                })
                .map(|r| r.label.clone())
        };
        assert_eq!(heading(&empty), None, "an empty query shows no count");

        let no_match = chrome_with_find(&e, "zzz");
        assert_eq!(heading(&no_match), Some("No matches".to_string()));

        let one_match = chrome_with_find(&e, "approval");
        assert_eq!(heading(&one_match), Some("Match 1 of 1".to_string()));
    }

    #[test]
    fn the_find_bar_is_opaque_so_it_does_not_leak_clicks_to_the_canvas() {
        let c = chrome_with_find(&editor(), "");
        let bar = c.find_bar.unwrap();
        assert!(!c.is_over_canvas(bar.center()));
    }

    // ---------------------------------------------------------- hit testing

    #[test]
    fn hit_testing_finds_a_control_at_its_own_rect() {
        let c = chrome(&editor());
        for (i, region) in c.regions.iter().enumerate() {
            if !region.is_clickable() {
                continue;
            }
            let center = region.rect.center();
            let hit = c.hit(center);
            assert!(hit.is_some(), "nothing hit at {center:?} for {:?}", region.action);
            let found = &c.regions[hit.unwrap()];
            assert!(found.rect.contains(center), "region {i} resolved badly");
        }
    }

    #[test]
    fn headings_are_not_clickable() {
        let c = chrome(&editor());
        let heading = c
            .regions
            .iter()
            .find(|r| r.kind == RegionKind::Heading)
            .expect("the panels have section labels");
        assert_eq!(
            c.hit(heading.rect.center()),
            None,
            "a section label should not swallow clicks"
        );
    }

    #[test]
    fn controls_do_not_overlap_the_canvas() {
        let c = chrome(&editor());
        for region in &c.regions {
            let overlaps = region.rect.x0 < c.layout.canvas.x1
                && c.layout.canvas.x0 < region.rect.x1
                && region.rect.y0 < c.layout.canvas.y1
                && c.layout.canvas.y0 < region.rect.y1;
            assert!(
                !overlaps,
                "{:?} at {:?} intrudes on the canvas {:?}",
                region.action, region.rect, c.layout.canvas
            );
        }
    }

    #[test]
    fn a_canvas_point_is_reported_as_canvas() {
        let c = chrome(&editor());
        let center = c.layout.canvas.center();
        assert!(c.is_over_canvas(center));
        assert!(!c.is_over_canvas(kurbo::Point::new(4.0, 4.0)), "the menu bar is not canvas");
    }

    // ------------------------------------------------------------ page tabs

    #[test]
    fn page_tabs_reflect_the_active_page() {
        let mut e = editor();
        e.add_page();
        let c = chrome(&e);

        let tabs: Vec<&Region> = c
            .regions
            .iter()
            .filter(|r| matches!(r.action, UiAction::SelectPage(_)))
            .collect();
        assert_eq!(tabs.len(), 2);

        let active: Vec<bool> = tabs
            .iter()
            .map(|r| matches!(r.kind, RegionKind::Tab { active: true }))
            .collect();
        assert_eq!(active, vec![false, true], "the second page is current");
    }

    #[test]
    fn a_pointer_over_a_tab_resolves_to_its_index() {
        let mut e = editor();
        e.add_page();
        e.add_page();
        let c = chrome(&e);
        assert_eq!(c.tab_rects.len(), 3);

        for (i, rect) in c.tab_rects.iter().enumerate() {
            assert_eq!(c.tab_index_at(rect.center().x), Some(i), "tab {i}");
        }
    }

    #[test]
    fn a_pointer_past_the_last_tab_resolves_to_the_end() {
        let mut e = editor();
        e.add_page();
        let c = chrome(&e);
        let far_right = c.tab_rects.last().unwrap().x1 + 200.0;
        assert_eq!(c.tab_index_at(far_right), Some(1));
    }

    #[test]
    fn a_tab_being_dragged_shows_where_it_would_land() {
        let mut e = editor();
        e.add_page();
        e.add_page();
        let base = chrome(&e);
        // Dragging the first tab out past the last one.
        let pointer_x = base.tab_rects[2].center().x + 10.0;
        let c = Chrome::build_for(
            window(),
            &e,
            &Metrics::default(),
            &ViewState::default(),
            None,
            Some(TabDrag { from: 0, pointer_x }),
            None,
        );

        assert!(
            c.regions.iter().any(|r| r.kind == RegionKind::DropMarker),
            "a drag should show an insertion point"
        );
    }

    #[test]
    fn dropping_a_tab_where_it_already_is_shows_no_marker() {
        let mut e = editor();
        e.add_page();
        let base = chrome(&e);
        let pointer_x = base.tab_rects[0].center().x;
        let c = Chrome::build_for(
            window(),
            &e,
            &Metrics::default(),
            &ViewState::default(),
            None,
            Some(TabDrag { from: 0, pointer_x }),
            None,
        );
        assert!(!c.regions.iter().any(|r| r.kind == RegionKind::DropMarker));
    }

    #[test]
    fn a_single_page_cannot_be_reordered() {
        let e = editor();
        let c = Chrome::build_for(
            window(),
            &e,
            &Metrics::default(),
            &ViewState::default(),
            None,
            Some(TabDrag { from: 0, pointer_x: 500.0 }),
            None,
        );
        assert!(!c.regions.iter().any(|r| r.kind == RegionKind::DropMarker));
    }

    #[test]
    fn the_drop_marker_shows_where_the_tab_will_actually_land() {
        let mut e = editor();
        e.add_page();
        e.add_page();
        e.add_page();
        let base = chrome(&e);
        let rects = base.tab_rects.clone();

        // For every drag of every tab to every position, the marker must sit
        // where the reordered tab would be, or be absent when nothing moves.
        for from in 0..rects.len() {
            for over in 0..rects.len() {
                let pointer_x = rects[over].center().x;
                let drag = TabDrag { from, pointer_x };
                let c = Chrome::build_for(
                    window(),
                    &e,
                    &Metrics::default(),
                    &ViewState::default(),
                    None,
                    Some(drag),
                    None,
                );
                let marker = c
                    .regions
                    .iter()
                    .find(|r| r.kind == RegionKind::DropMarker)
                    .map(|r| r.rect.center().x);
                let to = c.tab_index_at(pointer_x).unwrap();

                if to == from {
                    assert!(marker.is_none(), "from {from} over {over} moves nothing");
                    continue;
                }

                // Where the tab ends up, read off the reordered title list.
                let mut order: Vec<usize> = (0..rects.len()).collect();
                let moved = order.remove(from);
                order.insert(to, moved);
                let landed = order.iter().position(|i| *i == moved).unwrap();
                assert_eq!(landed, to, "move_page semantics changed");

                let marker = marker.unwrap_or_else(|| {
                    panic!("from {from} to {to} should show a marker")
                });
                // The marker sits just before the tab that will follow it, or
                // just after the last tab.
                let expected = if to < from {
                    rects[to].x0
                } else if to + 1 < rects.len() {
                    rects[to + 1].x0
                } else {
                    rects[rects.len() - 1].x1
                };
                assert!(
                    (marker - expected).abs() <= 3.0,
                    "from {from} to {to}: marker at {marker}, expected near {expected}"
                );
            }
        }
    }

    #[test]
    fn a_drop_marker_is_not_clickable() {
        let mut e = editor();
        e.add_page();
        e.add_page();
        let base = chrome(&e);
        let pointer_x = base.tab_rects[2].center().x + 10.0;
        let c = Chrome::build_for(
            window(),
            &e,
            &Metrics::default(),
            &ViewState::default(),
            None,
            Some(TabDrag { from: 0, pointer_x }),
            None,
        );
        let marker = c
            .regions
            .iter()
            .find(|r| r.kind == RegionKind::DropMarker)
            .unwrap();
        let hit = c.hit(marker.rect.center());
        assert!(
            hit.map(|i| c.regions[i].kind != RegionKind::DropMarker).unwrap_or(true),
            "the marker swallowed a click"
        );
    }

    // ----------------------------------------------------------- properties

    #[test]
    fn properties_panel_summarizes_the_selection() {
        let mut e = editor();
        let labels = |e: &Editor| {
            chrome(e)
                .regions
                .iter()
                .map(|r| r.label.clone())
                .collect::<Vec<_>>()
        };
        assert!(labels(&e).contains(&"Nothing selected".to_string()));

        e.create_shape(ShapeType::Decision, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        assert!(labels(&e).contains(&"Decision".to_string()));

        e.create_shape(ShapeType::Process, BoundingBox::new(20.0, 0.0, 10.0, 10.0));
        e.select_all();
        assert!(labels(&e).contains(&"2 items selected".to_string()));
    }

    #[test]
    fn swatch_actions_carry_their_color() {
        let c = chrome(&editor());
        let fill_actions: Vec<Color> = c
            .regions
            .iter()
            .filter_map(|r| match r.action {
                UiAction::SetFill(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(fill_actions.len(), FILL_SWATCHES.len());
        assert!(fill_actions.contains(&Color::WHITE));
    }

    #[test]
    fn the_text_buttons_show_the_state_of_the_selection() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("<b>bold</b>"));
        }
        e.selection = vec![id];

        let c = chrome(&e);
        let bold = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::ToggleText(TextToggle::Bold))
            .unwrap();
        assert_eq!(bold.kind, RegionKind::Choice { active: true });

        let italic = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::ToggleText(TextToggle::Italic))
            .unwrap();
        assert_eq!(italic.kind, RegionKind::Choice { active: false });
    }

    #[test]
    fn the_font_family_buttons_mark_the_selections_shared_family() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("label"));
        }
        e.selection = vec![id.clone()];
        e.set_font_family("Georgia");

        let c = chrome(&e);
        let serif = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::SetFontFamily("Georgia"))
            .unwrap();
        assert_eq!(serif.kind, RegionKind::Choice { active: true });

        let sans = c
            .regions
            .iter()
            .find(|r| r.action == UiAction::SetFontFamily("Liberation Sans"))
            .unwrap();
        assert_eq!(sans.kind, RegionKind::Choice { active: false });
    }

    /// The emphasis on the control carrying an action.
    fn emphasis_of(c: &Chrome, action: &UiAction) -> Emphasis {
        c.regions
            .iter()
            .find(|r| r.action == *action)
            .unwrap_or_else(|| panic!("no control for {action:?}"))
            .emphasis
    }

    #[test]
    fn each_text_style_button_is_set_in_the_style_it_applies() {
        // The button says what it does by being what it does: the bold one is
        // bold, the italic one italic, and so on.
        let c = chrome(&editor());
        assert_eq!(
            emphasis_of(&c, &UiAction::ToggleText(TextToggle::Bold)),
            Emphasis { bold: true, ..Default::default() }
        );
        assert_eq!(
            emphasis_of(&c, &UiAction::ToggleText(TextToggle::Italic)),
            Emphasis { italic: true, ..Default::default() }
        );
        assert_eq!(
            emphasis_of(&c, &UiAction::ToggleText(TextToggle::Underline)),
            Emphasis { underline: true, ..Default::default() }
        );
        assert_eq!(
            emphasis_of(&c, &UiAction::ToggleText(TextToggle::Strikethrough)),
            Emphasis { strikethrough: true, ..Default::default() }
        );
    }

    #[test]
    fn the_style_buttons_are_the_single_letters_they_should_be() {
        let c = chrome(&editor());
        for (toggle, letter) in [
            (TextToggle::Bold, "B"),
            (TextToggle::Italic, "I"),
            (TextToggle::Underline, "U"),
            (TextToggle::Strikethrough, "S"),
        ] {
            let region = c
                .regions
                .iter()
                .find(|r| r.action == UiAction::ToggleText(toggle))
                .unwrap();
            assert_eq!(region.label, letter);
        }
    }

    #[test]
    fn the_type_size_buttons_differ_in_size() {
        let c = chrome(&editor());
        let size = |action: &UiAction| {
            c.regions
                .iter()
                .find(|r| r.action == *action)
                .unwrap()
                .label_size
                .expect("the size buttons set their own size")
        };
        let smaller = size(&UiAction::ScaleTextSize(1.0 / 1.15));
        let bigger = size(&UiAction::ScaleTextSize(1.15));
        assert!(bigger > smaller, "{bigger} should be larger than {smaller}");
    }

    /// Every preview the panel draws, with whether it is marked current.
    fn previews(c: &Chrome) -> Vec<(Preview, bool)> {
        c.regions
            .iter()
            .filter_map(|r| match &r.kind {
                RegionKind::PreviewChoice { active, preview } => {
                    Some((preview.clone(), *active))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_rows_that_can_be_drawn_are_drawn_rather_than_named() {
        let c = chrome(&editor());
        let kinds = previews(&c);

        assert!(kinds.iter().any(|(p, _)| matches!(p, Preview::Weight(_))), "weight");
        assert!(kinds.iter().any(|(p, _)| matches!(p, Preview::Dash(_))), "line style");
        assert!(kinds.iter().any(|(p, _)| matches!(p, Preview::Route(_))), "connector");
        assert!(kinds.iter().any(|(p, _)| matches!(p, Preview::Align(_))), "alignment");
        assert!(
            kinds.iter().any(|(p, _)| matches!(p, Preview::VerticalAlign(_))),
            "vertical alignment"
        );

        // A drawn choice carries no text, so nothing is labelled twice.
        for region in &c.regions {
            if matches!(region.kind, RegionKind::PreviewChoice { .. }) {
                assert!(region.label.is_empty(), "{:?} is both drawn and named", region.action);
            }
        }
    }

    #[test]
    fn every_line_style_the_model_has_is_offered() {
        use dc_model::StrokeStyle;
        let c = chrome(&editor());
        for style in [StrokeStyle::Solid, StrokeStyle::Dashed, StrokeStyle::Dotted] {
            assert!(
                c.regions.iter().any(|r| r.action == UiAction::SetStrokeStyle(style)),
                "{style:?} cannot be chosen"
            );
        }
    }

    #[test]
    fn the_panel_marks_what_the_selection_already_is() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        e.selection = vec![id];

        let blue = Color::rgb(0x25, 0x63, 0xeb);
        e.set_selection_fill(Color::rgb(0xdb, 0xea, 0xfe));
        e.set_selection_stroke(blue, Some(3.0));
        e.set_selection_stroke_style(dc_model::StrokeStyle::Dashed);

        let c = chrome(&e);

        // The fill and line chips the selection uses are ringed.
        let ringed = |action: UiAction| {
            c.regions
                .iter()
                .find(|r| r.action == action)
                .map(|r| r.selected)
                .unwrap_or(false)
        };
        assert!(ringed(UiAction::SetFill(Color::rgb(0xdb, 0xea, 0xfe))), "fill");
        assert!(ringed(UiAction::SetStroke(blue)), "line colour");
        assert!(!ringed(UiAction::SetFill(Color::WHITE)), "only one fill is current");

        // And the weight and dash rows mark theirs.
        let active = |action: UiAction| {
            c.regions
                .iter()
                .find(|r| r.action == action)
                .map(|r| {
                    matches!(
                        r.kind,
                        RegionKind::Choice { active: true }
                            | RegionKind::PreviewChoice { active: true, .. }
                    )
                })
                .unwrap_or(false)
        };
        assert!(active(UiAction::SetStrokeWidth(3.0)), "weight");
        assert!(!active(UiAction::SetStrokeWidth(1.0)));
        assert!(active(UiAction::SetStrokeStyle(dc_model::StrokeStyle::Dashed)), "dash");
        assert!(!active(UiAction::SetStrokeStyle(dc_model::StrokeStyle::Solid)));
    }

    #[test]
    fn a_mixed_selection_marks_nothing_as_current() {
        let mut e = editor();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(40.0, 0.0, 10.0, 10.0));

        e.selection = vec![a.clone()];
        e.set_selection_fill(Color::rgb(0xfe, 0xe2, 0xe2));
        e.selection = vec![b.clone()];
        e.set_selection_fill(Color::rgb(0xd1, 0xfa, 0xe5));
        e.selection = vec![a, b];

        let c = chrome(&e);
        let ringed: Vec<&Region> = c
            .regions
            .iter()
            .filter(|r| matches!(r.action, UiAction::SetFill(_)) && r.selected)
            .collect();
        assert!(
            ringed.is_empty(),
            "two different fills should leave the row showing none"
        );
    }

    #[test]
    fn nothing_is_marked_with_nothing_selected() {
        let c = chrome(&editor());
        for region in &c.regions {
            if matches!(region.action, UiAction::SetFill(_) | UiAction::SetStroke(_)) {
                assert!(!region.selected, "{:?} claims to be current", region.action);
            }
        }
    }

    // ------------------------------------------------ a dynamic properties panel

    /// An editor with a connector between two shapes.
    fn editor_with_connector() -> Editor {
        let mut e = editor();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 40.0, 20.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(200.0, 0.0, 40.0, 20.0));
        let id = e.package.document.fresh_id("line");
        e.package.document.pages[0].add_line(dc_model::Line::new(
            &id,
            dc_model::Endpoint::at_shape(a, dc_model::EndpointStyle::None),
            dc_model::Endpoint::at_shape(b, dc_model::EndpointStyle::Arrow),
        ));
        e.selection = vec![id];
        e
    }

    fn has_action(c: &Chrome, action: &UiAction) -> bool {
        c.regions.iter().any(|r| r.action == *action)
    }

    #[test]
    fn connector_controls_appear_only_when_a_connector_is_selected() {
        use crate::editor::LineEnd;
        let arrow = UiAction::SetEndpointStyle(LineEnd::End, dc_model::EndpointStyle::Arrow);

        let with_line = chrome(&editor_with_connector());
        assert!(has_action(&with_line, &arrow), "a connector needs its endpoints");
        assert!(has_action(&with_line, &UiAction::SwapEndpoints));
        assert!(has_action(&with_line, &UiAction::SetLineType(LineType::Elbow)));

        let mut shapes_only = editor_with_connector();
        shapes_only.select_all_shapes();
        let with_shape = chrome(&shapes_only);
        assert!(
            !has_action(&with_shape, &arrow),
            "a shape has no endpoints, so the row should be absent"
        );
    }

    #[test]
    fn shape_controls_appear_only_when_a_shape_is_selected() {
        let mut shapes_only = editor_with_connector();
        shapes_only.select_all_shapes();
        let with_shape = chrome(&shapes_only);
        assert!(has_action(&with_shape, &UiAction::SetOpacity(50.0)));
        assert!(has_action(&with_shape, &UiAction::Flip(true)));

        let with_line = chrome(&editor_with_connector());
        assert!(
            !has_action(&with_line, &UiAction::SetOpacity(50.0)),
            "a connector has no opacity, so the row should be absent"
        );
    }

    #[test]
    fn with_nothing_selected_every_control_is_shown() {
        // An empty panel would be worse than one whose controls are waiting:
        // this is where the user goes to find out what is available.
        use crate::editor::LineEnd;
        let c = chrome(&editor());
        assert!(has_action(&c, &UiAction::SetOpacity(50.0)));
        assert!(has_action(
            &c,
            &UiAction::SetEndpointStyle(LineEnd::End, dc_model::EndpointStyle::Arrow)
        ));
        assert!(has_action(&c, &UiAction::SetLineType(LineType::Elbow)));
    }

    #[test]
    fn both_ends_get_their_own_picker() {
        use crate::editor::LineEnd;
        let c = chrome(&editor_with_connector());
        for end in [LineEnd::Start, LineEnd::End] {
            for style in &ENDPOINT_CHOICES {
                assert!(
                    has_action(&c, &UiAction::SetEndpointStyle(end, style.clone())),
                    "{end:?} cannot be set to {style:?}"
                );
            }
        }
    }

    #[test]
    fn the_endpoint_pickers_mark_what_the_connector_has() {
        use crate::editor::LineEnd;
        let c = chrome(&editor_with_connector());
        let active = |end: LineEnd, style: dc_model::EndpointStyle| {
            c.regions
                .iter()
                .find(|r| r.action == UiAction::SetEndpointStyle(end, style.clone()))
                .map(|r| matches!(r.kind, RegionKind::PreviewChoice { active: true, .. }))
                .unwrap_or(false)
        };
        assert!(active(LineEnd::Start, dc_model::EndpointStyle::None));
        assert!(active(LineEnd::End, dc_model::EndpointStyle::Arrow));
        assert!(!active(LineEnd::End, dc_model::EndpointStyle::None));
    }

    #[test]
    fn the_endpoint_previews_point_away_from_the_line() {
        // A terminator on the end of a line points forward, away from the
        // line; one on the start points backward. The vector these are built
        // from means the opposite of the way the glyph points, so getting its
        // sign wrong turns every terminator inward — which is what this pins.
        use kurbo::Shape as _;

        for pointing_right in [true, false] {
            let tip = dc_model::Point::new(if pointing_right { 100.0 } else { 0.0 }, 50.0);
            let marker = dc_geom::endpoint_marker(
                &dc_model::EndpointStyle::Arrow,
                tip,
                endpoint_outward(pointing_right),
                1.4,
            )
            .expect("an arrow has a glyph");
            let body = marker.path.bounding_box();

            if pointing_right {
                assert!(
                    body.x1 <= tip.x + 0.5 && body.x0 < tip.x - 1.0,
                    "a forward arrow's body must sit behind its tip, got {body:?}"
                );
            } else {
                assert!(
                    body.x0 >= tip.x - 0.5 && body.x1 > tip.x + 1.0,
                    "a backward arrow's body must sit behind its tip, got {body:?}"
                );
            }
        }
    }

    #[test]
    fn the_two_pickers_point_opposite_ways() {
        use crate::editor::LineEnd;
        let c = chrome(&editor_with_connector());
        let pointing = |end: LineEnd| {
            c.regions
                .iter()
                .find_map(|r| match (&r.action, &r.kind) {
                    (
                        UiAction::SetEndpointStyle(e, _),
                        RegionKind::PreviewChoice {
                            preview: Preview::Endpoint { pointing_right, .. },
                            ..
                        },
                    ) if *e == end => Some(*pointing_right),
                    _ => None,
                })
                .expect("a picker")
        };
        assert!(!pointing(LineEnd::Start), "the start terminator points back");
        assert!(pointing(LineEnd::End), "the end one points forward");
    }

    #[test]
    fn change_shape_offers_alternatives_from_the_same_library() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Decision, BoundingBox::new(0.0, 0.0, 40.0, 20.0));
        e.selection = vec![id];

        let offered: Vec<ShapeType> = chrome(&e)
            .regions
            .iter()
            .filter_map(|r| match &r.action {
                UiAction::ChangeShapeType(t) => Some(t.clone()),
                _ => None,
            })
            .collect();

        assert!(!offered.is_empty(), "a shape should offer substitutes");
        for shape_type in &offered {
            assert_eq!(
                shape_type.library(),
                dc_model::ShapeLibrary::Flowchart,
                "{shape_type:?} is not from the selection's own library"
            );
            assert_ne!(*shape_type, ShapeType::Decision, "its own type is not a change");
        }
    }

    #[test]
    fn change_shape_is_offered_for_one_shape_only() {
        let mut e = editor();
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(40.0, 0.0, 10.0, 10.0));
        e.select_all_shapes();

        let offered = chrome(&e)
            .regions
            .iter()
            .filter(|r| matches!(r.action, UiAction::ChangeShapeType(_)))
            .count();
        assert_eq!(offered, 0, "two shapes have no single starting point");
    }

    #[test]
    fn all_four_ordering_commands_are_present() {
        let c = chrome(&editor());
        for action in [
            UiAction::BringToFront,
            UiAction::BringForward,
            UiAction::SendBackward,
            UiAction::SendToBack,
        ] {
            assert!(has_action(&c, &action), "{action:?} is missing");
        }
    }

    #[test]
    fn the_panel_offers_paint_format() {
        let c = chrome(&editor());
        assert!(has_action(&c, &UiAction::CopyStyle));
        assert!(has_action(&c, &UiAction::PasteStyle));
    }

    #[test]
    fn the_object_rows_mark_what_the_shape_has() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        e.selection = vec![id];
        e.set_opacity(50.0);
        e.set_corner_rounding(10.0);

        let c = chrome(&e);
        let active = |action: UiAction| {
            c.regions
                .iter()
                .find(|r| r.action == action)
                .map(|r| matches!(r.kind, RegionKind::PreviewChoice { active: true, .. }))
                .unwrap_or(false)
        };
        assert!(active(UiAction::SetOpacity(50.0)));
        assert!(!active(UiAction::SetOpacity(100.0)));
        assert!(active(UiAction::SetCornerRounding(10.0)));
        assert!(!active(UiAction::SetCornerRounding(0.0)));
    }

    #[test]
    fn every_panel_control_stays_inside_its_panel() {
        // The panel grew a lot; nothing may spill into the canvas or past the
        // bottom of the window.
        let e = editor_with_connector();
        let c = chrome(&e);
        for region in &c.regions {
            if region.rect.x0 < c.layout.right_panel.x0 {
                continue;
            }
            assert!(
                region.rect.x1 <= c.layout.right_panel.x1 + 0.01,
                "{:?} runs past the panel edge",
                region.action
            );
        }
    }

    // -------------------------------------------------------------- drawing

    #[test]
    fn drawing_the_chrome_encodes_geometry() {
        let e = editor();
        let c = chrome(&e);
        let mut scene = Scene::new();
        let mut text = TextEngine::new();

        c.draw(
            &mut scene,
            &Theme::light(),
            &Metrics::default(),
            &mut text,
            None,
            "Ready",
        );
        assert!(scene.encoding().n_paths > 10, "chrome did not encode");
    }

    #[test]
    fn drawing_an_open_menu_adds_geometry() {
        let e = editor();
        let mut text = TextEngine::new();

        let mut closed = Scene::new();
        chrome(&e).draw(&mut closed, &Theme::light(), &Metrics::default(), &mut text, None, "");

        let mut open = Scene::new();
        chrome_with_menu(&e, 0).draw(
            &mut open,
            &Theme::light(),
            &Metrics::default(),
            &mut text,
            None,
            "",
        );

        assert!(
            open.encoding().n_paths > closed.encoding().n_paths,
            "an open menu should paint a panel"
        );
    }

    #[test]
    fn drawing_with_a_hover_differs_from_without() {
        let e = editor();
        let c = chrome(&e);
        let mut text = TextEngine::new();

        let mut plain = Scene::new();
        c.draw(&mut plain, &Theme::light(), &Metrics::default(), &mut text, None, "");

        let mut hovered = Scene::new();
        c.draw(&mut hovered, &Theme::light(), &Metrics::default(), &mut text, Some(0), "");

        assert_ne!(
            hovered.encoding().draw_data,
            plain.encoding().draw_data,
            "hover should change the painted colors"
        );
    }

    #[test]
    fn dark_theme_chrome_draws() {
        let e = editor();
        let c = chrome_with_menu(&e, 4);
        let mut scene = Scene::new();
        let mut text = TextEngine::new();
        c.draw(&mut scene, &Theme::dark(), &Metrics::default(), &mut text, None, "x");
        assert!(scene.encoding().n_paths > 10);
    }

    #[test]
    fn every_library_palette_draws() {
        let e = editor();
        for library in ShapeLibrary::ALL {
            let mut scene = Scene::new();
            let mut text = TextEngine::new();
            chrome_with_library(&e, *library).draw(
                &mut scene,
                &Theme::light(),
                &Metrics::default(),
                &mut text,
                None,
                "",
            );
            assert!(
                scene.encoding().n_paths > 10,
                "{} did not draw",
                library.display_name()
            );
        }
    }

    // ------------------------------------------------------ settings screen

    fn chrome_with_settings(e: &Editor, settings: crate::settings::Settings) -> Chrome {
        let view = ViewState { settings, settings_open: true, ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    fn settings_chrome(e: &Editor) -> Chrome {
        chrome_with_settings(e, crate::settings::Settings::default())
    }

    /// Every setting the screen can change, with the action that changes it.
    fn settings_choices(c: &Chrome) -> Vec<(String, SettingChange, bool)> {
        c.regions
            .iter()
            .filter_map(|r| match (&r.kind, &r.action) {
                (RegionKind::Choice { active }, UiAction::ChangeSetting(change)) => {
                    Some((r.label.clone(), change.clone(), *active))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_settings_screen_is_absent_until_it_is_opened() {
        let c = chrome(&editor());
        assert!(c.settings_sheet.is_none());
        assert!(!c.regions.iter().any(|r| r.kind == RegionKind::Scrim));
    }

    // ------------------------------------------------------- template gallery

    fn gallery_chrome(e: &Editor) -> Chrome {
        let view = ViewState { template_gallery_open: true, ..Default::default() };
        Chrome::build_for(window(), e, &Metrics::default(), &view, None, None, None)
    }

    #[test]
    fn the_template_gallery_is_absent_until_opened() {
        let c = chrome(&editor());
        assert!(c.template_gallery.is_none());
        assert!(!c.regions.iter().any(|r| matches!(r.action, UiAction::NewFromTemplate(_))));
    }

    #[test]
    fn opening_the_gallery_lists_every_template() {
        let c = gallery_chrome(&editor());
        assert!(c.template_gallery.is_some());
        for i in 0..crate::templates::ALL.len() {
            assert!(
                c.regions.iter().any(|r| r.action == UiAction::NewFromTemplate(i)),
                "template {i} is not offered"
            );
        }
    }

    #[test]
    fn every_templates_name_and_description_are_shown() {
        let c = gallery_chrome(&editor());
        for t in crate::templates::ALL {
            assert!(c.regions.iter().any(|r| r.label == t.name), "{} not shown", t.name);
            assert!(
                c.regions.iter().any(|r| r.label == t.description),
                "{}'s description not shown",
                t.name
            );
        }
    }

    #[test]
    fn the_gallery_is_opaque_and_modal() {
        let c = gallery_chrome(&editor());
        let sheet = c.template_gallery.unwrap();
        assert!(!c.is_over_canvas(sheet.center()));
        assert!(c.regions.iter().any(|r| r.kind == RegionKind::Scrim));
    }

    #[test]
    fn opening_it_puts_a_sheet_over_the_window() {
        let c = settings_chrome(&editor());
        let sheet = c.settings_sheet.expect("a sheet");

        assert!(sheet.width() > 200.0 && sheet.height() > 150.0);
        assert!(sheet.x0 >= 0.0 && sheet.x1 <= window().x);
        assert!(sheet.y0 >= 0.0 && sheet.y1 <= window().y);
        assert!(c.regions.iter().any(|r| r.kind == RegionKind::Scrim));
    }

    #[test]
    fn the_sheet_fits_a_window_too_small_for_it() {
        let e = editor();
        let view = ViewState { settings_open: true, ..Default::default() };
        for size in [Vec2::new(300.0, 240.0), Vec2::new(120.0, 120.0)] {
            let c = Chrome::build_for(size, &e, &Metrics::default(), &view, None, None, None);
            let sheet = c.settings_sheet.expect("a sheet");
            assert!(sheet.x0 >= -0.01 && sheet.x1 <= size.x + 0.01, "{size:?}: {sheet:?}");
            assert!(sheet.y0 >= -0.01 && sheet.y1 <= size.y + 0.01, "{size:?}: {sheet:?}");
        }
    }

    #[test]
    fn it_is_modal() {
        let c = settings_chrome(&editor());
        // Nothing behind it is reachable, including the canvas.
        assert!(!c.is_over_canvas(c.layout.canvas.center()));

        // A click that misses the sheet lands on the scrim, not on whatever
        // control happens to be under it.
        let outside = kurbo::Point::new(4.0, c.layout.left_panel.y0 + 20.0);
        let hit = c.hit(outside).expect("the scrim catches it");
        assert_eq!(c.regions[hit].kind, RegionKind::Scrim);
        assert_eq!(c.regions[hit].action, UiAction::CloseSettings);
    }

    #[test]
    fn every_setting_offers_the_value_it_currently_holds() {
        // Whatever a preference is set to, exactly one choice in its row is
        // marked active — otherwise the screen would be lying about state.
        let settings = crate::settings::Settings {
            dark_theme: true,
            grid_style: dc_render::GridStyle::Dots,
            grid_spacing: 18.0,
            snap_to_objects: false,
            default_line_type: LineType::Curved,
            default_units: dc_model::Units::Cm,
            default_page_size: "a4".into(),
            zoom_step: 1.5,
            snap_distance: 16.0,
            ..Default::default()
        };

        let c = chrome_with_settings(&editor(), settings.clone());
        for (label, change, active) in settings_choices(&c) {
            // A choice is active exactly when applying it would change
            // nothing, which is the definition of it being the current value.
            let mut probe = settings.clone();
            change.apply(&mut probe);
            let would_change_nothing = probe == settings;
            assert_eq!(
                active, would_change_nothing,
                "{label:?} is marked {active} but applying it {} the settings",
                if would_change_nothing { "would not change" } else { "would change" }
            );
        }
    }

    #[test]
    fn every_preference_the_settings_hold_can_be_changed_from_the_screen() {
        // A preference with no control is one the user cannot reach.
        let c = settings_chrome(&editor());
        let changes: Vec<SettingChange> =
            settings_choices(&c).into_iter().map(|(_, c, _)| c).collect();

        let touched = |probe: &dyn Fn(&crate::settings::Settings) -> bool| {
            changes.iter().any(|change| {
                let mut s = crate::settings::Settings::default();
                change.apply(&mut s);
                probe(&s)
            })
        };

        assert!(touched(&|s| s.dark_theme), "theme");
        assert!(touched(&|s| !s.show_grid), "show grid");
        assert!(touched(&|s| s.grid_style != dc_render::GridStyle::Graph), "grid style");
        assert!(touched(&|s| s.grid_spacing != 9.0), "grid size");
        assert!(touched(&|s| !s.snap_to_objects), "snap to objects");
        assert!(touched(&|s| !s.snap_to_grid), "snap to grid");
        assert!(touched(&|s| s.snap_distance != 6.0), "snap distance");
        assert!(touched(&|s| !s.show_page_bounds), "page bounds");
        assert!(touched(&|s| !s.route_around_shapes), "routing");
        assert!(touched(&|s| s.default_line_type != LineType::Elbow), "default routing");
        assert!(touched(&|s| s.zoom_step != 1.25), "zoom step");
        assert!(touched(&|s| s.default_units != dc_model::Units::In), "units");
        assert!(touched(&|s| s.default_page_size != "letter"), "page size");
    }

    #[test]
    fn the_screen_offers_a_way_out_and_a_way_back_to_the_defaults() {
        let c = settings_chrome(&editor());
        let actions: Vec<&UiAction> = c.regions.iter().map(|r| &r.action).collect();
        assert!(actions.contains(&&UiAction::CloseSettings), "no Done button");
        assert!(
            actions.contains(&&UiAction::ChangeSetting(SettingChange::RestoreDefaults)),
            "no way back to the defaults"
        );
    }

    #[test]
    fn restoring_the_defaults_keeps_what_a_newer_build_wrote() {
        let mut settings =
            crate::settings::Settings { dark_theme: true, ..Default::default() };
        settings
            .extra
            .insert("futureThing".into(), serde_json::json!(42));

        SettingChange::RestoreDefaults.apply(&mut settings);
        assert!(!settings.dark_theme, "the known preferences reset");
        assert_eq!(
            settings.extra.get("futureThing"),
            Some(&serde_json::json!(42)),
            "but an unknown one is not thrown away"
        );
    }

    /// The regions belonging to the settings screen, which are everything
    /// appended from the scrim onward.
    fn settings_regions(c: &Chrome) -> &[Region] {
        let scrim = c
            .regions
            .iter()
            .position(|r| r.kind == RegionKind::Scrim)
            .expect("a scrim");
        &c.regions[scrim..]
    }

    #[test]
    fn every_control_on_the_sheet_sits_inside_it() {
        for size in [window(), Vec2::new(900.0, 560.0), Vec2::new(640.0, 480.0)] {
            let e = editor();
            let view = ViewState { settings_open: true, ..Default::default() };
            let c = Chrome::build_for(size, &e, &Metrics::default(), &view, None, None, None);
            let sheet = c.settings_sheet.unwrap();

            for region in settings_regions(&c) {
                if region.kind == RegionKind::Scrim {
                    continue;
                }
                assert!(
                    sheet.union(region.rect) == sheet,
                    "{size:?}: {:?} at {:?} escapes the sheet {sheet:?}",
                    region.label,
                    region.rect
                );
            }
        }
    }

    #[test]
    fn a_short_window_drops_rows_rather_than_drawing_over_the_footer() {
        let e = editor();
        let view = ViewState { settings_open: true, ..Default::default() };

        let full = Chrome::build_for(window(), &e, &Metrics::default(), &view, None, None, None);
        let short = Chrome::build_for(
            Vec2::new(900.0, 420.0),
            &e,
            &Metrics::default(),
            &view,
            None,
            None,
            None,
        );

        assert!(
            settings_choices(&short).len() < settings_choices(&full).len(),
            "a short sheet should show fewer rows, not overlapping ones"
        );
        // And whatever it does show, the way out is still there.
        let actions: Vec<&UiAction> = short.regions.iter().map(|r| &r.action).collect();
        assert!(actions.contains(&&UiAction::CloseSettings));
    }

    #[test]
    fn the_whole_screen_fits_a_window_of_the_size_it_opens_at() {
        // The sheet is sized to its content; at the default window size every
        // preference must be reachable without anything being dropped.
        let c = settings_chrome(&editor());
        let shown = settings_choices(&c).len();
        assert!(shown >= 26, "only {shown} choices fit");
    }

    #[test]
    fn the_settings_screen_draws_in_both_themes() {
        let e = editor();
        for theme in [Theme::light(), Theme::dark()] {
            let mut scene = Scene::new();
            let mut text = TextEngine::new();
            settings_chrome(&e).draw(
                &mut scene,
                &theme,
                &Metrics::default(),
                &mut text,
                None,
                "",
            );
            assert!(scene.encoding().n_paths > 20, "the sheet did not draw");
        }
    }

    #[test]
    fn opening_the_settings_adds_geometry_over_the_editor() {
        let e = editor();
        let mut text = TextEngine::new();

        let mut closed = Scene::new();
        chrome(&e).draw(&mut closed, &Theme::light(), &Metrics::default(), &mut text, None, "");

        let mut open = Scene::new();
        settings_chrome(&e).draw(
            &mut open,
            &Theme::light(),
            &Metrics::default(),
            &mut text,
            None,
            "",
        );

        assert!(open.encoding().n_paths > closed.encoding().n_paths);
    }

    #[test]
    fn canvas_local_coordinates_map_through_the_layout() {
        let c = chrome(&editor());
        let window_point = kurbo::Point::new(c.layout.canvas.x0 + 30.0, c.layout.canvas.y0 + 40.0);
        let local = Point::new(
            window_point.x - c.layout.canvas.x0,
            window_point.y - c.layout.canvas.y0,
        );
        assert_eq!(local, Point::new(30.0, 40.0));
    }
}
