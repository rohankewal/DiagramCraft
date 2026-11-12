//! The menu bar.
//!
//! Commands live here rather than on a row of toolbar buttons, so the whole
//! command surface is discoverable in one place and carries its keyboard
//! shortcut beside it.
//!
//! The menu structure follows Lucidchart's: **File, Edit, Select, Insert,
//! Format, Arrange, View, Help**, with each menu holding the commands that
//! product documents under that heading — page setup and export under File,
//! the "select every shape with the same fill" family under Select, align,
//! distribute, match size and group under Arrange, and zoom, grid, guides and
//! theme under View. Commands that product has and this one does not — version
//! history, publishing, comments — are absent rather than present and dead.
//!
//! A menu is rebuilt from editor state every frame, the same way the rest of
//! the chrome is, so an item's enabled state and check mark can never
//! disagree with what the command would actually do.

use crate::editor::{Align, Editor, SelectBy, TextToggle, Tool};
use crate::settings::Settings;
use crate::ui::{ContextTarget, CustomDataFocus, UiAction};
use dc_model::{ShapeLibrary, ShapeType};

/// The view-level state the menus reflect, which lives in the window layer
/// rather than in the editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    /// The saved preferences. These are the one source of truth for the
    /// things both the View menu and the settings screen can change, so the
    /// two can never disagree about what is on.
    pub settings: Settings,
    /// Which shape library the palette is showing.
    pub library: ShapeLibrary,
    /// Whether the settings screen is up.
    pub settings_open: bool,
    /// The live query over the palette. Empty means "browse the current
    /// library normally"; otherwise every library is searched at once.
    pub palette_search: String,
    /// Whether the search field is currently taking keystrokes.
    pub palette_search_focused: bool,
    /// The URL being typed for the selected shape's link.
    pub link_edit: String,
    /// Whether the link field is currently taking keystrokes.
    pub link_edit_focused: bool,
    /// Which custom-data field is currently taking keystrokes, if any.
    pub custom_data_focus: Option<CustomDataFocus>,
    /// The live text of the focused custom-data field.
    pub custom_data_edit: String,
    /// Whether the layers panel is showing.
    pub layers_panel_open: bool,
    /// Whether the find bar is showing.
    pub find_open: bool,
    pub find_query: String,
    pub find_query_focused: bool,
    pub find_replacement: String,
    pub find_replacement_focused: bool,
    /// Which match Next/Previous is currently on.
    pub find_match_index: usize,
    /// Whether the "New from Template" gallery is showing.
    pub template_gallery_open: bool,
    /// Shape types the person starred, which is what the My Shapes tab
    /// offers. Carried here because the palette reads it every frame.
    pub favorites: Vec<ShapeType>,
    /// Names of the templates they saved, shown under the built-in ones.
    pub saved_templates: Vec<String>,
}

/// The key a menu item's shortcut names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutKey {
    Char(char),
    Delete,
    Enter,
    Escape,
}

/// A keyboard shortcut, parsed from the string a menu item displays.
///
/// The displayed string stays the one definition: a platform that draws its
/// own menus prints it, and one with a real menu bar parses it into whatever
/// that bar wants. Two separate declarations would drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: ShortcutKey,
}

impl Shortcut {
    /// Reads a shortcut as the menus write them: any of `Shift` and `Alt`,
    /// an optional `⌘`, and a key.
    pub fn parse(text: &str) -> Option<Shortcut> {
        let mut command = false;
        let mut shift = false;
        let mut alt = false;
        let mut key = None;

        for token in text.split_whitespace() {
            if token.eq_ignore_ascii_case("shift") {
                shift = true;
                continue;
            }
            if token.eq_ignore_ascii_case("alt") {
                alt = true;
                continue;
            }
            let rest = match token.strip_prefix('⌘') {
                Some(rest) => {
                    command = true;
                    rest
                }
                None => token,
            };
            if rest.is_empty() {
                continue;
            }
            key = Some(match rest {
                "Del" => ShortcutKey::Delete,
                "Enter" => ShortcutKey::Enter,
                "Esc" => ShortcutKey::Escape,
                other => {
                    let mut chars = other.chars();
                    let c = chars.next()?;
                    // A multi-character token that is not one of the named
                    // keys is not something this understands.
                    if chars.next().is_some() {
                        return None;
                    }
                    ShortcutKey::Char(c)
                }
            });
        }

        key.map(|key| Shortcut { command, shift, alt, key })
    }

    /// Whether a platform menu bar should claim this shortcut.
    ///
    /// Only the ones holding the command key. A bare letter or `Delete`
    /// registered as a menu accelerator fires before the window sees the key,
    /// which would make it impossible to type a `T` into a label or to
    /// backspace inside one.
    pub fn is_claimable(&self) -> bool {
        self.command
    }
}

/// What one line of an open menu is.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// Runs its action.
    Command,
    /// Runs its action and shows whether the thing it controls is on.
    Check(bool),
    /// A rule between groups of commands.
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// Printed right-aligned in the menu, for example `⌘S`. Empty when the
    /// command has no binding.
    pub shortcut: String,
    pub action: UiAction,
    pub kind: ItemKind,
    pub enabled: bool,
}

impl MenuItem {
    fn new(label: &str, shortcut: &str, action: UiAction) -> Self {
        MenuItem {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action,
            kind: ItemKind::Command,
            enabled: true,
        }
    }

    fn check(label: &str, shortcut: &str, action: UiAction, on: bool) -> Self {
        MenuItem { kind: ItemKind::Check(on), ..MenuItem::new(label, shortcut, action) }
    }

    fn separator() -> Self {
        MenuItem {
            label: String::new(),
            shortcut: String::new(),
            action: UiAction::NoOp,
            kind: ItemKind::Separator,
            enabled: false,
        }
    }

    fn enabled_when(mut self, condition: bool) -> Self {
        self.enabled = condition;
        self
    }

    pub fn is_separator(&self) -> bool {
        self.kind == ItemKind::Separator
    }

    /// Whether clicking this line does anything. A rule and a dimmed command
    /// are both inert, and the chrome refuses both for the same reason.
    pub fn is_clickable(&self) -> bool {
        self.enabled && !self.is_separator()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
}

/// Builds every menu for the current state.
pub fn menus(editor: &Editor, view: &ViewState) -> Vec<Menu> {
    vec![
        file_menu(editor),
        edit_menu(editor),
        select_menu(editor),
        insert_menu(editor, view),
        format_menu(editor),
        arrange_menu(editor),
        view_menu(editor, view),
        help_menu(),
    ]
}

fn file_menu(editor: &Editor) -> Menu {
    let infinite = editor.page_is_infinite();
    let size = editor.page_size_name();
    let named = |label: &str, name: &str| {
        MenuItem::check(
            label,
            "",
            UiAction::SetPageSize(name.to_string()),
            !infinite && size == name,
        )
    };

    Menu {
        title: "File".into(),
        items: vec![
            MenuItem::new("New", "⌘N", UiAction::NewDocument),
            MenuItem::new("New from Template…", "", UiAction::OpenTemplateGallery),
            MenuItem::new("Open…", "⌘O", UiAction::Open),
            MenuItem::new("Import Visio…", "", UiAction::ImportVsdx),
            MenuItem::separator(),
            MenuItem::new("Save", "⌘S", UiAction::Save),
            MenuItem::new("Save As…", "Shift ⌘S", UiAction::SaveAs),
            MenuItem::separator(),
            MenuItem::new("Export as PNG…", "Shift ⌘E", UiAction::ExportPng),
            MenuItem::new("Export as SVG…", "", UiAction::ExportVector { pdf: false }),
            MenuItem::new("Export as PDF…", "", UiAction::ExportVector { pdf: true }),
            MenuItem::separator(),
            // Lucidchart keeps page setup under File, and so does this.
            named("Page size: Letter", "letter"),
            named("Page size: Legal", "legal"),
            named("Page size: Tabloid", "tabloid"),
            named("Page size: A3", "a3"),
            named("Page size: A4", "a4"),
            MenuItem::check("Infinite canvas", "", UiAction::ToggleInfiniteCanvas, infinite),
            MenuItem::separator(),
            // macOS lifts this into the application menu, where every other
            // program keeps it; see `native_menu`.
            MenuItem::new("Settings…", "⌘,", UiAction::OpenSettings),
            MenuItem::separator(),
            MenuItem::new("Quit", "⌘Q", UiAction::Quit),
        ],
    }
}

fn edit_menu(editor: &Editor) -> Menu {
    let has_selection = !editor.selection.is_empty();
    Menu {
        title: "Edit".into(),
        items: vec![
            MenuItem::new("Undo", "⌘Z", UiAction::Undo).enabled_when(editor.can_undo()),
            MenuItem::new("Redo", "Shift ⌘Z", UiAction::Redo).enabled_when(editor.can_redo()),
            MenuItem::separator(),
            MenuItem::new("Cut", "⌘X", UiAction::Cut).enabled_when(has_selection),
            MenuItem::new("Copy", "⌘C", UiAction::Copy).enabled_when(has_selection),
            MenuItem::new("Paste", "⌘V", UiAction::Paste),
            MenuItem::new("Duplicate", "⌘D", UiAction::Duplicate).enabled_when(has_selection),
            MenuItem::new("Delete", "Del", UiAction::Delete).enabled_when(has_selection),
            MenuItem::separator(),
            MenuItem::new("Copy Style", "Alt ⌘C", UiAction::CopyStyle).enabled_when(has_selection),
            MenuItem::new("Paste Style", "Alt ⌘V", UiAction::PasteStyle)
                .enabled_when(has_selection && editor.has_copied_style()),
            MenuItem::separator(),
            MenuItem::new("Edit Label", "Enter", UiAction::EditLabel).enabled_when(has_selection),
            MenuItem::separator(),
            MenuItem::new("Rename Page…", "", UiAction::RenamePage(editor.page_index)),
            MenuItem::separator(),
            MenuItem::new("Find…", "⌘F", UiAction::ToggleFind),
        ],
    }
}

/// Lucidchart's Select menu: groupings of what is on the page.
fn select_menu(editor: &Editor) -> Menu {
    let has_selection = !editor.selection.is_empty();
    Menu {
        title: "Select".into(),
        items: vec![
            MenuItem::new("All", "⌘A", UiAction::SelectAll),
            MenuItem::new("None", "Esc", UiAction::SelectNone).enabled_when(has_selection),
            MenuItem::separator(),
            MenuItem::new("All Shapes", "", UiAction::SelectAllShapes),
            MenuItem::new("All Lines", "", UiAction::SelectAllLines),
            MenuItem::separator(),
            MenuItem::new("Same Fill Color", "", UiAction::SelectMatching(SelectBy::FillColor))
                .enabled_when(has_selection),
            MenuItem::new("Same Line Color", "", UiAction::SelectMatching(SelectBy::LineColor))
                .enabled_when(has_selection),
            MenuItem::new("Same Shape Type", "", UiAction::SelectMatching(SelectBy::ShapeType))
                .enabled_when(has_selection),
        ],
    }
}

fn insert_menu(editor: &Editor, view: &ViewState) -> Menu {
    let mut items = vec![
        MenuItem::new("Text Box", "T", UiAction::SelectTool(Tool::Shape(ShapeType::TextBlock))),
        MenuItem::new("Connector", "C", UiAction::SelectTool(Tool::Connector)),
        MenuItem::separator(),
        MenuItem::new("Image…", "", UiAction::InsertImage),
        MenuItem::separator(),
        MenuItem::check(
            "Star Shape in My Shapes",
            "",
            UiAction::ToggleFavoriteShape,
            editor
                .selected_shape_type()
                .is_some_and(|t| view.favorites.contains(&t)),
        )
        .enabled_when(editor.selected_shape_type().is_some()),
        MenuItem::new("Save Page as Template", "", UiAction::SavePageAsTemplate)
            .enabled_when(!editor.page().title.trim().is_empty()),
        MenuItem::separator(),
        MenuItem::new("Page", "", UiAction::AddPage),
        MenuItem::new("Duplicate Page", "", UiAction::DuplicatePage(editor.page_index))
            .enabled_when(true),
        MenuItem::new("Delete Page", "", UiAction::DeletePage(editor.page_index))
            .enabled_when(editor.document().pages.len() > 1),
        MenuItem::separator(),
    ];

    // One item per CSV the package carries, naming the column the link
    // matches on. A document with no data directory shows nothing here
    // rather than a dimmed command that could never run.
    let sources = editor.data_sources();
    if !sources.is_empty() {
        for source in &sources {
            let Some(key) = source.headers.first() else {
                continue;
            };
            items.push(MenuItem::new(
                &format!("Link Data: {} by {}", source.name, key.trim()),
                "",
                UiAction::LinkData {
                    source: source.name.clone(),
                    key_column: key.trim().to_string(),
                },
            ));
        }
        items.push(MenuItem::separator());
    }

    // The palette can only show one library at a time, so choosing one here is
    // how the other six are reached.
    for library in ShapeLibrary::ALL {
        items.push(MenuItem::check(
            &format!("Shapes: {}", library.display_name()),
            "",
            UiAction::SelectLibrary(*library),
            view.library == *library,
        ));
    }

    Menu { title: "Insert".into(), items }
}

/// Text formatting. Lucidchart puts these on a context toolbar rather than in
/// a menu; in a desktop application they belong in the menu bar, where their
/// shortcuts are visible.
/// Whether Tidy Up has at least two shapes to arrange.
///
/// It acts on the selection when there is one and on the page otherwise, so
/// the condition has to ask about whichever of the two it would use.
fn can_tidy(editor: &Editor) -> bool {
    if editor.selection.is_empty() {
        editor.page().shapes().len() >= 2
    } else {
        editor
            .selection
            .iter()
            .filter(|id| editor.page().shape(id).is_some())
            .count()
            >= 2
    }
}

/// The data-driven colouring block of the Format menu.
///
/// One item per field the page actually carries, so the command names a
/// field that exists rather than asking for one to be typed.
fn conditional_items(editor: &Editor) -> Vec<MenuItem> {
    let fields = editor.data_fields();
    if fields.is_empty() {
        return Vec::new();
    }
    let mut items = vec![MenuItem::separator()];
    for field in fields {
        items.push(MenuItem::new(
            &format!("Colour by {field}"),
            "",
            UiAction::ColorByField(field.clone()),
        ));
    }
    if !editor.format_rules().is_empty() {
        items.push(MenuItem::new("Reapply Formatting", "", UiAction::ApplyFormatRules));
        items.push(MenuItem::new("Clear Formatting Rules", "", UiAction::ClearFormatRules));
    }
    items
}

fn format_menu(editor: &Editor) -> Menu {
    let can_format = !editor.selection.is_empty() || editor.is_editing();
    let on = |which: TextToggle| editor.text_style_is_on(which);

    Menu {
        title: "Format".into(),
        items: vec![
            MenuItem::check("Bold", "⌘B", UiAction::ToggleText(TextToggle::Bold), on(TextToggle::Bold))
                .enabled_when(can_format),
            MenuItem::check("Italic", "⌘I", UiAction::ToggleText(TextToggle::Italic), on(TextToggle::Italic))
                .enabled_when(can_format),
            MenuItem::check(
                "Underline",
                "⌘U",
                UiAction::ToggleText(TextToggle::Underline),
                on(TextToggle::Underline),
            )
            .enabled_when(can_format),
            MenuItem::check(
                "Strikethrough",
                "Shift ⌘X",
                UiAction::ToggleText(TextToggle::Strikethrough),
                on(TextToggle::Strikethrough),
            )
            .enabled_when(can_format),
            MenuItem::separator(),
            MenuItem::new("Bigger", "Shift ⌘.", UiAction::ScaleTextSize(1.15)).enabled_when(can_format),
            MenuItem::new("Smaller", "Shift ⌘,", UiAction::ScaleTextSize(1.0 / 1.15))
                .enabled_when(can_format),
            MenuItem::separator(),
            MenuItem::new("Align Text Left", "", UiAction::SetTextAlign(dc_model::TextAlign::Left))
                .enabled_when(can_format),
            MenuItem::new("Align Text Center", "", UiAction::SetTextAlign(dc_model::TextAlign::Center))
                .enabled_when(can_format),
            MenuItem::new("Align Text Right", "", UiAction::SetTextAlign(dc_model::TextAlign::Right))
                .enabled_when(can_format),
            MenuItem::separator(),
            MenuItem::new("Text Top", "", UiAction::SetVerticalAlign(dc_model::VerticalAlign::Top))
                .enabled_when(can_format),
            MenuItem::new("Text Middle", "", UiAction::SetVerticalAlign(dc_model::VerticalAlign::Center))
                .enabled_when(can_format),
            MenuItem::new("Text Bottom", "", UiAction::SetVerticalAlign(dc_model::VerticalAlign::Bottom))
                .enabled_when(can_format),
        ]
        .into_iter()
        .chain(conditional_items(editor))
        .collect(),
    }
}

fn arrange_menu(editor: &Editor) -> Menu {
    let count = editor.selection.len();
    let several = count >= 2;
    let three = count >= 3;

    Menu {
        title: "Arrange".into(),
        items: vec![
            MenuItem::new("Bring to Front", "Shift ⌘]", UiAction::BringToFront).enabled_when(count >= 1),
            MenuItem::new("Bring Forward", "⌘]", UiAction::BringForward).enabled_when(count >= 1),
            MenuItem::new("Send Backward", "⌘[", UiAction::SendBackward).enabled_when(count >= 1),
            MenuItem::new("Send to Back", "Shift ⌘[", UiAction::SendToBack).enabled_when(count >= 1),
            MenuItem::separator(),
            MenuItem::new("Align Left", "", UiAction::Align(Align::Left)).enabled_when(several),
            MenuItem::new("Align Center", "", UiAction::Align(Align::CenterHorizontally))
                .enabled_when(several),
            MenuItem::new("Align Right", "", UiAction::Align(Align::Right)).enabled_when(several),
            MenuItem::new("Align Top", "", UiAction::Align(Align::Top)).enabled_when(several),
            MenuItem::new("Align Middle", "", UiAction::Align(Align::Middle)).enabled_when(several),
            MenuItem::new("Align Bottom", "", UiAction::Align(Align::Bottom)).enabled_when(several),
            MenuItem::separator(),
            MenuItem::new("Distribute Horizontally", "", UiAction::Distribute(true))
                .enabled_when(three),
            MenuItem::new("Distribute Vertically", "", UiAction::Distribute(false))
                .enabled_when(three),
            MenuItem::separator(),
            MenuItem::new("Match Width", "", UiAction::MatchSize(true, false)).enabled_when(several),
            MenuItem::new("Match Height", "", UiAction::MatchSize(false, true)).enabled_when(several),
            MenuItem::new("Match Size", "", UiAction::MatchSize(true, true)).enabled_when(several),
            MenuItem::separator(),
            MenuItem::new("Rotate Left", "", UiAction::Rotate(-90.0)).enabled_when(count >= 1),
            MenuItem::new("Rotate Right", "", UiAction::Rotate(90.0)).enabled_when(count >= 1),
            MenuItem::new("Flip Horizontal", "", UiAction::Flip(true)).enabled_when(count >= 1),
            MenuItem::new("Flip Vertical", "", UiAction::Flip(false)).enabled_when(count >= 1),
            MenuItem::separator(),
            MenuItem::new("Group", "⌘G", UiAction::Group).enabled_when(several),
            MenuItem::new("Ungroup", "Shift ⌘G", UiAction::Ungroup)
                .enabled_when(editor.selection_is_grouped()),
            MenuItem::separator(),
            MenuItem::check(
                "Lock",
                "",
                UiAction::SetLocked(!editor.selection_is_locked()),
                editor.selection_is_locked(),
            )
            .enabled_when(editor.selection_has_shapes()),
            MenuItem::new("Tidy Up (Vertical)", "", UiAction::TidyLayout(true))
                .enabled_when(can_tidy(editor)),
            MenuItem::new("Tidy Up (Horizontal)", "", UiAction::TidyLayout(false))
                .enabled_when(can_tidy(editor)),
            MenuItem::separator(),
            MenuItem::new("Fit Container to Contents", "", UiAction::FitContainer)
                .enabled_when(editor.selection_has_container()),
            MenuItem::separator(),
            MenuItem::new("Unlock All", "", UiAction::UnlockAll)
                .enabled_when(editor.page().shapes().iter().any(|s| s.is_locked())),
        ],
    }
}

fn view_menu(editor: &Editor, view: &ViewState) -> Menu {
    Menu {
        title: "View".into(),
        items: vec![
            MenuItem::new("Zoom In", "⌘+", UiAction::ZoomIn),
            MenuItem::new("Zoom Out", "⌘-", UiAction::ZoomOut),
            MenuItem::new("Zoom to Fit", "⌘1", UiAction::ZoomFit),
            MenuItem::new("Zoom to Selection", "", UiAction::ZoomToSelection)
                .enabled_when(!editor.selection.is_empty()),
            MenuItem::new("Actual Size", "⌘0", UiAction::ActualSize),
            MenuItem::separator(),
            MenuItem::check("Show Grid", "", UiAction::ToggleGrid, view.settings.show_grid),
            MenuItem::check("Show Page Bounds", "", UiAction::TogglePageBounds, view.settings.show_page_bounds),
            MenuItem::check(
                "Show Rulers",
                "",
                UiAction::ChangeSetting(crate::ui::SettingChange::ShowRulers(
                    !view.settings.show_rulers,
                )),
                view.settings.show_rulers,
            ),
            MenuItem::new("Guides from Selection", "", UiAction::GuidesFromSelection)
                .enabled_when(!editor.selection.is_empty()),
            MenuItem::new("Clear Guides", "", UiAction::ClearGuides)
                .enabled_when(!editor.page_guides().is_empty()),
            MenuItem::check("Snap to Guides", "", UiAction::ToggleSnap, editor.snap_enabled),
            MenuItem::check(
                "Route Around Shapes",
                "",
                UiAction::ToggleRouting,
                view.settings.route_around_shapes,
            ),
            MenuItem::separator(),
            MenuItem::check("Dark Theme", "", UiAction::ToggleTheme, view.settings.dark_theme),
            MenuItem::separator(),
            MenuItem::check("Layers", "", UiAction::ToggleLayersPanel, view.layers_panel_open),
        ],
    }
}

/// The right-click menu for whatever was under the pointer.
///
/// Lucidchart's context menu changes with what you click: a shape or a
/// selection gets the commands that act on it, blank canvas gets paste and
/// view commands, and a page tab gets the commands that act on the page it
/// names rather than on the current selection.
pub fn context_menu(editor: &Editor, target: ContextTarget) -> Menu {
    match target {
        ContextTarget::Canvas if editor.selection.is_empty() => canvas_context_menu(),
        ContextTarget::Canvas => selection_context_menu(editor),
        ContextTarget::Page(page) => page_context_menu(editor, page),
    }
}

/// A right click on blank canvas: nothing is selected, so only the commands
/// that do not need a selection make sense.
fn canvas_context_menu() -> Menu {
    Menu {
        title: String::new(),
        items: vec![
            MenuItem::new("Paste", "⌘V", UiAction::Paste),
            MenuItem::separator(),
            MenuItem::new("Select All", "⌘A", UiAction::SelectAll),
            MenuItem::separator(),
            MenuItem::new("Zoom In", "⌘+", UiAction::ZoomIn),
            MenuItem::new("Zoom Out", "⌘-", UiAction::ZoomOut),
            MenuItem::new("Zoom to Fit", "⌘1", UiAction::ZoomFit),
            MenuItem::new("Actual Size", "⌘0", UiAction::ActualSize),
        ],
    }
}

/// A right click on a shape, line, or multi-selection: `select_for_context_menu`
/// has already settled what the click means, so every command here has
/// something to act on.
fn selection_context_menu(editor: &Editor) -> Menu {
    let several = editor.selection.len() >= 2;

    Menu {
        title: String::new(),
        items: vec![
            MenuItem::new("Cut", "⌘X", UiAction::Cut),
            MenuItem::new("Copy", "⌘C", UiAction::Copy),
            MenuItem::new("Paste", "⌘V", UiAction::Paste),
            MenuItem::new("Duplicate", "⌘D", UiAction::Duplicate),
            MenuItem::new("Delete", "Del", UiAction::Delete),
            MenuItem::separator(),
            MenuItem::new("Copy Style", "Alt ⌘C", UiAction::CopyStyle),
            MenuItem::new("Paste Style", "Alt ⌘V", UiAction::PasteStyle)
                .enabled_when(editor.has_copied_style()),
            MenuItem::separator(),
            MenuItem::new("Edit Label", "Enter", UiAction::EditLabel),
            MenuItem::separator(),
            MenuItem::new("Open Link", "", UiAction::OpenLink)
                .enabled_when(editor.selection_link().is_some()),
            MenuItem::separator(),
            MenuItem::new("Zoom to Selection", "", UiAction::ZoomToSelection),
            MenuItem::separator(),
            MenuItem::new("Bring to Front", "Shift ⌘]", UiAction::BringToFront),
            MenuItem::new("Bring Forward", "⌘]", UiAction::BringForward),
            MenuItem::new("Send Backward", "⌘[", UiAction::SendBackward),
            MenuItem::new("Send to Back", "Shift ⌘[", UiAction::SendToBack),
            MenuItem::separator(),
            MenuItem::new("Rotate Left", "", UiAction::Rotate(-90.0)),
            MenuItem::new("Rotate Right", "", UiAction::Rotate(90.0)),
            MenuItem::new("Flip Horizontal", "", UiAction::Flip(true)),
            MenuItem::new("Flip Vertical", "", UiAction::Flip(false)),
            MenuItem::separator(),
            MenuItem::new("Group", "⌘G", UiAction::Group).enabled_when(several),
            MenuItem::new("Ungroup", "Shift ⌘G", UiAction::Ungroup)
                .enabled_when(editor.selection_is_grouped()),
            MenuItem::separator(),
            MenuItem::check(
                "Lock",
                "",
                UiAction::SetLocked(!editor.selection_is_locked()),
                editor.selection_is_locked(),
            )
            .enabled_when(editor.selection_has_shapes()),
            MenuItem::new("Tidy Up (Vertical)", "", UiAction::TidyLayout(true))
                .enabled_when(can_tidy(editor)),
            MenuItem::new("Tidy Up (Horizontal)", "", UiAction::TidyLayout(false))
                .enabled_when(can_tidy(editor)),
            MenuItem::separator(),
            MenuItem::new("Fit Container to Contents", "", UiAction::FitContainer)
                .enabled_when(editor.selection_has_container()),
            MenuItem::separator(),
            MenuItem::new("Unlock All", "", UiAction::UnlockAll)
                .enabled_when(editor.page().shapes().iter().any(|s| s.is_locked())),
        ],
    }
}

/// A right click on a page tab: the commands act on that page, whether or
/// not it is the one currently showing.
fn page_context_menu(editor: &Editor, page: usize) -> Menu {
    Menu {
        title: String::new(),
        items: vec![
            MenuItem::new("New Page", "", UiAction::AddPage),
            MenuItem::new("Duplicate Page", "", UiAction::DuplicatePage(page)),
            MenuItem::new("Rename Page…", "", UiAction::RenamePage(page)),
            MenuItem::separator(),
            MenuItem::new("Delete Page", "", UiAction::DeletePage(page))
                .enabled_when(editor.document().pages.len() > 1),
        ],
    }
}

fn help_menu() -> Menu {
    Menu {
        title: "Help".into(),
        items: vec![
            MenuItem::new("Keyboard Shortcuts", "?", UiAction::ShowShortcuts),
            MenuItem::separator(),
            MenuItem::new("About DiagramCraft", "", UiAction::ShowAbout),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::BoundingBox;

    fn editor() -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(900.0, 700.0);
        e
    }

    fn built(editor: &Editor) -> Vec<Menu> {
        menus(editor, &ViewState::default())
    }

    fn item<'a>(menus: &'a [Menu], menu: &str, label: &str) -> &'a MenuItem {
        menus
            .iter()
            .find(|m| m.title == menu)
            .unwrap_or_else(|| panic!("no {menu} menu"))
            .items
            .iter()
            .find(|i| i.label == label)
            .unwrap_or_else(|| panic!("no {label:?} in {menu}"))
    }

    #[test]
    fn the_menu_bar_matches_the_published_structure() {
        let menus = built(&editor());
        let titles: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["File", "Edit", "Select", "Insert", "Format", "Arrange", "View", "Help"]
        );
    }

    #[test]
    fn a_document_with_no_data_offers_no_link_command() {
        let insert = built(&editor()).into_iter().find(|m| m.title == "Insert").unwrap();
        assert!(
            !insert.items.iter().any(|i| i.label.starts_with("Link Data")),
            "a dimmed command that could never run is worse than none"
        );
    }

    #[test]
    fn each_bundled_csv_gets_a_link_command_naming_its_key_column() {
        let mut e = editor();
        e.package
            .data
            .insert("records.csv".into(), b"id,owner\n1,ops\n".to_vec());

        let insert = built(&e).into_iter().find(|m| m.title == "Insert").unwrap();
        let item = insert
            .items
            .iter()
            .find(|i| i.label.starts_with("Link Data"))
            .expect("the source should be offered");
        assert_eq!(item.label, "Link Data: records.csv by id");
        assert_eq!(
            item.action,
            UiAction::LinkData { source: "records.csv".into(), key_column: "id".into() }
        );
    }

    #[test]
    fn every_menu_has_items() {
        for menu in built(&editor()) {
            assert!(!menu.items.is_empty(), "{} is empty", menu.title);
        }
    }

    #[test]
    fn every_menu_has_something_to_run_once_there_is_a_selection() {
        // With nothing selected some menus are legitimately all dimmed; with
        // a shape selected every menu must offer at least one live command.
        let mut e = editor();
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        for menu in built(&e) {
            assert!(
                menu.items.iter().any(|i| i.is_clickable()),
                "{} has nothing that can be clicked",
                menu.title
            );
        }
    }

    #[test]
    fn formatting_is_dimmed_with_nothing_to_format() {
        let menus = built(&editor());
        assert!(!item(&menus, "Format", "Bold").enabled);
        assert!(!item(&menus, "Format", "Bigger").enabled);
    }

    #[test]
    fn no_menu_starts_or_ends_with_a_rule() {
        for menu in built(&editor()) {
            assert!(!menu.items.first().unwrap().is_separator(), "{}", menu.title);
            assert!(!menu.items.last().unwrap().is_separator(), "{}", menu.title);
        }
    }

    #[test]
    fn no_menu_has_two_rules_in_a_row() {
        for menu in built(&editor()) {
            for pair in menu.items.windows(2) {
                assert!(
                    !(pair[0].is_separator() && pair[1].is_separator()),
                    "{} has a doubled rule",
                    menu.title
                );
            }
        }
    }

    #[test]
    fn the_toolbar_commands_all_have_a_home_in_the_menus() {
        // Everything the old toolbar row carried must still be reachable.
        let menus = built(&editor());
        let actions: Vec<&UiAction> = menus
            .iter()
            .flat_map(|m| m.items.iter())
            .map(|i| &i.action)
            .collect();

        for expected in [
            UiAction::NewDocument,
            UiAction::Open,
            UiAction::Save,
            UiAction::SaveAs,
            UiAction::Undo,
            UiAction::Redo,
            UiAction::Duplicate,
            UiAction::Delete,
            UiAction::ZoomIn,
            UiAction::ZoomOut,
            UiAction::ZoomFit,
            UiAction::ToggleGrid,
            UiAction::ToggleSnap,
            UiAction::ToggleTheme,
        ] {
            assert!(actions.contains(&&expected), "{expected:?} has no menu item");
        }
    }

    #[test]
    fn the_new_features_are_reachable_from_the_menus() {
        let menus = built(&editor());
        let actions: Vec<&UiAction> = menus
            .iter()
            .flat_map(|m| m.items.iter())
            .map(|i| &i.action)
            .collect();

        assert!(actions.contains(&&UiAction::ImportVsdx), "no Visio import");
        assert!(actions.contains(&&UiAction::Group), "no grouping");
        assert!(
            actions.contains(&&UiAction::Align(Align::Left)),
            "no alignment"
        );
        assert!(actions.contains(&&UiAction::Distribute(true)), "no distribute");
        assert!(actions.contains(&&UiAction::MatchSize(true, true)), "no match size");
        for library in ShapeLibrary::ALL {
            assert!(
                actions.contains(&&UiAction::SelectLibrary(*library)),
                "{} is unreachable",
                library.display_name()
            );
        }
    }

    #[test]
    fn zoom_to_selection_is_disabled_with_nothing_selected() {
        let mut e = editor();
        assert!(!item(&built(&e), "View", "Zoom to Selection").enabled);

        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.selection = vec![id];
        assert!(item(&built(&e), "View", "Zoom to Selection").enabled);
    }

    #[test]
    fn zoom_to_selection_is_in_the_shape_context_menu() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.selection = vec![id];
        let menu = context_menu(&e, ContextTarget::Canvas);
        assert!(menu.items.iter().any(|i| i.label == "Zoom to Selection" && i.enabled));
    }

    #[test]
    fn open_link_is_disabled_without_a_link_and_enabled_with_one() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.selection = vec![id];
        let menu = context_menu(&e, ContextTarget::Canvas);
        assert!(!menu.items.iter().find(|i| i.label == "Open Link").unwrap().enabled);

        e.set_link_url("https://example.com");
        let menu = context_menu(&e, ContextTarget::Canvas);
        assert!(menu.items.iter().find(|i| i.label == "Open Link").unwrap().enabled);
    }

    #[test]
    fn undo_is_disabled_until_there_is_something_to_undo() {
        let mut e = editor();
        assert!(!item(&built(&e), "Edit", "Undo").enabled);

        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        assert!(item(&built(&e), "Edit", "Undo").enabled);
        assert!(!item(&built(&e), "Edit", "Redo").enabled);

        e.undo();
        assert!(item(&built(&e), "Edit", "Redo").enabled);
    }

    #[test]
    fn selection_commands_are_disabled_with_nothing_selected() {
        let mut e = editor();
        for (menu, label) in [
            ("Edit", "Delete"),
            ("Edit", "Copy"),
            ("Arrange", "Bring to Front"),
            ("Select", "Same Fill Color"),
        ] {
            assert!(!item(&built(&e), menu, label).enabled, "{menu} > {label}");
        }

        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        for (menu, label) in [("Edit", "Delete"), ("Arrange", "Bring to Front")] {
            assert!(item(&built(&e), menu, label).enabled, "{menu} > {label}");
        }
    }

    #[test]
    fn align_needs_two_and_distribute_needs_three() {
        let mut e = editor();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        assert!(!item(&built(&e), "Arrange", "Align Left").enabled);

        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(50.0, 0.0, 10.0, 10.0));
        e.selection = vec![a.clone(), b.clone()];
        assert!(item(&built(&e), "Arrange", "Align Left").enabled);
        assert!(!item(&built(&e), "Arrange", "Distribute Horizontally").enabled);

        let c = e.create_shape(ShapeType::Rectangle, BoundingBox::new(100.0, 0.0, 10.0, 10.0));
        e.selection = vec![a, b, c];
        assert!(item(&built(&e), "Arrange", "Distribute Horizontally").enabled);
    }

    #[test]
    fn ungroup_is_offered_only_for_something_grouped() {
        let mut e = editor();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(50.0, 0.0, 10.0, 10.0));
        e.selection = vec![a, b];
        assert!(!item(&built(&e), "Arrange", "Ungroup").enabled);

        e.group_selection();
        assert!(item(&built(&e), "Arrange", "Ungroup").enabled);
    }

    #[test]
    fn deleting_the_only_page_is_not_offered() {
        let mut e = editor();
        assert!(!item(&built(&e), "Insert", "Delete Page").enabled);
        e.add_page();
        assert!(item(&built(&e), "Insert", "Delete Page").enabled);
    }

    #[test]
    fn view_checks_follow_their_state() {
        let e = editor();
        let with = |show_grid, dark_theme| ViewState {
            settings: Settings { show_grid, dark_theme, ..Default::default() },
            ..Default::default()
        };
        let on = with(true, false);
        let off = with(false, true);

        let checked = |view: &ViewState, label: &str| {
            let built = menus(&e, view);
            match item(&built, "View", label).kind {
                ItemKind::Check(state) => state,
                ref other => panic!("{label} is {other:?}, not a check"),
            }
        };
        assert!(checked(&on, "Show Grid"));
        assert!(!checked(&off, "Show Grid"));
        assert!(!checked(&on, "Dark Theme"));
        assert!(checked(&off, "Dark Theme"));
    }

    #[test]
    fn the_page_size_check_follows_the_page() {
        let mut e = editor();
        let letter = |e: &Editor| match item(&built(e), "File", "Page size: Letter").kind {
            ItemKind::Check(state) => state,
            _ => panic!("not a check"),
        };
        assert!(letter(&e), "a new page is Letter");

        e.set_page_size(dc_model::PageSize {
            size_type: "a4".into(),
            format: None,
            w: None,
            h: None,
        });
        assert!(!letter(&e));

        e.set_infinite_canvas(true);
        match item(&built(&e), "File", "Infinite canvas").kind {
            ItemKind::Check(true) => {}
            ref other => panic!("infinite canvas reads {other:?}"),
        }
    }

    #[test]
    fn formatting_checks_follow_the_selected_text() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("<b>bold</b>"));
        }
        e.selection = vec![id];

        match item(&built(&e), "Format", "Bold").kind {
            ItemKind::Check(true) => {}
            ref other => panic!("Bold reads {other:?}"),
        }
        match item(&built(&e), "Format", "Italic").kind {
            ItemKind::Check(false) => {}
            ref other => panic!("Italic reads {other:?}"),
        }
    }

    #[test]
    fn the_selected_library_is_checked() {
        let e = editor();
        let view = ViewState { library: ShapeLibrary::Bpmn, ..Default::default() };
        let built = menus(&e, &view);
        match item(&built, "Insert", "Shapes: BPMN 2.0").kind {
            ItemKind::Check(true) => {}
            ref other => panic!("BPMN reads {other:?}"),
        }
        match item(&built, "Insert", "Shapes: Standard").kind {
            ItemKind::Check(false) => {}
            ref other => panic!("Standard reads {other:?}"),
        }
    }

    #[test]
    fn shortcuts_are_shown_for_the_commands_that_have_them() {
        let menus = built(&editor());
        assert_eq!(item(&menus, "File", "Save").shortcut, "⌘S");
        assert_eq!(item(&menus, "Edit", "Undo").shortcut, "⌘Z");
        assert_eq!(item(&menus, "Format", "Bold").shortcut, "⌘B");
        assert_eq!(item(&menus, "Arrange", "Group").shortcut, "⌘G");
    }

    #[test]
    fn shortcuts_use_only_characters_the_bundled_font_can_draw() {
        // Liberation Sans has the command glyph but not the other Apple
        // modifier symbols, which render as missing-character boxes. Anything
        // it lacks is spelled out instead.
        for menu in built(&editor()) {
            for entry in &menu.items {
                for c in entry.shortcut.chars() {
                    assert!(
                        c.is_ascii() || c == '⌘',
                        "{} > {} uses {c:?}, which has no glyph",
                        menu.title,
                        entry.label
                    );
                }
            }
        }
    }

    #[test]
    fn shortcuts_parse_back_into_their_parts() {
        let s = Shortcut::parse("⌘S").unwrap();
        assert_eq!(
            s,
            Shortcut { command: true, shift: false, alt: false, key: ShortcutKey::Char('S') }
        );

        let s = Shortcut::parse("Shift ⌘Z").unwrap();
        assert_eq!(
            s,
            Shortcut { command: true, shift: true, alt: false, key: ShortcutKey::Char('Z') }
        );

        assert_eq!(
            Shortcut::parse("Del").unwrap(),
            Shortcut { command: false, shift: false, alt: false, key: ShortcutKey::Delete }
        );
        assert_eq!(Shortcut::parse("Esc").unwrap().key, ShortcutKey::Escape);
        assert_eq!(Shortcut::parse("Enter").unwrap().key, ShortcutKey::Enter);
        assert_eq!(Shortcut::parse("T").unwrap().key, ShortcutKey::Char('T'));
    }

    #[test]
    fn an_empty_or_unreadable_shortcut_parses_to_nothing() {
        assert_eq!(Shortcut::parse(""), None);
        assert_eq!(Shortcut::parse("   "), None);
        assert_eq!(Shortcut::parse("⌘"), None);
        assert_eq!(Shortcut::parse("Shift"), None);
        assert_eq!(Shortcut::parse("Gibberish"), None);
    }

    #[test]
    fn every_shortcut_in_the_menus_parses() {
        // A platform menu bar reads these; one that does not parse would
        // silently lose its accelerator.
        for menu in built(&editor()) {
            for entry in &menu.items {
                if entry.shortcut.is_empty() {
                    continue;
                }
                assert!(
                    Shortcut::parse(&entry.shortcut).is_some(),
                    "{} > {} has an unreadable shortcut {:?}",
                    menu.title,
                    entry.label,
                    entry.shortcut
                );
            }
        }
    }

    #[test]
    fn only_command_shortcuts_are_claimable_by_a_menu_bar() {
        // Letting a menu bar claim a bare key would stop it reaching an open
        // label, so `T` must never be claimable and `⌘B` must be.
        assert!(Shortcut::parse("⌘B").unwrap().is_claimable());
        assert!(Shortcut::parse("Shift ⌘S").unwrap().is_claimable());
        assert!(!Shortcut::parse("T").unwrap().is_claimable());
        assert!(!Shortcut::parse("Del").unwrap().is_claimable());
        assert!(!Shortcut::parse("Esc").unwrap().is_claimable());
        assert!(!Shortcut::parse("Enter").unwrap().is_claimable());
    }

    #[test]
    fn no_two_commands_share_a_claimable_shortcut() {
        // A menu bar dispatches by accelerator, so a duplicate would make one
        // of the two unreachable.
        let menus = built(&editor());
        let mut seen: Vec<(Shortcut, String)> = Vec::new();
        for menu in &menus {
            for entry in &menu.items {
                let Some(s) = Shortcut::parse(&entry.shortcut) else {
                    continue;
                };
                if !s.is_claimable() {
                    continue;
                }
                if let Some((_, other)) = seen.iter().find(|(existing, _)| *existing == s) {
                    panic!("{:?} is on both {other} and {}", entry.shortcut, entry.label);
                }
                seen.push((s, entry.label.clone()));
            }
        }
        assert!(seen.len() > 15, "only {} claimable shortcuts", seen.len());
    }

    #[test]
    fn shortcuts_are_short_enough_for_their_column() {
        for menu in built(&editor()) {
            for entry in &menu.items {
                assert!(
                    entry.shortcut.chars().count() <= 9,
                    "{} > {} has an overlong shortcut {:?}",
                    menu.title,
                    entry.label,
                    entry.shortcut
                );
            }
        }
    }

    #[test]
    fn no_action_other_than_a_separator_is_a_no_op() {
        for menu in built(&editor()) {
            for entry in &menu.items {
                if entry.is_separator() {
                    continue;
                }
                assert_ne!(
                    entry.action,
                    UiAction::NoOp,
                    "{} > {} does nothing",
                    menu.title,
                    entry.label
                );
            }
        }
    }

    #[test]
    fn menu_titles_are_unique() {
        let menus = built(&editor());
        for (i, menu) in menus.iter().enumerate() {
            assert!(
                !menus[..i].iter().any(|m| m.title == menu.title),
                "{} appears twice",
                menu.title
            );
        }
    }

    #[test]
    fn labels_within_a_menu_are_unique() {
        for menu in built(&editor()) {
            let labels: Vec<&str> = menu
                .items
                .iter()
                .filter(|i| !i.is_separator())
                .map(|i| i.label.as_str())
                .collect();
            for (i, label) in labels.iter().enumerate() {
                assert!(
                    !labels[..i].contains(label),
                    "{} lists {label:?} twice",
                    menu.title
                );
            }
        }
    }

    // --------------------------------------------------------- context menu

    #[test]
    fn blank_canvas_offers_paste_and_view_commands_but_nothing_about_a_selection() {
        let e = editor();
        let menu = context_menu(&e, ContextTarget::Canvas);
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Paste"));
        assert!(labels.contains(&"Select All"));
        assert!(labels.contains(&"Zoom to Fit"));
        assert!(!labels.contains(&"Delete"), "nothing is selected");
        assert!(!labels.contains(&"Cut"));
    }

    #[test]
    fn a_shape_under_the_click_offers_the_commands_that_act_on_it() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        e.selection = vec![id];
        let menu = context_menu(&e, ContextTarget::Canvas);
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        for expected in ["Cut", "Copy", "Paste", "Duplicate", "Delete", "Bring to Front", "Group"] {
            assert!(labels.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn group_is_only_offered_for_two_or_more() {
        let mut e = editor();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.selection = vec![id];
        let menu = context_menu(&e, ContextTarget::Canvas);
        let group = menu.items.iter().find(|i| i.label == "Group").unwrap();
        assert!(!group.enabled, "one shape cannot be grouped with itself");
    }

    #[test]
    fn a_page_tab_offers_page_commands_regardless_of_selection() {
        let mut e = editor();
        e.add_page();
        let menu = context_menu(&e, ContextTarget::Page(0));
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Duplicate Page"));
        assert!(labels.contains(&"Rename Page…"));
        let delete = menu.items.iter().find(|i| i.label == "Delete Page").unwrap();
        assert!(delete.enabled, "a second page exists, so the first can go");
    }

    #[test]
    fn the_only_page_cannot_be_deleted_from_its_context_menu() {
        let e = editor();
        let menu = context_menu(&e, ContextTarget::Page(0));
        let delete = menu.items.iter().find(|i| i.label == "Delete Page").unwrap();
        assert!(!delete.enabled);
    }

    #[test]
    fn every_context_menu_follows_the_same_shape_rules_as_the_bar_menus() {
        let e = editor();
        let mut selected = editor();
        let id = selected.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        selected.selection = vec![id];

        for menu in [
            context_menu(&e, ContextTarget::Canvas),
            context_menu(&selected, ContextTarget::Canvas),
            context_menu(&e, ContextTarget::Page(0)),
        ] {
            assert!(!menu.items.is_empty());
            assert!(!menu.items.first().unwrap().is_separator());
            assert!(!menu.items.last().unwrap().is_separator());
            for pair in menu.items.windows(2) {
                assert!(!(pair[0].is_separator() && pair[1].is_separator()));
            }
            for entry in &menu.items {
                if !entry.is_separator() {
                    assert_ne!(entry.action, UiAction::NoOp, "{} does nothing", entry.label);
                }
            }
            assert!(menu.items.iter().any(|i| i.is_clickable()));
        }
    }
}
