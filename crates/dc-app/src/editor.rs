//! Editor state and interaction.
//!
//! All editing lives here rather than in the window layer, so the entire
//! interaction model can be exercised without opening a window or a GPU
//! device. The window layer translates platform events into these calls and
//! asks for an [`Overlay`] to draw.

use crate::history::History;
use crate::snap;
use crate::textedit::{EditTarget, TextCommand, TextCtx, TextEdit};
use dc_lucid::LucidPackage;
use dc_model::{
    BoundingBox, Color, Document, Endpoint, EndpointStyle, Fill, Line, LineType, Page, Point,
    Shape, ShapeType, Stroke, Style, Text,
};
use dc_render::handles::{self, HandleKind};
use dc_render::{Camera, Overlay, SnapGuide};
use std::path::{Path, PathBuf};

/// The active tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Select, move, resize, and rotate.
    Select,
    /// Drag the canvas.
    Pan,
    /// Draw connectors between shapes.
    Connector,
    /// Place a shape of this type.
    Shape(ShapeType),
    /// Draw a freehand stroke.
    Freehand,
}

impl Default for Tool {
    fn default() -> Self {
        Tool::Select
    }
}

/// Keyboard modifiers relevant to editing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// Shift: additive selection, proportional resize, axis locking.
    pub shift: bool,
    /// Control or Command: toggle selection, bypass snapping.
    pub command: bool,
    pub alt: bool,
}

/// An in-progress pointer gesture.
#[derive(Clone, Debug)]
enum Drag {
    Move {
        start: Point,
        originals: Vec<(String, BoundingBox)>,
    },
    Resize {
        handle: HandleKind,
        start: Point,
        originals: Vec<(String, BoundingBox)>,
        /// The selection box when the drag began.
        original_bounds: BoundingBox,
    },
    Rotate {
        id: String,
        center: Point,
        /// Pointer angle at the start, in degrees.
        start_angle: f64,
        original_rotation: f64,
    },
    Marquee {
        start: Point,
        current: Point,
    },
    Pan {
        last_screen: Point,
    },
    Connector {
        from_shape: Option<String>,
        from: Point,
        current: Point,
    },
    Create {
        shape_type: ShapeType,
        start: Point,
        current: Point,
    },
    /// Recording a pen stroke, in document coordinates.
    Freehand { points: Vec<Point> },
    /// Dragging inside an active text edit to extend the selection.
    TextSelect,
}

/// A character style that the formatting controls switch on and off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextToggle {
    Bold,
    Italic,
    Underline,
    Strikethrough,
}

impl TextToggle {
    fn apply(self, style: &mut dc_model::TextStyle, on: bool) {
        match self {
            TextToggle::Bold => style.bold = on,
            TextToggle::Italic => style.italic = on,
            TextToggle::Underline => style.underline = on,
            TextToggle::Strikethrough => style.strikethrough = on,
        }
    }

    fn probe(self) -> fn(&dc_model::TextStyle) -> bool {
        match self {
            TextToggle::Bold => |s: &dc_model::TextStyle| s.bold,
            TextToggle::Italic => |s: &dc_model::TextStyle| s.italic,
            TextToggle::Underline => |s: &dc_model::TextStyle| s.underline,
            TextToggle::Strikethrough => |s: &dc_model::TextStyle| s.strikethrough,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TextToggle::Bold => "bold",
            TextToggle::Italic => "italic",
            TextToggle::Underline => "underline",
            TextToggle::Strikethrough => "strikethrough",
        }
    }

    /// The same property as the text editor names it.
    pub fn as_style_toggle(self) -> crate::textedit::StyleToggle {
        use crate::textedit::StyleToggle;
        match self {
            TextToggle::Bold => StyleToggle::Bold,
            TextToggle::Italic => StyleToggle::Italic,
            TextToggle::Underline => StyleToggle::Underline,
            TextToggle::Strikethrough => StyleToggle::Strikethrough,
        }
    }
}

/// Which end of a connector a command acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnd {
    Start,
    End,
}

impl LineEnd {
    pub fn name(self) -> &'static str {
        match self {
            LineEnd::Start => "start",
            LineEnd::End => "end",
        }
    }
}

/// A style lifted off one item to be painted onto others.
#[derive(Clone, Debug, PartialEq)]
pub struct CopiedStyle {
    /// The whole style, for a shape. Absent when the style came off a
    /// connector, which has only a stroke.
    pub style: Option<Style>,
    pub stroke: Option<Stroke>,
    pub opacity: Option<f64>,
}

/// Which edge or axis the align commands line a selection up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    CenterHorizontally,
    Right,
    Top,
    Middle,
    Bottom,
}

impl Align {
    pub fn name(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::CenterHorizontally => "center",
            Align::Right => "right",
            Align::Top => "top",
            Align::Middle => "middle",
            Align::Bottom => "bottom",
        }
    }

}

/// What the Select menu's "same …" commands match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectBy {
    FillColor,
    LineColor,
    ShapeType,
}

/// The value a [`SelectBy`] compares, kept as a comparable key so shapes and
/// connectors can be matched through the same path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchKey {
    Color(Color),
    Type(String),
}

impl SelectBy {
    pub fn name(self) -> &'static str {
        match self {
            SelectBy::FillColor => "fill color",
            SelectBy::LineColor => "line color",
            SelectBy::ShapeType => "shape type",
        }
    }

    /// The key an item presents for this criterion, or `None` when the item
    /// has nothing to compare: a connector has no fill, for instance.
    fn key_of(self, page: &Page, id: &str) -> Option<MatchKey> {
        match self {
            SelectBy::FillColor => {
                let shape = page.shape(id)?;
                shape.resolved_style().resolved_fill().color().map(MatchKey::Color)
            }
            SelectBy::LineColor => {
                if let Some(shape) = page.shape(id) {
                    return Some(MatchKey::Color(shape.effective_stroke().resolved_color()));
                }
                let line = page.line(id)?;
                Some(MatchKey::Color(line.resolved_stroke().resolved_color()))
            }
            SelectBy::ShapeType => {
                let shape = page.shape(id)?;
                Some(MatchKey::Type(shape.shape_type.as_str().to_string()))
            }
        }
    }
}

/// Minimum drag distance, in screen pixels, before a press becomes a drag.
const DRAG_THRESHOLD_PX: f64 = 3.0;
/// How close a click must be to a connector to select it, in screen pixels.
const LINE_PICK_SLOP_PX: f64 = 6.0;

pub struct Editor {
    pub package: LucidPackage,
    pub page_index: usize,
    pub camera: Camera,
    pub selection: Vec<String>,
    pub tool: Tool,
    pub hovered: Option<String>,
    pub attach_target: Option<String>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Whether a drag aligns to the edges and centres of other shapes.
    pub snap_enabled: bool,
    /// Whether a drag with nothing to align to falls back to the grid.
    pub snap_to_grid: bool,
    pub snap_threshold_px: f64,
    pub grid_spacing: f64,
    /// The routing a newly drawn connector takes.
    pub default_line_type: LineType,
    /// The units a new document is measured in.
    pub default_units: dc_model::Units,
    /// The paper size a new document starts at.
    pub default_page_size: String,
    pub status: String,

    /// The active text editing session, if any.
    pub text_edit: Option<TextEdit>,
    /// A style picked up by the paint-format command.
    copied_style: Option<CopiedStyle>,

    history: History,
    drag: Option<Drag>,
    guides: Vec<SnapGuide>,
    /// Where the press began, used to tell a click from a drag.
    press_screen: Option<Point>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            package: LucidPackage::new(Document::new()),
            page_index: 0,
            camera: Camera::default(),
            selection: Vec::new(),
            tool: Tool::default(),
            hovered: None,
            attach_target: None,
            path: None,
            dirty: false,
            snap_enabled: true,
            snap_to_grid: true,
            snap_threshold_px: 6.0,
            grid_spacing: 9.0,
            default_line_type: LineType::Elbow,
            default_units: dc_model::Units::In,
            default_page_size: "letter".to_string(),
            status: String::new(),
            text_edit: None,
            copied_style: None,
            history: History::default(),
            drag: None,
            guides: Vec::new(),
            press_screen: None,
        }
    }

    // ---------------------------------------------------------------- access

    pub fn document(&self) -> &Document {
        &self.package.document
    }

    /// The current page. A document always has at least one page; loading
    /// repairs an empty one, so this never fails.
    pub fn page(&self) -> &Page {
        let index = self.page_index.min(self.package.document.pages.len() - 1);
        &self.package.document.pages[index]
    }

    fn page_mut(&mut self) -> &mut Page {
        let index = self.page_index.min(self.package.document.pages.len() - 1);
        &mut self.package.document.pages[index]
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The window title, marking unsaved changes with a leading dot.
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        if self.dirty {
            format!("• {name} — DiagramCraft")
        } else {
            format!("{name} — DiagramCraft")
        }
    }

    /// The overlay describing current interaction state.
    pub fn overlay(&self) -> Overlay<'_> {
        let (marquee, pending_connector) = match &self.drag {
            Some(Drag::Marquee { start, current }) => {
                (Some(BoundingBox::from_corners(*start, *current)), None)
            }
            Some(Drag::Create { start, current, .. }) => {
                (Some(BoundingBox::from_corners(*start, *current)), None)
            }
            // The real stroke only exists once the drag ends, so the bounds
            // it will land in stand in for a preview, the same way a shape
            // being dragged out shows its box rather than its final outline.
            Some(Drag::Freehand { points }) if points.len() >= 2 => {
                let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
                let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
                for p in points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                (Some(BoundingBox::new(min_x, min_y, max_x - min_x, max_y - min_y)), None)
            }
            Some(Drag::Connector { from, current, .. }) => (None, Some((*from, *current))),
            _ => (None, None),
        };

        Overlay {
            selected: &self.selection,
            hovered: self.hovered.as_deref(),
            attach_target: self.attach_target.as_deref(),
            marquee,
            snap_guides: &self.guides,
            pending_connector,
            editing: self.editing_id(),
        }
    }

    // ------------------------------------------------------------- lifecycle

    fn begin_change(&mut self) {
        self.history.record(&self.package.document);
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(doc) = self.history.undo(&self.package.document) {
            self.package.document = doc;
            self.after_history_change();
        }
    }

    pub fn redo(&mut self) {
        if let Some(doc) = self.history.redo(&self.package.document) {
            self.package.document = doc;
            self.after_history_change();
        }
    }

    fn after_history_change(&mut self) {
        self.dirty = true;
        self.ensure_page_exists();
        self.page_index = self
            .page_index
            .min(self.package.document.pages.len().saturating_sub(1));
        // Selection may name items the restored document no longer has.
        let surviving: Vec<String> = {
            let page = self.page();
            self.selection
                .iter()
                .filter(|id| page.shape(id).is_some() || page.line(id).is_some())
                .cloned()
                .collect()
        };
        self.selection = surviving;
        self.drag = None;
        self.guides.clear();
    }

    fn ensure_page_exists(&mut self) {
        if self.package.document.pages.is_empty() {
            self.package.document.pages.push(Page::new("page1", "Page 1"));
        }
    }

    pub fn new_document(&mut self) {
        let mut document = Document::new();
        // A new document starts the way the settings say it should.
        document.set_units(self.default_units);
        document.pages[0].settings = Some(dc_model::PageSettings {
            size: Some(dc_model::PageSize {
                size_type: self.default_page_size.clone(),
                format: None,
                w: None,
                h: None,
            }),
            ..Default::default()
        });

        self.load_document(document);
        self.status = "New document".into();
    }

    /// Replaces the open document with one built from a template, the same
    /// way `new_document` replaces it with a blank one.
    pub fn new_document_from_template(&mut self, document: Document, name: &str) {
        self.load_document(document);
        self.status = format!("New document from {name}");
    }

    /// The reset shared by starting a blank document and starting one from
    /// a template: a fresh package with no selection, no undo history, and
    /// the camera back at its default framing.
    fn load_document(&mut self, document: Document) {
        self.package = LucidPackage::new(document);
        self.page_index = 0;
        self.selection.clear();
        self.path = None;
        self.dirty = false;
        self.history.clear();
        self.drag = None;
        self.camera.reset();
    }

    pub fn open_path(&mut self, path: impl AsRef<Path>) -> Result<(), dc_lucid::Error> {
        let path = path.as_ref();
        let package = LucidPackage::read_path(path)?;
        self.package = package;
        self.ensure_page_exists();
        self.page_index = 0;
        self.selection.clear();
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.history.clear();
        self.drag = None;

        // The status bar is narrow, so report the file name rather than the
        // whole path, which would overflow it.
        let name = file_label(path);
        let issues = self.package.validate();
        self.status = if issues.is_empty() {
            format!("Opened {name}")
        } else {
            format!("Opened {name} with {} issue(s)", issues.len())
        };
        self.zoom_to_fit();
        Ok(())
    }

    /// Replaces the document with an imported Visio drawing.
    ///
    /// The result is a new untitled document rather than a save over the
    /// `.vsdx`: this build writes `.lucid`, and quietly turning someone's
    /// Visio file into a different format under the same name would be a
    /// surprise.
    pub fn import_vsdx(&mut self, path: impl AsRef<Path>) -> Result<(), dc_vsdx::Error> {
        let path = path.as_ref();
        let imported = dc_vsdx::read_path(path)?;
        let summary = imported.summary();

        self.package = dc_lucid::LucidPackage::new(imported.document);
        self.ensure_page_exists();
        self.page_index = 0;
        self.selection.clear();
        // Saving must prompt for a `.lucid` name rather than overwrite the
        // drawing that was imported.
        self.path = None;
        self.dirty = true;
        self.history.clear();
        self.drag = None;
        self.text_edit = None;

        self.status = format!("{summary} from {}", file_label(path));
        for warning in imported.warnings.iter().take(5) {
            log::warn!("importing {}: {warning}", path.display());
        }
        self.zoom_to_fit();
        Ok(())
    }

    /// Adds an image file to the package and places it as an image block.
    pub fn insert_image(&mut self, path: impl AsRef<Path>) -> std::io::Result<String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let name = dc_model::sanitize_id(&file_label(path));

        // Centre it in the view at a size that fits without dominating.
        let center = self.camera.screen_to_doc(Point::new(
            self.camera.viewport.x / 2.0,
            self.camera.viewport.y / 2.0,
        ));
        let (w, h) = (240.0, 180.0);
        let bounds = BoundingBox::new(center.x - w / 2.0, center.y - h / 2.0, w, h);

        let id = self.package.document.fresh_id("shape");
        let mut shape = Shape::new(&id, ShapeType::Image, bounds);
        shape.image = Some(dc_model::ImageRef { r#ref: Some(name.clone()), url: None });

        self.begin_change();
        self.package.images.insert(name, bytes);
        self.page_mut().add_shape(shape);
        self.selection = vec![id.clone()];
        self.status = format!("Inserted {}", file_label(path));
        Ok(id)
    }

    /// Sets the current page's paper size.
    pub fn set_page_size(&mut self, size: dc_model::PageSize) {
        self.begin_change();
        let settings = self.page_mut().settings.get_or_insert_with(Default::default);
        settings.size = Some(size);
        settings.infinite_canvas = Some(false);
        self.status = "Page size changed".into();
    }

    /// Switches the current page between a fixed sheet and an endless canvas.
    pub fn set_infinite_canvas(&mut self, infinite: bool) {
        self.begin_change();
        let settings = self.page_mut().settings.get_or_insert_with(Default::default);
        settings.infinite_canvas = Some(infinite);
        self.status = if infinite {
            "Infinite canvas".into()
        } else {
            "Fixed page size".into()
        };
    }

    /// The current page's size, as the page setup menu reports it.
    pub fn page_size_name(&self) -> String {
        let settings = self.page().resolved_settings();
        if settings.is_infinite() {
            return "infinite".to_string();
        }
        settings.size.unwrap_or_default().size_type
    }

    pub fn page_is_infinite(&self) -> bool {
        self.page().resolved_settings().is_infinite()
    }

    pub fn save_path(&mut self, path: impl AsRef<Path>) -> Result<(), dc_lucid::Error> {
        let path = path.as_ref();
        self.package.write_path(path)?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        self.status = format!("Saved {}", file_label(path));
        Ok(())
    }

    /// Saves to the current path, if there is one.
    pub fn save(&mut self) -> Option<Result<(), dc_lucid::Error>> {
        let path = self.path.clone()?;
        Some(self.save_path(path))
    }

    // ------------------------------------------------------------ navigation

    pub fn resize_viewport(&mut self, width: f64, height: f64) {
        self.camera.viewport = kurbo::Vec2::new(width.max(1.0), height.max(1.0));
    }

    pub fn zoom_to_fit(&mut self) {
        match self.page().content_bounds() {
            Some(bounds) => self.camera.fit(bounds.inflated(40.0), 24.0),
            None => {
                // An empty page frames the sheet instead.
                let units = self.package.document.units();
                match self.page().resolved_settings().bounds_points(units) {
                    Some((w, h)) => self.camera.fit(BoundingBox::new(0.0, 0.0, w, h), 24.0),
                    None => self.camera.reset(),
                }
            }
        }
    }

    /// Frames the current selection, the same way `zoom_to_fit` frames the
    /// page. Does nothing with no selection, rather than resetting the view.
    pub fn zoom_to_selection(&mut self) {
        if let Some(bounds) = self.selection_bounds() {
            self.camera.fit(bounds.inflated(40.0), 24.0);
        }
    }

    pub fn zoom_by(&mut self, factor: f64) {
        let center = Point::new(self.camera.viewport.x / 2.0, self.camera.viewport.y / 2.0);
        self.camera.zoom_at(center, factor);
    }

    pub fn reset_zoom(&mut self) {
        let center = Point::new(self.camera.viewport.x / 2.0, self.camera.viewport.y / 2.0);
        self.camera.set_zoom_at(center, 1.0);
    }

    /// Scroll-wheel input. `ctrl_or_cmd` switches scrolling to zooming.
    pub fn wheel(&mut self, screen: Point, dx: f64, dy: f64, ctrl_or_cmd: bool) {
        if ctrl_or_cmd {
            // Exponential so each notch is a constant ratio.
            let factor = (dy * 0.0035).exp();
            self.camera.zoom_at(screen, factor);
        } else {
            self.camera.pan_by_screen(dx, dy);
        }
    }

    // --------------------------------------------------------------- picking

    fn pick_at(&self, doc_point: Point) -> Option<String> {
        let slop = self.camera.screen_to_doc_scalar(LINE_PICK_SLOP_PX);
        dc_render::pick(self.page(), doc_point, slop, true)
    }

    /// The box enclosing the current selection, in document units.
    pub fn selection_bounds(&self) -> Option<BoundingBox> {
        let page = self.page();
        self.selection
            .iter()
            .filter_map(|id| page.shape(id))
            .map(|s| s.bounding_box.aabb())
            .reduce(|a, b| a.union(&b))
    }

    fn selection_bounds_screen(&self) -> Option<BoundingBox> {
        let doc = self.selection_bounds()?;
        let tl = self.camera.doc_to_screen(Point::new(doc.x, doc.y));
        let br = self.camera.doc_to_screen(Point::new(doc.right(), doc.bottom()));
        Some(BoundingBox::new(tl.x, tl.y, br.x - tl.x, br.y - tl.y))
    }

    /// The handle under a screen point, if the selection has handles there.
    pub fn handle_at(&self, screen: Point) -> Option<HandleKind> {
        let bounds = self.selection_bounds_screen()?;
        handles::hit_test(bounds, screen, 8.0, 4.0)
    }

    // ---------------------------------------------------------- pointer input

    pub fn pointer_down(
        &mut self,
        screen: Point,
        mods: Mods,
        middle_button: bool,
        click_count: u32,
        cx: &mut TextCtx<'_>,
    ) {
        let doc_point = self.camera.screen_to_doc(screen);
        self.press_screen = Some(screen);
        self.guides.clear();

        // While editing, clicks inside the text area drive the caret; a click
        // anywhere else finishes the edit before doing its normal job.
        if let Some(edit) = &mut self.text_edit {
            if edit.area().contains(kurbo::Point::new(doc_point.x, doc_point.y)) {
                if click_count >= 2 {
                    edit.select_word_at(doc_point, cx);
                } else {
                    edit.click(doc_point, mods.shift, cx);
                }
                self.drag = Some(Drag::TextSelect);
                return;
            }
            self.commit_text_edit();
        }

        // A double click on a shape or connector opens its label for editing.
        if click_count >= 2 && !middle_button && self.tool == Tool::Select {
            if let Some(id) = self.pick_at(doc_point) {
                if self.begin_edit_for(&id, true, cx) {
                    self.selection = vec![id];
                    return;
                }
            }
        }

        // A middle drag pans regardless of tool, matching the convention in
        // every other canvas editor.
        if middle_button || self.tool == Tool::Pan {
            self.drag = Some(Drag::Pan { last_screen: screen });
            return;
        }

        match self.tool.clone() {
            Tool::Pan => unreachable!("handled above"),

            Tool::Shape(shape_type) => {
                self.drag = Some(Drag::Create { shape_type, start: doc_point, current: doc_point });
            }

            Tool::Freehand => {
                self.drag = Some(Drag::Freehand { points: vec![doc_point] });
            }

            Tool::Connector => {
                let from_shape = self.pick_at(doc_point).filter(|id| {
                    self.page().shape(id).is_some()
                });
                let from = match &from_shape {
                    Some(id) => self
                        .page()
                        .shape(id)
                        .map(|s| s.bounding_box.center())
                        .unwrap_or(doc_point),
                    None => doc_point,
                };
                self.drag = Some(Drag::Connector { from_shape, from, current: doc_point });
            }

            Tool::Select => {
                // Handles take priority over the shapes beneath them.
                if let Some(handle) = self.handle_at(screen) {
                    self.start_handle_drag(handle, doc_point);
                    return;
                }

                match self.pick_at(doc_point) {
                    Some(id) => {
                        // A click on one member of a group takes the group.
                        let picked = self.group_members_of(&id);
                        let already = picked.iter().all(|i| self.selection.contains(i));
                        if mods.shift || mods.command {
                            if already {
                                self.selection.retain(|s| !picked.contains(s));
                            } else {
                                for member in &picked {
                                    if !self.selection.contains(member) {
                                        self.selection.push(member.clone());
                                    }
                                }
                            }
                        } else if !already {
                            self.selection = picked;
                        }

                        // Only shapes move; a connector follows its endpoints.
                        if self.selection.iter().any(|s| self.page().shape(s).is_some()) {
                            let originals = self.selected_shape_boxes();
                            self.drag = Some(Drag::Move { start: doc_point, originals });
                        }
                    }
                    None => {
                        if !mods.shift && !mods.command {
                            self.selection.clear();
                        }
                        self.drag = Some(Drag::Marquee { start: doc_point, current: doc_point });
                    }
                }
            }
        }
    }

    /// Settles the selection for a right click: a click on something already
    /// selected leaves a multi-selection alone, a click on something else
    /// takes just that shape's group, and a click on empty space clears the
    /// selection — so the context menu that follows always matches what it
    /// is about to act on.
    pub fn select_for_context_menu(&mut self, doc_point: Point) {
        match self.pick_at(doc_point) {
            Some(id) => {
                let picked = self.group_members_of(&id);
                let already = picked.iter().all(|i| self.selection.contains(i));
                if !already {
                    self.selection = picked;
                }
            }
            None => self.selection.clear(),
        }
    }

    fn start_handle_drag(&mut self, handle: HandleKind, doc_point: Point) {
        let Some(original_bounds) = self.selection_bounds() else {
            return;
        };

        if handle == HandleKind::Rotate {
            // Rotation applies to a single shape; a group rotation would need
            // to transform positions as well as angles.
            if let Some(id) = self
                .selection
                .iter()
                .find(|id| self.page().shape(id).is_some())
                .cloned()
            {
                let center = original_bounds.center();
                self.drag = Some(Drag::Rotate {
                    original_rotation: self
                        .page()
                        .shape(&id)
                        .map(|s| s.bounding_box.rotation_degrees())
                        .unwrap_or(0.0),
                    id,
                    center,
                    start_angle: angle_of(center, doc_point),
                });
            }
            return;
        }

        self.drag = Some(Drag::Resize {
            handle,
            start: doc_point,
            originals: self.selected_shape_boxes(),
            original_bounds,
        });
    }

    fn selected_shape_boxes(&self) -> Vec<(String, BoundingBox)> {
        let page = self.page();
        self.selection
            .iter()
            .filter_map(|id| page.shape(id).map(|s| (id.clone(), s.bounding_box)))
            .collect()
    }

    pub fn pointer_move(&mut self, screen: Point, mods: Mods, cx: &mut TextCtx<'_>) {
        let doc_point = self.camera.screen_to_doc(screen);

        if matches!(self.drag, Some(Drag::TextSelect)) {
            if let Some(edit) = &mut self.text_edit {
                edit.click(doc_point, true, cx);
            }
            return;
        }

        if self.drag.is_none() {
            self.hovered = self.pick_at(doc_point);
            return;
        }

        // Ignore sub-threshold jitter so a click does not nudge a shape.
        if let Some(press) = self.press_screen {
            if press.distance_to(screen) < DRAG_THRESHOLD_PX
                && matches!(self.drag, Some(Drag::Move { .. }))
            {
                return;
            }
        }

        let drag = self.drag.take();
        match drag {
            Some(Drag::Pan { last_screen }) => {
                self.camera
                    .pan_by_screen(screen.x - last_screen.x, screen.y - last_screen.y);
                self.drag = Some(Drag::Pan { last_screen: screen });
            }

            Some(Drag::Marquee { start, .. }) => {
                self.drag = Some(Drag::Marquee { start, current: doc_point });
            }

            Some(Drag::Create { shape_type, start, .. }) => {
                self.drag = Some(Drag::Create { shape_type, start, current: doc_point });
            }

            Some(Drag::Freehand { mut points }) => {
                points.push(doc_point);
                self.drag = Some(Drag::Freehand { points });
            }

            Some(Drag::Connector { from_shape, from, .. }) => {
                self.attach_target = self
                    .pick_at(doc_point)
                    .filter(|id| self.page().shape(id).is_some())
                    .filter(|id| Some(id) != from_shape.as_ref());
                self.drag = Some(Drag::Connector { from_shape, from, current: doc_point });
            }

            Some(Drag::Move { start, originals }) => {
                self.apply_move(start, &originals, doc_point, mods);
                self.drag = Some(Drag::Move { start, originals });
            }

            Some(Drag::Resize { handle, start, originals, original_bounds }) => {
                self.apply_resize(handle, start, &originals, original_bounds, doc_point, mods);
                self.drag = Some(Drag::Resize { handle, start, originals, original_bounds });
            }

            Some(Drag::Rotate { id, center, start_angle, original_rotation }) => {
                self.apply_rotate(&id, center, start_angle, original_rotation, doc_point, mods);
                self.drag = Some(Drag::Rotate { id, center, start_angle, original_rotation });
            }

            // Handled by the early return above.
            Some(Drag::TextSelect) | None => {}
        }
    }

    fn apply_move(
        &mut self,
        start: Point,
        originals: &[(String, BoundingBox)],
        doc_point: Point,
        mods: Mods,
    ) {
        let mut dx = doc_point.x - start.x;
        let mut dy = doc_point.y - start.y;

        // Shift locks the drag to the dominant axis.
        if mods.shift {
            if dx.abs() >= dy.abs() {
                dy = 0.0;
            } else {
                dx = 0.0;
            }
        }

        // Holding command bypasses snapping entirely, whatever is switched on.
        self.guides.clear();
        if !mods.command {
            if let Some((_, first)) = originals.first() {
                // Aligning to other shapes comes first, because it is the
                // more specific of the two: a shape that lines up with its
                // neighbour should stay lined up with it rather than being
                // pulled onto the nearest grid line.
                let mut aligned = false;
                if self.snap_enabled {
                    let moved = first.translated(dx, dy);
                    let others = self.other_shape_boxes(originals);
                    let threshold = self.camera.screen_to_doc_scalar(self.snap_threshold_px);
                    let result = snap::snap_box(moved, &others, threshold);

                    if result.is_snapped() {
                        dx += result.dx;
                        dy += result.dy;
                        self.guides = result.guides;
                        aligned = true;
                    }
                }

                // Nothing to align with, so fall back to the grid. Without
                // this, dragging on an empty canvas leaves shapes on
                // fractional coordinates that never line up later.
                if !aligned && self.snap_to_grid {
                    let pitch = self.grid_spacing;
                    dx = snap::snap_to_grid(first.x + dx, pitch) - first.x;
                    dy = snap::snap_to_grid(first.y + dy, pitch) - first.y;
                }
            }
        }

        // The whole gesture is one undo step, recorded on the first movement.
        self.record_once_for_drag();
        for (id, original) in originals {
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = original.translated(dx, dy);
            }
        }
    }

    fn other_shape_boxes(&self, excluding: &[(String, BoundingBox)]) -> Vec<BoundingBox> {
        self.page()
            .shapes()
            .iter()
            .filter(|s| !excluding.iter().any(|(id, _)| id == &s.id))
            .map(|s| s.bounding_box.aabb())
            .collect()
    }

    fn apply_resize(
        &mut self,
        handle: HandleKind,
        start: Point,
        originals: &[(String, BoundingBox)],
        original_bounds: BoundingBox,
        doc_point: Point,
        mods: Mods,
    ) {
        let dx = doc_point.x - start.x;
        let dy = doc_point.y - start.y;
        let new_bounds = handles::resize(original_bounds, handle, dx, dy, mods.shift);

        self.record_once_for_drag();

        // Each shape keeps its relative place and proportion inside the box.
        let sx = if original_bounds.w > 0.0 { new_bounds.w / original_bounds.w } else { 1.0 };
        let sy = if original_bounds.h > 0.0 { new_bounds.h / original_bounds.h } else { 1.0 };

        for (id, original) in originals {
            let rel_x = original.x - original_bounds.x;
            let rel_y = original.y - original_bounds.y;
            let updated = BoundingBox {
                x: new_bounds.x + rel_x * sx,
                y: new_bounds.y + rel_y * sy,
                w: (original.w * sx).max(1.0),
                h: (original.h * sy).max(1.0),
                rotation: original.rotation,
            };
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = updated;
            }
        }
    }

    fn apply_rotate(
        &mut self,
        id: &str,
        center: Point,
        start_angle: f64,
        original_rotation: f64,
        doc_point: Point,
        mods: Mods,
    ) {
        let delta = angle_of(center, doc_point) - start_angle;
        let mut rotation = (original_rotation + delta).rem_euclid(360.0);
        // Shift steps through the common angles.
        if mods.shift {
            rotation = (rotation / 15.0).round() * 15.0;
        }

        self.record_once_for_drag();
        if let Some(shape) = self.page_mut().shape_mut(id) {
            shape.bounding_box = shape.bounding_box.with_rotation(rotation);
        }
    }

    /// Records one history entry for the current gesture.
    ///
    /// The flag lives in `press_screen`: taking it means the first mutation of
    /// this drag has happened and later moves must not stack undo steps.
    fn record_once_for_drag(&mut self) {
        if self.press_screen.take().is_some() {
            self.begin_change();
        }
    }

    pub fn pointer_up(&mut self, screen: Point, _mods: Mods, _cx: &mut TextCtx<'_>) {
        let doc_point = self.camera.screen_to_doc(screen);
        let drag = self.drag.take();
        self.guides.clear();
        self.attach_target = None;
        let was_click = self
            .press_screen
            .is_some_and(|p| p.distance_to(screen) < DRAG_THRESHOLD_PX);
        self.press_screen = None;

        if matches!(drag, Some(Drag::TextSelect)) {
            return;
        }

        match drag {
            Some(Drag::Marquee { start, current }) => {
                let region = BoundingBox::from_corners(start, current);
                if region.w > 1.0 || region.h > 1.0 {
                    let hits = dc_render::pick_in_region(self.page(), region);
                    for id in hits {
                        if !self.selection.contains(&id) {
                            self.selection.push(id);
                        }
                    }
                }
            }

            Some(Drag::Create { shape_type, start, current }) => {
                let box_ = if was_click {
                    // A plain click places the shape at its default size,
                    // centered on the click.
                    let (w, h) = shape_type.default_size();
                    BoundingBox::new(start.x - w / 2.0, start.y - h / 2.0, w, h)
                } else {
                    BoundingBox::from_corners(start, current)
                };
                self.create_shape(shape_type, box_);
                self.tool = Tool::Select;
            }

            Some(Drag::Freehand { points }) => {
                // Fewer than two points, or a stroke that never moved, is a
                // stray click rather than a drawing — nothing is created,
                // so the tool stays armed for another attempt.
                let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
                let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
                for p in &points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                if points.len() >= 2 && (max_x - min_x).max(max_y - min_y) > 1.0 {
                    let id = self.create_shape(
                        ShapeType::FreehandDrawing,
                        BoundingBox::new(min_x, min_y, max_x - min_x, max_y - min_y),
                    );
                    if let Some(shape) = self.page_mut().shape_mut(&id) {
                        shape.points = Some(points);
                    }
                }
            }

            Some(Drag::Connector { from_shape, from, .. }) => {
                let to_shape = self
                    .pick_at(doc_point)
                    .filter(|id| self.page().shape(id).is_some())
                    .filter(|id| Some(id) != from_shape.as_ref());
                self.create_connector(from_shape, from, to_shape, doc_point);
            }

            _ => {}
        }
    }

    // --------------------------------------------------------- text editing

    pub fn is_editing(&self) -> bool {
        self.text_edit.is_some()
    }

    /// The id of the item whose label is being edited, so the renderer can
    /// suppress its static text and draw the live one instead.
    pub fn editing_id(&self) -> Option<&str> {
        match self.text_edit.as_ref()?.target {
            EditTarget::Shape(ref id) => Some(id),
            EditTarget::LineLabel { ref line_id, .. } => Some(line_id),
            EditTarget::PageTitle(_) => None,
        }
    }

    /// The text area of a shape, matching exactly what the renderer uses so
    /// the caret lands where the glyphs are.
    fn shape_text_area(&self, shape_id: &str) -> Option<kurbo::Rect> {
        let shape = self.page().shape(shape_id)?;
        let geo = dc_geom::geometry(shape);
        let bb = shape.bounding_box;
        let i = geo.text_insets;
        let area = kurbo::Rect::new(
            bb.x + i.x0,
            bb.y + i.y0,
            bb.right() - i.x1,
            bb.bottom() - i.y1,
        );
        (area.width() > 1.0 && area.height() > 0.0).then_some(area)
    }

    /// The area around a connector label.
    fn line_label_area(&self, line_id: &str, index: usize) -> Option<kurbo::Rect> {
        let line = self.page().line(line_id)?;
        let route = dc_geom::route_line(line, self.page())?;
        let label = line.labels().get(index)?;
        let anchor = route.point_at_fraction(label.resolved_position());
        // A generous box so wrapping does not fight the caret.
        let half_width = 90.0;
        let height = 24.0;
        Some(kurbo::Rect::new(
            anchor.x - half_width,
            anchor.y - height / 2.0,
            anchor.x + half_width,
            anchor.y + height / 2.0,
        ))
    }

    /// Opens the label of a shape or connector for editing.
    ///
    /// A connector with no label gets one, so there is always something to
    /// type into.
    fn begin_edit_for(&mut self, id: &str, select_all: bool, cx: &mut TextCtx<'_>) -> bool {
        if let Some(shape) = self.page().shape(id) {
            if !shape.shape_type.accepts_text() {
                return false;
            }
            let Some(area) = self.shape_text_area(id) else {
                return false;
            };
            let existing = shape.text_str().to_string();
            let base = self.base_text_style(Some(id));
            let backdrop = self.surface_under_shape(id);
            self.text_edit = Some(TextEdit::begin(
                EditTarget::Shape(id.to_string()),
                &existing,
                &base,
                backdrop,
                area,
                select_all,
                cx,
            ));
            return true;
        }

        if self.page().line(id).is_some() {
            // Give the connector a label to edit if it has none.
            if self.page().line(id).is_some_and(|l| l.labels().is_empty()) {
                self.begin_change();
                if let Some(line) = self.page_mut().line_mut(id) {
                    line.text = Some(vec![dc_model::LineLabel::new("")]);
                }
            }
            let Some(area) = self.line_label_area(id, 0) else {
                return false;
            };
            let existing = self
                .page()
                .line(id)
                .and_then(|l| l.labels().first())
                .map(|l| l.text.clone())
                .unwrap_or_default();
            let base = dc_model::TextStyle {
                color: self
                    .page()
                    .line(id)
                    .map(|l| l.resolved_stroke().resolved_color())
                    .unwrap_or(Color::BLACK),
                ..Default::default()
            };
            // A connector label sits on the page, not on any shape.
            let backdrop = self.page_fill();
            self.text_edit = Some(TextEdit::begin(
                EditTarget::LineLabel { line_id: id.to_string(), index: 0 },
                &existing,
                &base,
                backdrop,
                area,
                select_all,
                cx,
            ));
            return true;
        }

        false
    }

    /// The page's own background color.
    pub fn page_fill(&self) -> Color {
        self.page().resolved_settings().resolved_fill()
    }

    /// The surface a shape's label is painted over.
    ///
    /// Normally the shape's own fill, which is what its text colour was
    /// derived from. A shape with no flat fill — an image, or one drawn with
    /// no fill at all — leaves the page showing through instead.
    fn surface_under_shape(&self, shape_id: &str) -> Color {
        let page = self.page_fill();
        let Some(shape) = self.page().shape(shape_id) else {
            return page;
        };
        if !shape.shape_type.accepts_style() {
            return page;
        }
        match shape.resolved_style().resolved_fill().color() {
            Some(color) if color.a > 0 => color,
            _ => page,
        }
    }

    /// The default text style for an item, which supplies the contrast-derived
    /// text color the format specifies.
    fn base_text_style(&self, shape_id: Option<&str>) -> dc_model::TextStyle {
        let mut base = dc_model::TextStyle::default();
        if let Some(shape) = shape_id.and_then(|id| self.page().shape(id)) {
            base.color = shape.resolved_style().resolved_text_color();
        }
        base
    }

    /// Opens the selected item's label, which is what Enter and F2 do.
    pub fn begin_text_edit_on_selection(&mut self, cx: &mut TextCtx<'_>) -> bool {
        let Some(id) = self.selection.first().cloned() else {
            return false;
        };
        self.begin_edit_for(&id, true, cx)
    }

    /// Finishes editing and writes the result into the document.
    pub fn commit_text_edit(&mut self) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        if !edit.is_modified() {
            return;
        }

        let markup = edit.commit();
        self.begin_change();
        match edit.target {
            EditTarget::Shape(id) => {
                if let Some(shape) = self.page_mut().shape_mut(&id) {
                    shape.text = if markup.is_empty() {
                        None
                    } else {
                        Some(Text::new(markup))
                    };
                }
            }
            EditTarget::LineLabel { line_id, index } => {
                if let Some(line) = self.page_mut().line_mut(&line_id) {
                    let labels = line.text.get_or_insert_with(Vec::new);
                    if markup.is_empty() {
                        if index < labels.len() {
                            labels.remove(index);
                        }
                        if labels.is_empty() {
                            line.text = None;
                        }
                    } else if let Some(label) = labels.get_mut(index) {
                        label.text = markup;
                    } else {
                        labels.push(dc_model::LineLabel::new(markup));
                    }
                }
            }
            EditTarget::PageTitle(index) => {
                if let Some(page) = self.package.document.pages.get_mut(index) {
                    if !markup.is_empty() {
                        page.title = markup;
                    }
                }
            }
        }
        self.status = "Label updated".into();
    }

    /// Abandons editing, leaving the document untouched.
    pub fn cancel_text_edit(&mut self) {
        self.text_edit = None;
        self.drag = None;
    }

    /// Routes a command to the active editor. Returns whether it was handled.
    pub fn text_command(&mut self, command: TextCommand, cx: &mut TextCtx<'_>) -> bool {
        match &mut self.text_edit {
            Some(edit) => {
                edit.command(command, cx);
                true
            }
            None => false,
        }
    }

    /// Sets one shape's text directly, bypassing the selection. Used by tests
    /// to establish a starting label without going through an edit session.
    #[cfg(test)]
    fn set_selection_text_for_test(&mut self, id: &str, text: &str) {
        if let Some(shape) = self.page_mut().shape_mut(id) {
            shape.text = Some(Text::new(text));
        }
    }

    /// Re-reads the text area from the item being edited.
    ///
    /// The shape can move, resize, or rotate while its label is open, and a
    /// stale area would leave the caret hit-testing against where the shape
    /// used to be.
    pub fn refresh_text_area(&mut self) {
        let Some(target) = self.text_edit.as_ref().map(|e| e.target.clone()) else {
            return;
        };
        let area = match &target {
            EditTarget::Shape(id) => self.shape_text_area(id),
            EditTarget::LineLabel { line_id, index } => self.line_label_area(line_id, *index),
            // A page title lives in the tab bar, which does not move.
            EditTarget::PageTitle(_) => None,
        };
        if let (Some(area), Some(edit)) = (area, self.text_edit.as_mut()) {
            if edit.area() != area {
                edit.set_area(area);
            }
        }
    }

    /// Advances the caret blink.
    pub fn tick(&mut self, delta_seconds: f64) {
        if let Some(edit) = &mut self.text_edit {
            edit.tick(delta_seconds);
        }
    }

    // -------------------------------------------------------------- commands

    pub fn create_shape(&mut self, shape_type: ShapeType, bounds: BoundingBox) -> String {
        let id = self.package.document.fresh_id("shape");
        let mut shape = Shape::new(&id, shape_type.clone(), bounds);
        if shape_type.accepts_style() {
            shape.style = Some(Style {
                fill: Some(Fill::solid(Color::WHITE)),
                stroke: Some(Stroke::new(Color::rgb(0x33, 0x3a, 0x45), 1.0)),
                ..Default::default()
            });
        }

        self.begin_change();
        self.page_mut().add_shape(shape);
        self.selection = vec![id.clone()];
        self.status = format!("Added {}", shape_type.display_name());
        id
    }

    fn create_connector(
        &mut self,
        from_shape: Option<String>,
        from: Point,
        to_shape: Option<String>,
        to: Point,
    ) {
        // A connector anchored to nothing at both ends is almost always an
        // accidental drag on empty canvas.
        if from_shape.is_none() && to_shape.is_none() {
            return;
        }
        if from.distance_to(to) < 2.0 {
            return;
        }

        let id = self.package.document.fresh_id("line");
        let start = match from_shape {
            Some(s) => Endpoint::at_shape(s, EndpointStyle::None),
            None => Endpoint::at_point(from, EndpointStyle::None),
        };
        let end = match to_shape {
            Some(s) => Endpoint::at_shape(s, EndpointStyle::Arrow),
            None => Endpoint::at_point(to, EndpointStyle::Arrow),
        };

        let routing = self.default_line_type;
        self.begin_change();
        self.page_mut()
            .add_line(Line::new(&id, start, end).with_line_type(routing));
        self.selection = vec![id];
        self.tool = Tool::Select;
        self.status = "Added connector".into();
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        self.begin_change();
        let ids = std::mem::take(&mut self.selection);
        let page = self.page_mut();
        for id in &ids {
            if !page.remove_shape(id) {
                page.remove_line(id);
            }
        }
        self.status = format!("Deleted {} item(s)", ids.len());
    }

    pub fn select_all(&mut self) {
        let page = self.page();
        self.selection = page
            .shapes()
            .iter()
            .map(|s| s.id.clone())
            .chain(page.lines().iter().map(|l| l.id.clone()))
            .collect();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Selects every shape on the page, leaving connectors alone.
    pub fn select_all_shapes(&mut self) {
        self.selection = self.page().shapes().iter().map(|s| s.id.clone()).collect();
        self.status = format!("Selected {} shape(s)", self.selection.len());
    }

    /// Selects every connector on the page, leaving shapes alone.
    pub fn select_all_lines(&mut self) {
        self.selection = self.page().lines().iter().map(|l| l.id.clone()).collect();
        self.status = format!("Selected {} connector(s)", self.selection.len());
    }

    /// Extends the selection to everything matching it in some property.
    ///
    /// This is what "select all with the same fill" does: it reads the property
    /// from what is already selected and gathers everything that agrees.
    pub fn select_matching(&mut self, criterion: SelectBy) {
        if self.selection.is_empty() {
            return;
        }
        let page = self.page();
        let wanted: Vec<MatchKey> = self
            .selection
            .iter()
            .filter_map(|id| criterion.key_of(page, id))
            .collect();
        if wanted.is_empty() {
            self.status = format!("The selection has no {} to match", criterion.name());
            return;
        }

        let mut found: Vec<String> = Vec::new();
        for shape in page.shapes() {
            if criterion
                .key_of(page, &shape.id)
                .is_some_and(|k| wanted.contains(&k))
            {
                found.push(shape.id.clone());
            }
        }
        for line in page.lines() {
            if criterion
                .key_of(page, &line.id)
                .is_some_and(|k| wanted.contains(&k))
            {
                found.push(line.id.clone());
            }
        }

        self.status = format!(
            "Selected {} item(s) with the same {}",
            found.len(),
            criterion.name()
        );
        self.selection = found;
    }

    // ---------------------------------------------------------- arranging

    /// Lines the selection up along one edge or axis.
    ///
    /// Everything aligns to the enclosing box of the selection, which is what
    /// makes aligning twice a no-op rather than a slow drift.
    pub fn align_selection(&mut self, how: Align) {
        let boxes = self.selected_shape_boxes();
        if boxes.len() < 2 {
            return;
        }
        let Some(bounds) = self.selection_bounds() else {
            return;
        };

        self.begin_change();
        for (id, original) in &boxes {
            let aabb = original.aabb();
            let (dx, dy) = match how {
                Align::Left => (bounds.x - aabb.x, 0.0),
                Align::CenterHorizontally => {
                    (bounds.center().x - aabb.center().x, 0.0)
                }
                Align::Right => (bounds.right() - aabb.right(), 0.0),
                Align::Top => (0.0, bounds.y - aabb.y),
                Align::Middle => (0.0, bounds.center().y - aabb.center().y),
                Align::Bottom => (0.0, bounds.bottom() - aabb.bottom()),
            };
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = original.translated(dx, dy);
            }
        }
        self.status = format!("Aligned {}", how.name());
    }

    /// Spreads the selection out so the gaps between its items are equal.
    ///
    /// The two outermost items stay put and define the span, which is the
    /// behavior that makes the command predictable.
    pub fn distribute_selection(&mut self, horizontal: bool) {
        let mut boxes = self.selected_shape_boxes();
        if boxes.len() < 3 {
            // Two items are already evenly spaced, and one has nothing to
            // space against.
            self.status = "Distributing needs three or more shapes".into();
            return;
        }

        boxes.sort_by(|(_, a), (_, b)| {
            let (a, b) = (a.aabb(), b.aabb());
            let (pa, pb) = if horizontal {
                (a.center().x, b.center().x)
            } else {
                (a.center().y, b.center().y)
            };
            pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
        });

        let first = boxes.first().map(|(_, b)| b.aabb());
        let last = boxes.last().map(|(_, b)| b.aabb());
        let (Some(first), Some(last)) = (first, last) else {
            return;
        };

        // Space the gaps between the items, not their centres, so items of
        // different sizes end up visually even.
        let occupied: f64 = boxes
            .iter()
            .map(|(_, b)| if horizontal { b.aabb().w } else { b.aabb().h })
            .sum();
        let span = if horizontal {
            last.right() - first.x
        } else {
            last.bottom() - first.y
        };
        let gap = (span - occupied) / (boxes.len() - 1) as f64;

        self.begin_change();
        let mut cursor = if horizontal { first.x } else { first.y };
        for (id, original) in &boxes {
            let aabb = original.aabb();
            let (dx, dy) = if horizontal {
                (cursor - aabb.x, 0.0)
            } else {
                (0.0, cursor - aabb.y)
            };
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = original.translated(dx, dy);
            }
            cursor += (if horizontal { aabb.w } else { aabb.h }) + gap;
        }

        self.status = format!(
            "Distributed {}",
            if horizontal { "horizontally" } else { "vertically" }
        );
    }

    /// Resizes the selection to match the first shape in it.
    ///
    /// The first selected shape is the reference, which is the convention that
    /// lets the user choose what to match by choosing what to click first.
    pub fn match_size(&mut self, width: bool, height: bool) {
        let boxes = self.selected_shape_boxes();
        if boxes.len() < 2 || (!width && !height) {
            return;
        }
        let reference = boxes[0].1;

        self.begin_change();
        for (id, original) in boxes.iter().skip(1) {
            let updated = BoundingBox {
                w: if width { reference.w } else { original.w },
                h: if height { reference.h } else { original.h },
                ..*original
            };
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = updated;
            }
        }

        self.status = match (width, height) {
            (true, true) => "Matched size".into(),
            (true, false) => "Matched width".into(),
            _ => "Matched height".to_string(),
        };
    }

    /// Collects the selection into a group, so that selecting any member
    /// selects all of them and they move together.
    pub fn group_selection(&mut self) {
        if self.selection.len() < 2 {
            self.status = "Select two or more items to group".into();
            return;
        }
        let members = self.selection.clone();
        let id = self.package.document.fresh_id("group");

        self.begin_change();
        // A member can only belong to one group, so an item being regrouped
        // leaves whatever group it was in.
        let page = self.page_mut();
        for group in page.groups_mut().iter_mut() {
            if let Some(items) = group.items.as_mut() {
                items.retain(|i| !members.contains(i));
            }
        }
        page.groups_mut().retain(|g| g.item_ids().len() > 1);
        page.groups_mut().push(dc_model::Group::new(id, members.clone()));

        self.status = format!("Grouped {} items", members.len());
    }

    /// Breaks apart every group the selection touches.
    pub fn ungroup_selection(&mut self) {
        let selected = self.selection.clone();
        if selected.is_empty() {
            return;
        }
        let touched: Vec<String> = self
            .page()
            .groups()
            .iter()
            .filter(|g| {
                selected.contains(&g.id) || g.item_ids().iter().any(|i| selected.contains(i))
            })
            .map(|g| g.id.clone())
            .collect();
        if touched.is_empty() {
            return;
        }

        self.begin_change();
        self.page_mut().groups_mut().retain(|g| !touched.contains(&g.id));
        self.status = format!("Ungrouped {} group(s)", touched.len());
    }

    /// Whether anything selected belongs to a group.
    pub fn selection_is_grouped(&self) -> bool {
        let page = self.page();
        self.selection.iter().any(|id| {
            page.groups()
                .iter()
                .any(|g| g.item_ids().iter().any(|i| i == id))
        })
    }

    /// The members of the group an item belongs to, or just the item itself.
    ///
    /// Clicking one member of a group selects the whole group, which is what
    /// makes a group feel like one object.
    fn group_members_of(&self, id: &str) -> Vec<String> {
        let page = self.page();
        for group in page.groups() {
            if group.item_ids().iter().any(|i| i == id) {
                let members: Vec<String> = group
                    .item_ids()
                    .iter()
                    .filter(|i| page.shape(i).is_some() || page.line(i).is_some())
                    .cloned()
                    .collect();
                if !members.is_empty() {
                    return members;
                }
            }
        }
        vec![id.to_string()]
    }

    /// The current selection as a clipboard fragment.
    ///
    /// Connectors are included only when both of their endpoints are also in
    /// the selection, since a connector to a shape that was not copied would
    /// paste as a dangling reference.
    pub fn copy_fragment(&self) -> crate::clipboard::Fragment {
        let page = self.page();
        let shapes: Vec<Shape> = self
            .selection
            .iter()
            .filter_map(|id| page.shape(id).cloned())
            .collect();
        let copied: Vec<&str> = shapes.iter().map(|s| s.id.as_str()).collect();

        let lines: Vec<Line> = page
            .lines()
            .iter()
            .filter(|l| {
                let inside = |e: &Endpoint| {
                    e.shape_id().is_some_and(|id| copied.contains(&id))
                };
                // Either explicitly selected, or implied by both its ends.
                (self.selection.contains(&l.id) || inside(&l.endpoint1))
                    && inside(&l.endpoint1)
                    && inside(&l.endpoint2)
            })
            .cloned()
            .collect();

        crate::clipboard::Fragment::new(shapes, lines)
    }

    /// Inserts a fragment, offset so the copy is visible, and selects it.
    ///
    /// Every identifier is reassigned and connector endpoints are rewritten to
    /// match, so pasting the same fragment repeatedly is safe.
    pub fn paste_fragment(&mut self, fragment: &crate::clipboard::Fragment, offset: f64) {
        if fragment.is_empty() {
            return;
        }
        self.begin_change();

        let mut remap: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut new_selection = Vec::new();

        for shape in &fragment.shapes {
            let id = self.package.document.fresh_id("shape");
            remap.insert(shape.id.clone(), id.clone());

            let mut copy = shape.clone();
            copy.id = id.clone();
            copy.bounding_box = copy.bounding_box.translated(offset, offset);
            self.page_mut().add_shape(copy);
            new_selection.push(id);
        }

        for line in &fragment.lines {
            let id = self.package.document.fresh_id("line");
            let mut copy = line.clone();
            copy.id = id.clone();

            // Point the copy at the pasted shapes, not the originals.
            for endpoint in [&mut copy.endpoint1, &mut copy.endpoint2] {
                if let Endpoint::ShapeEndpoint { shape_id, .. } = endpoint {
                    if let Some(mapped) = remap.get(shape_id.as_str()) {
                        *shape_id = mapped.clone();
                    }
                }
            }
            if let Some(joints) = copy.joints.as_mut() {
                for joint in joints {
                    *joint = Point::new(joint.x + offset, joint.y + offset);
                }
            }
            self.page_mut().add_line(copy);
            new_selection.push(id);
        }

        self.selection = new_selection;
        self.status = format!(
            "Pasted {} shape(s), {} connector(s)",
            fragment.shapes.len(),
            fragment.lines.len()
        );
    }

    /// Copies the selected shapes, offset slightly, and selects the copies.
    pub fn duplicate_selection(&mut self) {
        let originals: Vec<Shape> = {
            let page = self.page();
            self.selection
                .iter()
                .filter_map(|id| page.shape(id).cloned())
                .collect()
        };
        if originals.is_empty() {
            return;
        }

        self.begin_change();
        let offset = self.grid_spacing.max(4.0);
        let mut new_ids = Vec::with_capacity(originals.len());
        for mut shape in originals {
            let id = self.package.document.fresh_id("shape");
            shape.id = id.clone();
            shape.bounding_box = shape.bounding_box.translated(offset, offset);
            self.page_mut().add_shape(shape);
            new_ids.push(id);
        }
        self.selection = new_ids;
        self.status = "Duplicated selection".into();
    }

    /// Moves the selection by a document-space delta.
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        if self.selection.is_empty() {
            return;
        }
        let ids: Vec<String> = self.selection.clone();
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                shape.bounding_box = shape.bounding_box.translated(dx, dy);
            }
        }
    }

    /// Sends the selection to the front or back of the z-order.
    pub fn reorder_selection(&mut self, to_front: bool) {
        if self.selection.is_empty() {
            return;
        }
        let page = self.page();
        let extreme = page
            .shapes()
            .iter()
            .map(|s| s.resolved_z_index())
            .chain(page.lines().iter().map(|l| l.resolved_z_index()))
            .fold(0i64, |acc, z| if to_front { acc.max(z) } else { acc.min(z) });
        let target = if to_front { extreme + 1 } else { extreme - 1 };

        let ids: Vec<String> = self.selection.clone();
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                shape.z_index = Some(target);
            } else if let Some(line) = self.page_mut().line_mut(&id) {
                line.z_index = Some(target);
            }
        }
    }

    pub fn set_selection_fill(&mut self, color: Color) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if !shape.shape_type.accepts_style() {
                    continue;
                }
                shape.style.get_or_insert_with(Default::default).fill = Some(Fill::solid(color));
            }
        }
    }

    pub fn set_selection_stroke(&mut self, color: Color, width: Option<f64>) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if !shape.shape_type.accepts_style() {
                    continue;
                }
                let style = shape.style.get_or_insert_with(Default::default);
                let mut stroke = style.stroke.unwrap_or_default();
                stroke.color = Some(color);
                if let Some(w) = width {
                    stroke.width = Some(w);
                }
                style.stroke = Some(stroke);
            } else if let Some(line) = self.page_mut().line_mut(&id) {
                let mut stroke = line.stroke.unwrap_or_default();
                stroke.color = Some(color);
                if let Some(w) = width {
                    stroke.width = Some(w);
                }
                line.stroke = Some(stroke);
            }
        }
    }

    // --------------------------------------------------------- endpoints

    /// Sets the terminator on one end of the selected connectors.
    ///
    /// Which end is "first" is the end the connector was drawn from, which is
    /// what the two pickers in the panel label as left and right.
    pub fn set_endpoint_style(&mut self, end: LineEnd, style: EndpointStyle) {
        let ids: Vec<String> = self.selection.clone();
        if !ids.iter().any(|id| self.page().line(id).is_some()) {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(line) = self.page_mut().line_mut(&id) {
                match end {
                    LineEnd::Start => line.endpoint1.set_style(style.clone()),
                    LineEnd::End => line.endpoint2.set_style(style.clone()),
                }
            }
        }
        self.status = format!("Set {} endpoint", end.name());
    }

    /// Turns the selected connectors around.
    ///
    /// Both ends swap, terminators included, so an arrow that pointed one way
    /// points the other. This is the double-arrow button beside the pickers.
    pub fn swap_endpoints(&mut self) {
        let ids: Vec<String> = self.selection.clone();
        if !ids.iter().any(|id| self.page().line(id).is_some()) {
            return;
        }
        self.begin_change();
        let mut swapped = 0;
        for id in ids {
            if let Some(line) = self.page_mut().line_mut(&id) {
                std::mem::swap(&mut line.endpoint1, &mut line.endpoint2);
                // The joints were recorded from the old start, so they run
                // backwards now too.
                if let Some(joints) = line.joints.as_mut() {
                    joints.reverse();
                }
                swapped += 1;
            }
        }
        self.status = format!("Reversed {swapped} connector(s)");
    }

    /// The terminator on one end of the selection, when they agree.
    pub fn selection_endpoint_style(&self, end: LineEnd) -> Option<EndpointStyle> {
        let page = self.page();
        let mut agreed: Option<EndpointStyle> = None;
        for id in &self.selection {
            let Some(line) = page.line(id) else { continue };
            let style = match end {
                LineEnd::Start => line.endpoint1.style(),
                LineEnd::End => line.endpoint2.style(),
            };
            match &agreed {
                Some(existing) if existing != style => return None,
                Some(_) => {}
                None => agreed = Some(style.clone()),
            }
        }
        agreed
    }

    /// Whether anything selected is a connector.
    pub fn selection_has_lines(&self) -> bool {
        let page = self.page();
        self.selection.iter().any(|id| page.line(id).is_some())
    }

    /// Whether anything selected is a shape.
    pub fn selection_has_shapes(&self) -> bool {
        let page = self.page();
        self.selection.iter().any(|id| page.shape(id).is_some())
    }

    // ------------------------------------------------------ shape geometry

    /// Sets how transparent the selected shapes are, as a percentage.
    pub fn set_opacity(&mut self, percent: f64) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        let percent = percent.clamp(0.0, 100.0);
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                // The format's own default is fully opaque, so that is stored
                // as no value rather than as a hundred.
                shape.opacity = (percent < 100.0).then_some(percent);
            }
        }
        self.status = format!("Opacity {percent:.0}%");
    }

    /// The opacity shared by the selection, as a percentage.
    pub fn selection_opacity(&self) -> Option<f64> {
        self.agreed(|page, id| Some((page.shape(id)?.opacity_factor() * 100.0).to_bits()))
            .map(f64::from_bits)
    }

    /// Sets the corner radius of the selected shapes, in points.
    pub fn set_corner_rounding(&mut self, radius: f64) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if !shape.shape_type.accepts_style() {
                    continue;
                }
                let style = shape.style.get_or_insert_with(Default::default);
                // The format stores double the pixel value.
                style.rounding = (radius > 0.0).then_some(radius * 2.0);
            }
        }
        self.status = if radius > 0.0 {
            format!("Corner radius {radius:.0}")
        } else {
            "Square corners".into()
        };
    }

    /// The corner radius shared by the selection, in points.
    pub fn selection_corner_rounding(&self) -> Option<f64> {
        self.agreed(|page, id| {
            let shape = page.shape(id)?;
            shape.shape_type.accepts_style().then_some(())?;
            Some(shape.resolved_style().corner_radius_px().to_bits())
        })
        .map(f64::from_bits)
    }

    /// Turns the selected shapes by a quarter, or sets them square again.
    pub fn rotate_selection(&mut self, degrees: f64) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                let turned = (shape.bounding_box.rotation_degrees() + degrees).rem_euclid(360.0);
                shape.bounding_box = shape.bounding_box.with_rotation(turned);
            }
        }
        self.status = format!("Rotated {degrees:.0}°");
    }

    /// Mirrors the selection about the centre of its own bounding box.
    ///
    /// The shapes keep their own outlines — the format has no per-shape mirror
    /// flag — so what moves is their arrangement. Flipping twice returns the
    /// selection to where it started.
    pub fn flip_selection(&mut self, horizontal: bool) {
        let boxes = self.selected_shape_boxes();
        if boxes.is_empty() {
            return;
        }
        let Some(bounds) = self.selection_bounds() else {
            return;
        };

        self.begin_change();
        for (id, original) in &boxes {
            let aabb = original.aabb();
            let updated = if horizontal {
                let x = bounds.x + bounds.right() - aabb.right();
                BoundingBox { x, ..*original }
            } else {
                let y = bounds.y + bounds.bottom() - aabb.bottom();
                BoundingBox { y, ..*original }
            };
            if let Some(shape) = self.page_mut().shape_mut(id) {
                shape.bounding_box = updated;
            }
        }
        self.status = format!(
            "Flipped {}",
            if horizontal { "horizontally" } else { "vertically" }
        );
    }

    /// Moves the selection one step through the z-order.
    ///
    /// Lucidchart offers all four of front, forward, backward, and back; this
    /// is the pair that move by one rather than all the way.
    pub fn nudge_z_order(&mut self, forward: bool) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            let step = if forward { 1 } else { -1 };
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                shape.z_index = Some(shape.resolved_z_index() + step);
            } else if let Some(line) = self.page_mut().line_mut(&id) {
                line.z_index = Some(line.resolved_z_index() + step);
            }
        }
        self.status = if forward { "Brought forward".into() } else { "Sent backward".to_string() };
    }

    /// Replaces the type of the selected shapes, keeping everything else.
    ///
    /// This is Lucidchart's "change shape": the box, the style, and the text
    /// stay put and only the outline changes.
    pub fn change_shape_type(&mut self, shape_type: ShapeType) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                // A type that rejects text would otherwise carry a label the
                // format does not allow it to have.
                if !shape_type.accepts_text() {
                    shape.text = None;
                }
                if !shape_type.accepts_style() {
                    shape.style = None;
                }
                shape.shape_type = shape_type.clone();
            }
        }
        self.status = format!("Changed to {}", shape_type.display_name());
    }

    // ------------------------------------------------------- paint format

    /// Remembers the style of the first selected item, for painting onto
    /// others. This is Lucidchart's paint format.
    pub fn copy_style(&mut self) -> bool {
        let page = self.page();
        let Some(id) = self.selection.first() else {
            return false;
        };
        if let Some(shape) = page.shape(id) {
            self.copied_style = Some(CopiedStyle {
                style: Some(shape.resolved_style()),
                stroke: Some(shape.effective_stroke()),
                opacity: shape.opacity,
            });
        } else if let Some(line) = page.line(id) {
            self.copied_style = Some(CopiedStyle {
                style: None,
                stroke: Some(line.resolved_stroke()),
                opacity: None,
            });
        } else {
            return false;
        }
        self.status = "Copied style".into();
        true
    }

    /// Applies the remembered style to everything selected.
    pub fn paste_style(&mut self) {
        let Some(copied) = self.copied_style.clone() else {
            self.status = "No style copied".into();
            return;
        };
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }

        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if shape.shape_type.accepts_style() {
                    if let Some(style) = &copied.style {
                        shape.style = Some(style.clone());
                    } else if let Some(stroke) = copied.stroke {
                        let s = shape.style.get_or_insert_with(Default::default);
                        s.stroke = Some(stroke);
                    }
                }
                shape.opacity = copied.opacity;
            } else if let Some(line) = self.page_mut().line_mut(&id) {
                line.stroke = copied.stroke;
            }
        }
        self.status = "Applied style".into();
    }

    pub fn has_copied_style(&self) -> bool {
        self.copied_style.is_some()
    }

    /// The id of the selection, when exactly one thing is selected.
    fn single_selected_id(&self) -> Option<String> {
        match self.selection.as_slice() {
            [id] => Some(id.clone()),
            _ => None,
        }
    }

    /// The navigation action on the selected shape, if there is exactly one
    /// shape selected and it has one.
    pub fn selection_link(&self) -> Option<dc_model::Action> {
        let id = self.single_selected_id()?;
        let shape = self.page().shape(&id)?;
        shape
            .actions
            .as_ref()?
            .iter()
            .find(|a| matches!(a, dc_model::Action::Url { .. } | dc_model::Action::GotoPage { .. }))
            .cloned()
    }

    /// Sets the selected shape's link to a URL, replacing whatever
    /// navigation action it had.
    pub fn set_link_url(&mut self, url: &str) {
        let Some(id) = self.single_selected_id() else {
            return;
        };
        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(&id) {
            let actions = shape.actions.get_or_insert_with(Vec::new);
            actions.retain(|a| {
                !matches!(a, dc_model::Action::Url { .. } | dc_model::Action::GotoPage { .. })
            });
            actions.push(dc_model::Action::Url { url: url.to_string(), new_window: None });
        }
    }

    /// Removes the selected shape's link, if it has one.
    pub fn clear_link(&mut self) {
        let Some(id) = self.single_selected_id() else {
            return;
        };
        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(&id) {
            if let Some(actions) = shape.actions.as_mut() {
                actions.retain(|a| {
                    !matches!(a, dc_model::Action::Url { .. } | dc_model::Action::GotoPage { .. })
                });
                if actions.is_empty() {
                    shape.actions = None;
                }
            }
        }
    }

    /// The custom-data entries on the selected shape, when exactly one
    /// shape is selected.
    pub fn selection_custom_data(&self) -> Vec<dc_model::CustomDataEntry> {
        let Some(id) = self.single_selected_id() else {
            return Vec::new();
        };
        self.page()
            .shape(&id)
            .and_then(|s| s.custom_data.clone())
            .unwrap_or_default()
    }

    /// Appends an empty entry to the selected shape's custom data.
    pub fn add_custom_data(&mut self) {
        let Some(id) = self.single_selected_id() else {
            return;
        };
        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(&id) {
            shape
                .custom_data
                .get_or_insert_with(Vec::new)
                .push(dc_model::CustomDataEntry { key: String::new(), value: String::new() });
        }
    }

    /// Removes one custom-data entry from the selected shape, by index.
    pub fn remove_custom_data(&mut self, index: usize) {
        let Some(id) = self.single_selected_id() else {
            return;
        };
        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(&id) {
            if let Some(entries) = shape.custom_data.as_mut() {
                if index < entries.len() {
                    entries.remove(index);
                }
                if entries.is_empty() {
                    shape.custom_data = None;
                }
            }
        }
    }

    /// Sets the key of one custom-data entry on the selected shape.
    pub fn set_custom_data_key(&mut self, index: usize, key: &str) {
        self.set_custom_data_field(index, |entry| entry.key = key.to_string());
    }

    /// Sets the value of one custom-data entry on the selected shape.
    pub fn set_custom_data_value(&mut self, index: usize, value: &str) {
        self.set_custom_data_field(index, |entry| entry.value = value.to_string());
    }

    fn set_custom_data_field(&mut self, index: usize, mutate: impl Fn(&mut dc_model::CustomDataEntry)) {
        let Some(id) = self.single_selected_id() else {
            return;
        };
        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(&id) {
            if let Some(entry) = shape.custom_data.as_mut().and_then(|e| e.get_mut(index)) {
                mutate(entry);
            }
        }
    }

    // -------------------------------------------------------------- layers

    pub fn layers(&self) -> &[dc_model::Layer] {
        self.page().layers()
    }

    /// The layer a shape or line belongs to, if any.
    pub fn layer_of(&self, id: &str) -> Option<&dc_model::Layer> {
        self.page().layers().iter().find(|l| l.items().iter().any(|i| i == id))
    }

    fn layer_mut(&mut self, id: &str) -> Option<&mut dc_model::Layer> {
        self.page_mut().layers.as_mut()?.iter_mut().find(|l| l.id == id)
    }

    /// Adds a new, empty layer on top of the others.
    pub fn add_layer(&mut self) -> String {
        self.begin_change();
        let id = self.package.document.fresh_id("layer");
        let title = format!("Layer {}", self.page().layers().len() + 1);
        let layer = dc_model::Layer {
            id: id.clone(),
            title: Some(title),
            items: None,
            visible: None,
            locked: None,
            extra: Default::default(),
        };
        self.page_mut().layers.get_or_insert_with(Vec::new).push(layer);
        id
    }

    /// Removes a layer. Shapes and lines that were in it are not deleted —
    /// they simply belong to no layer, the same as before one existed.
    pub fn delete_layer(&mut self, id: &str) {
        self.begin_change();
        if let Some(layers) = self.page_mut().layers.as_mut() {
            layers.retain(|l| l.id != id);
        }
    }

    pub fn rename_layer(&mut self, id: &str, title: &str) {
        self.begin_change();
        if let Some(layer) = self.layer_mut(id) {
            layer.title = Some(title.to_string());
        }
    }

    pub fn toggle_layer_visibility(&mut self, id: &str) {
        self.begin_change();
        if let Some(layer) = self.layer_mut(id) {
            layer.visible = Some(!layer.is_visible());
        }
    }

    pub fn toggle_layer_lock(&mut self, id: &str) {
        self.begin_change();
        if let Some(layer) = self.layer_mut(id) {
            layer.locked = Some(!layer.is_locked());
        }
    }

    /// Moves a layer one slot up or down in the panel's list.
    pub fn move_layer(&mut self, id: &str, up: bool) {
        self.begin_change();
        if let Some(layers) = self.page_mut().layers.as_mut() {
            if let Some(pos) = layers.iter().position(|l| l.id == id) {
                let target = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|&p| p < layers.len()) };
                if let Some(target) = target {
                    layers.swap(pos, target);
                }
            }
        }
    }

    /// Moves the current selection into a layer, removing its members from
    /// whichever layer they were in before.
    pub fn move_selection_to_layer(&mut self, layer_id: &str) {
        let ids = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        if let Some(layers) = self.page_mut().layers.as_mut() {
            for layer in layers.iter_mut() {
                if let Some(items) = layer.items.as_mut() {
                    items.retain(|i| !ids.contains(i));
                }
            }
        }
        if let Some(layer) = self.layer_mut(layer_id) {
            layer.items.get_or_insert_with(Vec::new).extend(ids);
        }
    }

    // ---------------------------------------------------------------- find

    /// Every shape or line whose label contains the query, case- and
    /// markup-insensitively. Only the first label on a connector is
    /// searched.
    pub fn find_matches(&self, query: &str) -> Vec<String> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let q = query.to_lowercase();
        let page = self.page();
        let mut ids = Vec::new();
        for shape in page.shapes() {
            if let Some((text, base)) = label_of_on(page, &shape.id) {
                if dc_model::markup::parse(&text, base).plain().to_lowercase().contains(&q) {
                    ids.push(shape.id.clone());
                }
            }
        }
        for line in page.lines() {
            if let Some((text, base)) = label_of_on(page, &line.id) {
                if dc_model::markup::parse(&text, base).plain().to_lowercase().contains(&q) {
                    ids.push(line.id.clone());
                }
            }
        }
        ids
    }

    /// Selects one item and frames it, the way clicking a find result does.
    pub fn select_and_frame(&mut self, id: &str) {
        self.selection = vec![id.to_string()];
        self.zoom_to_selection();
    }

    /// Replaces the first occurrence of `query` in an item's label with
    /// `replacement`, matched ASCII-case-insensitively (so the byte offset
    /// found in the lowercased copy still lines up with the original —
    /// true Unicode case folding can change a string's length, which plain
    /// text does not need to handle to be useful here).
    ///
    /// The label is rewritten as one plain run: a replace clears whatever
    /// character styling it had, the same simplification a quick find and
    /// replace makes in most editors.
    pub fn replace_in(&mut self, id: &str, query: &str, replacement: &str) {
        if query.is_empty() {
            return;
        }
        let Some((text, base)) = self.label_of(id) else {
            return;
        };
        let parsed = dc_model::markup::parse(&text, base.clone());
        let plain = parsed.plain();
        let Some(pos) = plain.to_ascii_lowercase().find(&query.to_ascii_lowercase()) else {
            return;
        };

        let mut updated = String::with_capacity(plain.len() - query.len() + replacement.len());
        updated.push_str(&plain[..pos]);
        updated.push_str(replacement);
        updated.push_str(&plain[pos + query.len()..]);

        let replaced = dc_model::ParsedText {
            runs: vec![dc_model::TextRun { text: updated, style: base.clone(), link: None }],
            align: parsed.align,
            vertical_align: parsed.vertical_align,
        };
        let markup = dc_model::markup::to_markup(&replaced, &base);

        self.begin_change();
        if let Some(shape) = self.page_mut().shape_mut(id) {
            shape.text = Some(Text::new(markup));
            return;
        }
        if let Some(line) = self.page_mut().line_mut(id) {
            if let Some(label) = line.text.as_mut().and_then(|l| l.first_mut()) {
                label.text = markup;
            }
        }
    }

    /// Sets the dash pattern of the selected shapes and connectors.
    pub fn set_selection_stroke_style(&mut self, style: dc_model::StrokeStyle) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if !shape.shape_type.accepts_style() {
                    continue;
                }
                let s = shape.style.get_or_insert_with(Default::default);
                let mut stroke = s.stroke.unwrap_or_default();
                stroke.style = Some(style);
                s.stroke = Some(stroke);
            } else if let Some(line) = self.page_mut().line_mut(&id) {
                let mut stroke = line.stroke.unwrap_or_default();
                stroke.style = Some(style);
                line.stroke = Some(stroke);
            }
        }
    }

    // ------------------------------------------------- what is selected

    /// A property of every selected item, when they all agree on it.
    ///
    /// The properties panel shows the current value this way: a row can only
    /// highlight a choice when the whole selection actually has it, and shows
    /// nothing highlighted when the selection is mixed.
    fn agreed<T: PartialEq>(&self, read: impl Fn(&Page, &str) -> Option<T>) -> Option<T> {
        let page = self.page();
        let mut agreed: Option<T> = None;
        for id in &self.selection {
            let value = read(page, id)?;
            match &agreed {
                Some(existing) if *existing != value => return None,
                Some(_) => {}
                None => agreed = Some(value),
            }
        }
        agreed
    }

    /// The fill shared by the selection, if they share one.
    pub fn selection_fill(&self) -> Option<Color> {
        self.agreed(|page, id| {
            let shape = page.shape(id)?;
            shape.shape_type.accepts_style().then_some(())?;
            shape.resolved_style().resolved_fill().color()
        })
    }

    /// The line colour shared by the selection.
    pub fn selection_stroke_color(&self) -> Option<Color> {
        self.agreed(|page, id| {
            if let Some(shape) = page.shape(id) {
                return Some(shape.effective_stroke().resolved_color());
            }
            Some(page.line(id)?.resolved_stroke().resolved_color())
        })
    }

    /// The line weight shared by the selection.
    pub fn selection_stroke_width(&self) -> Option<f64> {
        self.agreed(|page, id| {
            if let Some(shape) = page.shape(id) {
                return Some(shape.effective_stroke().resolved_width().to_bits());
            }
            Some(page.line(id)?.resolved_stroke().resolved_width().to_bits())
        })
        .map(f64::from_bits)
    }

    /// The dash pattern shared by the selection.
    pub fn selection_stroke_style(&self) -> Option<dc_model::StrokeStyle> {
        self.agreed(|page, id| {
            if let Some(shape) = page.shape(id) {
                return Some(shape.effective_stroke().resolved_style());
            }
            Some(page.line(id)?.resolved_stroke().resolved_style())
        })
    }

    /// The routing shared by the selected connectors. `None` when nothing
    /// selected is a connector, because then the row has nothing to report.
    pub fn selection_line_type(&self) -> Option<LineType> {
        let page = self.page();
        let mut agreed: Option<LineType> = None;
        for id in &self.selection {
            let Some(line) = page.line(id) else { continue };
            match agreed {
                Some(existing) if existing != line.line_type => return None,
                Some(_) => {}
                None => agreed = Some(line.line_type),
            }
        }
        agreed
    }

    /// The text colour shared by the selection, or by the characters
    /// highlighted inside an open label.
    pub fn selection_text_color(&self) -> Option<Color> {
        if let (Some(edit), Some(range)) = (self.text_edit.as_ref(), self.styling_range()) {
            let styles = edit.styles_in(range);
            let first = styles.first()?.color;
            return styles.iter().all(|s| s.color == first).then_some(first);
        }
        self.agreed(|page, id| {
            let (text, base) = label_of_on(page, id)?;
            let parsed = dc_model::markup::parse(&text, base);
            let first = parsed.runs.first()?.style.color;
            parsed
                .runs
                .iter()
                .all(|r| r.text.is_empty() || r.style.color == first)
                .then_some(first)
        })
    }

    /// The font family shared by the selection, or by the characters
    /// highlighted inside an open label.
    pub fn selection_font_family(&self) -> Option<String> {
        if let (Some(edit), Some(range)) = (self.text_edit.as_ref(), self.styling_range()) {
            let styles = edit.styles_in(range);
            let first = styles.first()?.font_family.clone();
            return styles
                .iter()
                .all(|s| s.font_family == first)
                .then_some(first);
        }
        self.agreed(|page, id| {
            let (text, base) = label_of_on(page, id)?;
            let parsed = dc_model::markup::parse(&text, base);
            let first = parsed.runs.first()?.style.font_family.clone();
            parsed
                .runs
                .iter()
                .all(|r| r.text.is_empty() || r.style.font_family == first)
                .then_some(first)
        })
    }

    /// Sets the font family on the selection, or on the characters
    /// highlighted inside an open label.
    pub fn set_font_family(&mut self, family: &str) {
        if self.restyle_open_label(|style| style.font_family = family.to_string()) {
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::restyle(markup, base, |style| style.font_family = family.to_string())
            });
        }
    }

    pub fn set_selection_line_type(&mut self, line_type: LineType) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(line) = self.page_mut().line_mut(&id) {
                line.line_type = line_type;
            }
        }
    }

    /// Replaces the text of every selected shape.
    pub fn set_selection_text(&mut self, text: &str) {
        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            if let Some(shape) = self.page_mut().shape_mut(&id) {
                if shape.shape_type.accepts_text() {
                    shape.text = Some(Text::new(text));
                }
            }
        }
    }

    // -------------------------------------------------------- text styling

    /// The byte range of the open label that formatting applies to.
    ///
    /// With text selected inside a label, formatting applies to exactly that
    /// run of characters. With a bare caret it arms the next thing typed. With
    /// no label open at all, this is `None` and formatting falls through to the
    /// whole label of every selected item.
    fn styling_range(&self) -> Option<std::ops::Range<usize>> {
        Some(self.text_edit.as_ref()?.selected_range())
    }

    /// Applies a style change to the open label, if there is one.
    ///
    /// Returns whether the change was handled here; `false` means the caller
    /// should fall back to restyling whole labels.
    fn restyle_open_label(&mut self, mutate: impl Fn(&mut dc_model::TextStyle)) -> bool {
        let Some(range) = self.styling_range() else {
            return false;
        };
        if let Some(edit) = self.text_edit.as_mut() {
            edit.restyle(range, mutate);
        }
        true
    }

    /// A character style property that can be switched on or off.
    pub fn toggle_text_style(&mut self, which: TextToggle) {
        // While a label is open, formatting is a selection operation: it
        // changes the characters the user highlighted, not the whole label.
        if let Some(range) = self.styling_range() {
            let toggle = which.as_style_toggle();
            if let Some(edit) = self.text_edit.as_mut() {
                let enabled = !edit.range_has(range.clone(), toggle);
                edit.toggle_style(range, toggle);
                self.status = format!(
                    "{} {}",
                    if enabled { "Applied" } else { "Removed" },
                    which.name()
                );
            }
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }

        // Turning on unless everything already has it, which is how a toggle
        // over a mixed selection should behave.
        let probe = which.probe();
        let all_on = ids.iter().all(|id| {
            let (text, base) = match self.label_of(id) {
                Some(pair) => pair,
                None => return true,
            };
            dc_model::markup::all_runs(&text, &base, probe)
        });
        let enable = !all_on;

        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::restyle(markup, base, |style| which.apply(style, enable))
            });
        }
        self.status = format!("{} {}", if enable { "Applied" } else { "Removed" }, which.name());
    }

    /// Multiplies the font size, which is what the size buttons do.
    pub fn scale_text_size(&mut self, factor: f64) {
        if self.restyle_open_label(|style| {
            style.font_size_pt = (style.font_size_pt * factor).clamp(4.0, 144.0);
        }) {
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::restyle(markup, base, |style| {
                    // Clamped so repeated clicks cannot reach an unusable size.
                    style.font_size_pt = (style.font_size_pt * factor).clamp(4.0, 144.0);
                })
            });
        }
    }

    pub fn set_text_color(&mut self, color: Color) {
        if self.restyle_open_label(move |style| style.color = color) {
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::restyle(markup, base, |style| style.color = color)
            });
        }
    }

    pub fn set_text_align(&mut self, align: dc_model::TextAlign) {
        // Alignment is a block property, so an open label takes it whole
        // rather than over a selected range.
        if let Some(edit) = self.text_edit.as_mut() {
            edit.set_align(align);
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::realign(markup, base, |parsed| parsed.align = align)
            });
        }
    }

    pub fn set_vertical_align(&mut self, align: dc_model::VerticalAlign) {
        if let Some(edit) = self.text_edit.as_mut() {
            edit.set_vertical_align(align);
            return;
        }

        let ids: Vec<String> = self.selection.clone();
        if ids.is_empty() {
            return;
        }
        self.begin_change();
        for id in ids {
            self.rewrite_label(&id, |markup, base| {
                dc_model::markup::realign(markup, base, |parsed| parsed.vertical_align = align)
            });
        }
    }

    /// The stored markup of an item's label and the style it resolves against.
    fn label_of(&self, id: &str) -> Option<(String, dc_model::TextStyle)> {
        label_of_on(self.page(), id)
    }

    /// Applies a markup transform to an item's label in place.
    fn rewrite_label(
        &mut self,
        id: &str,
        transform: impl Fn(&str, &dc_model::TextStyle) -> String,
    ) {
        let Some((existing, base)) = self.label_of(id) else {
            return;
        };
        if existing.is_empty() {
            return;
        }
        let updated = transform(&existing, &base);

        if let Some(shape) = self.page_mut().shape_mut(id) {
            shape.text = Some(Text::new(updated));
            return;
        }
        if let Some(line) = self.page_mut().line_mut(id) {
            if let Some(label) = line.text.as_mut().and_then(|l| l.first_mut()) {
                label.text = updated;
            }
        }
    }

    pub fn add_page(&mut self) {
        self.begin_change();
        let id = self.package.document.fresh_id("page");
        let number = self.package.document.pages.len() + 1;
        self.package
            .document
            .pages
            .push(Page::new(id, format!("Page {number}")));
        self.page_index = self.package.document.pages.len() - 1;
        self.selection.clear();
    }

    /// Removes a page. The last page cannot be removed, because a document
    /// with no pages is not representable in the format.
    pub fn delete_page(&mut self, index: usize) -> bool {
        if self.package.document.pages.len() <= 1 {
            self.status = "A document needs at least one page".into();
            return false;
        }
        if index >= self.package.document.pages.len() {
            return false;
        }

        self.cancel_text_edit();
        self.begin_change();
        let removed = self.package.document.pages.remove(index);
        self.page_index = self
            .page_index
            .min(self.package.document.pages.len() - 1);
        self.selection.clear();
        self.status = format!("Deleted {}", removed.title);
        true
    }

    /// Copies a page and everything on it, inserted after the original.
    pub fn duplicate_page(&mut self, index: usize) {
        let Some(original) = self.package.document.pages.get(index).cloned() else {
            return;
        };
        self.begin_change();

        let mut copy = original;
        copy.title = format!("{} copy", copy.title);

        // Item ids must be unique across the whole document, not just within a
        // page, because connectors reference them unqualified. Every id on the
        // copy is therefore reassigned against the ids already in use.
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        for page in &self.package.document.pages {
            taken.insert(page.id.clone());
            for shape in page.shapes() {
                taken.insert(shape.id.clone());
            }
            for line in page.lines() {
                taken.insert(line.id.clone());
            }
        }
        let mint = |prefix: &str, taken: &mut std::collections::HashSet<String>| {
            for n in 1..usize::MAX {
                let candidate = format!("{prefix}{n}");
                if taken.insert(candidate.clone()) {
                    return candidate;
                }
            }
            unreachable!("exhausted the id space")
        };

        copy.id = mint("page", &mut taken);
        let mut remap: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for shape in copy.shapes_mut() {
            let fresh = mint("shape", &mut taken);
            remap.insert(shape.id.clone(), fresh.clone());
            shape.id = fresh;
        }
        for line in copy.lines_mut() {
            line.id = mint("line", &mut taken);
            for endpoint in [&mut line.endpoint1, &mut line.endpoint2] {
                if let Endpoint::ShapeEndpoint { shape_id, .. } = endpoint {
                    if let Some(mapped) = remap.get(shape_id.as_str()) {
                        *shape_id = mapped.clone();
                    }
                }
            }
        }

        self.package.document.pages.insert(index + 1, copy);
        self.page_index = index + 1;
        self.selection.clear();
        self.status = "Duplicated page".into();
    }

    /// Opens a page title for editing in the tab bar.
    ///
    /// `area` is in window coordinates, because the tab bar is interface
    /// chrome rather than document content.
    /// `surface` is the colour of the tab bar, which is interface chrome and
    /// so is themed, unlike anything on the canvas.
    pub fn begin_page_title_edit(
        &mut self,
        index: usize,
        area: kurbo::Rect,
        surface: Color,
        cx: &mut TextCtx<'_>,
    ) -> bool {
        let Some(page) = self.package.document.pages.get(index) else {
            return false;
        };
        let title = page.title.clone();
        let base = dc_model::TextStyle {
            // A page title is chrome, so it is written in whatever reads
            // against the panel rather than in the document's text colour.
            color: surface.contrasting_text(),
            align: dc_model::TextAlign::Center,
            ..Default::default()
        };
        self.text_edit = Some(TextEdit::begin(
            EditTarget::PageTitle(index),
            &title,
            &base,
            surface,
            area,
            true,
            cx,
        ));
        true
    }

    /// Whether the open edit belongs to the tab bar rather than the canvas.
    pub fn editing_page_title(&self) -> Option<usize> {
        match self.text_edit.as_ref()?.target {
            EditTarget::PageTitle(index) => Some(index),
            _ => None,
        }
    }

    /// Moves a page to a new position, keeping it current.
    ///
    /// `to` is the index the page ends up at, so dragging the first tab to the
    /// end is `move_page(0, last)`. Returns whether anything moved.
    pub fn move_page(&mut self, from: usize, to: usize) -> bool {
        let count = self.package.document.pages.len();
        if from >= count || count < 2 {
            return false;
        }
        let to = to.min(count - 1);
        if from == to {
            return false;
        }

        // A title being edited belongs to a page index that is about to mean a
        // different page.
        self.cancel_text_edit();
        self.begin_change();
        let page = self.package.document.pages.remove(from);
        let title = page.title.clone();
        self.package.document.pages.insert(to, page);

        // Follow whichever page the user was on, which is the dragged one when
        // they dragged the current tab.
        self.page_index = if self.page_index == from {
            to
        } else {
            // Every page between the two shifts by one.
            let mut index = self.page_index;
            if from < self.page_index && self.page_index <= to {
                index -= 1;
            } else if to <= self.page_index && self.page_index < from {
                index += 1;
            }
            index
        };

        self.status = format!("Moved {title}");
        true
    }

    pub fn select_page(&mut self, index: usize) {
        if index < self.package.document.pages.len() {
            self.page_index = index;
            self.selection.clear();
            self.hovered = None;
        }
    }

    /// The horizontal alignment of what a control would act upon, if every
    /// part of it agrees.
    ///
    /// With a label open this is that label's alignment; otherwise it is the
    /// alignment shared by every selected item, or `None` when they differ.
    pub fn text_align(&self) -> Option<dc_model::TextAlign> {
        if let Some(edit) = self.text_edit.as_ref() {
            return Some(edit.align());
        }
        self.agreed_block_attribute(|parsed| parsed.align)
    }

    /// The vertical alignment of what a control would act upon.
    pub fn vertical_align(&self) -> Option<dc_model::VerticalAlign> {
        if let Some(edit) = self.text_edit.as_ref() {
            return Some(edit.vertical_align());
        }
        self.agreed_block_attribute(|parsed| parsed.vertical_align)
    }

    /// A block attribute of every selected label, when they all agree on it.
    fn agreed_block_attribute<T: PartialEq>(
        &self,
        read: impl Fn(&dc_model::ParsedText) -> T,
    ) -> Option<T> {
        let mut agreed: Option<T> = None;
        for id in &self.selection {
            let (text, base) = self.label_of(id)?;
            let value = read(&dc_model::markup::parse(&text, base));
            match &agreed {
                Some(existing) if *existing != value => return None,
                Some(_) => {}
                None => agreed = Some(value),
            }
        }
        agreed
    }

    /// Whether a formatting property is on for what a control would act upon.
    ///
    /// With a label open this reads the selected characters; otherwise it reads
    /// the labels of the selected items. Menus use it to show a check mark.
    pub fn text_style_is_on(&self, which: TextToggle) -> bool {
        if let (Some(edit), Some(range)) = (self.text_edit.as_ref(), self.styling_range()) {
            return edit.range_has(range, which.as_style_toggle());
        }
        let probe = which.probe();
        !self.selection.is_empty()
            && self.selection.iter().all(|id| {
                match self.label_of(id) {
                    Some((text, base)) => dc_model::markup::all_runs(&text, &base, probe),
                    None => false,
                }
            })
    }
}

/// The stored markup of an item's label and the style it resolves against.
///
/// Taken on a page rather than on the editor so the selection queries can use
/// it without borrowing the whole editor.
fn label_of_on(page: &Page, id: &str) -> Option<(String, dc_model::TextStyle)> {
    if let Some(shape) = page.shape(id) {
        if !shape.shape_type.accepts_text() {
            return None;
        }
        let base = dc_model::TextStyle {
            color: shape.resolved_style().resolved_text_color(),
            ..Default::default()
        };
        return Some((shape.text_str().to_string(), base));
    }
    let line = page.line(id)?;
    let label = line.labels().first()?;
    let base = dc_model::TextStyle {
        color: line.resolved_stroke().resolved_color(),
        ..Default::default()
    };
    Some((label.text.clone(), base))
}

/// A short, display-friendly name for a path.
fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// The angle from `center` to `p`, in degrees, clockwise from straight up.
fn angle_of(center: Point, p: Point) -> f64 {
    (p.y - center.y).atan2(p.x - center.x).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parley::{FontContext, LayoutContext};

    /// Owns the Parley contexts a test needs to drive text editing.
    struct TestCx {
        fonts: FontContext,
        layouts: LayoutContext<peniko::Brush>,
    }

    impl TestCx {
        fn new() -> Self {
            TestCx { fonts: FontContext::new(), layouts: LayoutContext::new() }
        }

        fn ctx(&mut self) -> TextCtx<'_> {
            TextCtx::new(&mut self.fonts, &mut self.layouts)
        }
    }

    fn editor_with_two_shapes() -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(100.0, 100.0, 100.0, 60.0));
        e.create_shape(ShapeType::Decision, BoundingBox::new(400.0, 100.0, 100.0, 60.0));
        e.selection.clear();
        // Treat the fixture as the saved baseline so tests can assert on
        // history and the dirty flag from a clean start.
        e.dirty = false;
        e.history.clear();
        e
    }

    fn mods() -> Mods {
        Mods::default()
    }

    #[test]
    fn a_new_editor_has_one_empty_page() {
        let e = Editor::new();
        assert_eq!(e.document().pages.len(), 1);
        assert_eq!(e.page().shapes().len(), 0);
        assert!(!e.dirty);
        assert!(!e.can_undo());
    }

    #[test]
    fn creating_a_shape_selects_it_and_marks_dirty() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Process, BoundingBox::new(0.0, 0.0, 80.0, 40.0));
        assert_eq!(e.selection, vec![id.clone()]);
        assert!(e.dirty);
        assert!(e.page().shape(&id).is_some());
    }

    #[test]
    fn clicking_a_shape_selects_it() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        // Shapes are at document coordinates; identity camera maps 1:1.
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());
        assert_eq!(e.selection.len(), 1);
    }

    #[test]
    fn clicking_empty_space_clears_the_selection() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.select_all();
        assert!(!e.selection.is_empty());

        e.pointer_down(Point::new(800.0, 600.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(800.0, 600.0), mods(), &mut tcx.ctx());
        assert!(e.selection.is_empty());
    }

    #[test]
    fn shift_click_extends_the_selection() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());

        let shift = Mods { shift: true, ..Default::default() };
        e.pointer_down(Point::new(450.0, 130.0), shift, false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), shift, &mut tcx.ctx());
        assert_eq!(e.selection.len(), 2);
    }

    #[test]
    fn shift_click_on_a_selected_shape_deselects_it() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.select_all();
        let before = e.selection.len();

        let shift = Mods { shift: true, ..Default::default() };
        e.pointer_down(Point::new(150.0, 130.0), shift, false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(150.0, 130.0), shift, &mut tcx.ctx());
        assert_eq!(e.selection.len(), before - 1);
    }

    #[test]
    fn dragging_a_shape_moves_it() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.snap_enabled = false;
        e.snap_to_grid = false;
        let id = e.page().shapes()[0].id.clone();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(250.0, 230.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(250.0, 230.0), mods(), &mut tcx.ctx());

        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!((bb.x, bb.y), (200.0, 200.0), "shape should have moved by (100,100)");
    }

    #[test]
    fn a_whole_drag_is_one_undo_step() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.snap_enabled = false;
        e.snap_to_grid = false;
        let id = e.page().shapes()[0].id.clone();
        let before = e.page().shape(&id).unwrap().bounding_box;

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        for step in 1..=10 {
            let p = Point::new(150.0 + step as f64 * 10.0, 130.0);
            e.pointer_move(p, mods(), &mut tcx.ctx());
        }
        e.pointer_up(Point::new(250.0, 130.0), mods(), &mut tcx.ctx());

        e.undo();
        let after = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!(after.x, before.x, "one undo should revert the entire drag");
    }

    #[test]
    fn a_click_does_not_nudge_the_shape() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        let before = e.page().shape(&id).unwrap().bounding_box;

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        // A couple of pixels of jitter, below the drag threshold.
        e.pointer_move(Point::new(151.0, 131.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(151.0, 131.0), mods(), &mut tcx.ctx());

        let after = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!((after.x, after.y), (before.x, before.y));
        assert!(!e.can_undo(), "a click should not create an undo step");
    }

    // -------------------------------------------------------- freehand tool

    #[test]
    fn the_freehand_tool_draws_a_stroke() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.tool = Tool::Freehand;

        e.pointer_down(Point::new(100.0, 100.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(150.0, 180.0), mods(), &mut tcx.ctx());
        e.pointer_move(Point::new(200.0, 100.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(200.0, 100.0), mods(), &mut tcx.ctx());

        assert_eq!(e.page().shapes().len(), 1);
        let shape = &e.page().shapes()[0];
        assert_eq!(shape.shape_type, ShapeType::FreehandDrawing);
        assert_eq!(shape.points.as_ref().map(Vec::len), Some(3));

        let bb = shape.bounding_box;
        assert!((bb.x - 100.0).abs() < 0.5 && (bb.y - 100.0).abs() < 0.5);
        assert!((bb.w - 100.0).abs() < 0.5 && (bb.h - 80.0).abs() < 0.5);

        // The pen stays armed, unlike placing a shape, so a string of
        // strokes does not need re-selecting the tool each time.
        assert_eq!(e.tool, Tool::Freehand);
    }

    #[test]
    fn a_plain_click_with_the_freehand_tool_draws_nothing() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.tool = Tool::Freehand;

        e.pointer_down(Point::new(100.0, 100.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(100.0, 100.0), mods(), &mut tcx.ctx());

        assert!(e.page().shapes().is_empty());
        assert!(!e.can_undo(), "nothing was created, so there is nothing to undo");
    }

    #[test]
    fn a_freehand_stroke_is_one_undo_step() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.tool = Tool::Freehand;

        e.pointer_down(Point::new(100.0, 100.0), mods(), false, 1, &mut tcx.ctx());
        for step in 1..=10 {
            e.pointer_move(Point::new(100.0 + step as f64 * 5.0, 100.0 + step as f64 * 3.0), mods(), &mut tcx.ctx());
        }
        e.pointer_up(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());
        assert_eq!(e.page().shapes().len(), 1);

        e.undo();
        assert!(e.page().shapes().is_empty(), "one undo should remove the whole stroke");
    }

    #[test]
    fn snapping_aligns_a_dragged_shape_to_its_neighbor() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(100.0, 100.0, 100.0, 60.0));
        let moving = e.create_shape(ShapeType::Rectangle, BoundingBox::new(400.0, 400.0, 100.0, 60.0));
        e.selection.clear();
        e.snap_enabled = true;

        // Drag the second shape so its left edge lands 3 units from 100.
        e.pointer_down(Point::new(450.0, 430.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(153.0, 430.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(153.0, 430.0), mods(), &mut tcx.ctx());

        let bb = e.page().shape(&moving).unwrap().bounding_box;
        assert_eq!(bb.x, 100.0, "left edges should align exactly; got {}", bb.x);
    }

    #[test]
    fn dragging_on_empty_canvas_lands_on_the_grid() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id.clone()];
        e.snap_enabled = true;
        e.grid_spacing = 9.0;

        // Drag by an amount that is not a grid multiple.
        e.pointer_down(Point::new(50.0, 30.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(50.0 + 103.0, 30.0 + 47.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(50.0 + 103.0, 30.0 + 47.0), mods(), &mut tcx.ctx());

        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!(bb.x % 9.0, 0.0, "x was {}", bb.x);
        assert_eq!(bb.y % 9.0, 0.0, "y was {}", bb.y);
    }

    #[test]
    fn alignment_beats_the_grid_when_a_neighbor_is_near() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        // A neighbor on a deliberately off-grid x.
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(101.0, 400.0, 100.0, 60.0));
        let moving = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![moving.clone()];
        e.snap_enabled = true;

        e.pointer_down(Point::new(50.0, 30.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(153.0, 30.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(153.0, 30.0), mods(), &mut tcx.ctx());

        let bb = e.page().shape(&moving).unwrap().bounding_box;
        assert_eq!(bb.x, 101.0, "alignment should win over the grid; got {}", bb.x);
    }

    #[test]
    fn the_text_area_follows_a_shape_that_moves() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        let before = e.text_edit.as_ref().unwrap().area();

        // Move the shape out from under the open label.
        if let Some(shape) = e.page_mut().shape_mut(&id) {
            shape.bounding_box = shape.bounding_box.translated(200.0, 150.0);
        }
        e.refresh_text_area();

        let after = e.text_edit.as_ref().unwrap().area();
        assert!(
            (after.x0 - before.x0 - 200.0).abs() < 1e-6,
            "the caret would hit-test against the old position: {before:?} -> {after:?}"
        );
        assert!((after.y0 - before.y0 - 150.0).abs() < 1e-6);
    }

    #[test]
    fn refreshing_the_area_without_an_edit_is_harmless() {
        let mut e = editor_with_two_shapes();
        e.refresh_text_area();
        assert!(!e.is_editing());
    }

    #[test]
    fn command_bypasses_snapping() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(100.0, 100.0, 100.0, 60.0));
        let moving = e.create_shape(ShapeType::Rectangle, BoundingBox::new(400.0, 400.0, 100.0, 60.0));
        e.selection = vec![moving.clone()];

        // Command toggles selection on press, so the bypass is expressed by
        // holding it during the drag rather than at the click.
        let cmd = Mods { command: true, ..Default::default() };
        e.pointer_down(Point::new(450.0, 430.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(153.0, 430.0), cmd, &mut tcx.ctx());
        e.pointer_up(Point::new(153.0, 430.0), cmd, &mut tcx.ctx());

        let bb = e.page().shape(&moving).unwrap().bounding_box;
        assert_eq!(bb.x, 103.0, "snapping should have been bypassed");
    }

    #[test]
    fn shift_locks_a_drag_to_one_axis() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.snap_enabled = false;
        e.snap_to_grid = false;
        let id = e.page().shapes()[0].id.clone();

        let shift = Mods { shift: true, ..Default::default() };
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(300.0, 160.0), shift, &mut tcx.ctx());
        e.pointer_up(Point::new(300.0, 160.0), shift, &mut tcx.ctx());

        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!(bb.y, 100.0, "vertical movement should be suppressed");
        assert_eq!(bb.x, 250.0);
    }

    #[test]
    fn marquee_drag_selects_enclosed_shapes() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.pointer_down(Point::new(50.0, 50.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(600.0, 300.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(600.0, 300.0), mods(), &mut tcx.ctx());
        assert_eq!(e.selection.len(), 2);
    }

    #[test]
    fn resize_handle_changes_the_shape_size() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.selection = vec![id.clone()];

        // The bottom-right handle of a shape at (100,100,100,60) is at (200,160).
        let handle = e.handle_at(Point::new(200.0, 160.0));
        assert_eq!(handle, Some(HandleKind::BottomRight), "handle lookup failed");

        e.pointer_down(Point::new(200.0, 160.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(250.0, 200.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(250.0, 200.0), mods(), &mut tcx.ctx());

        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!((bb.w, bb.h), (150.0, 100.0), "got {bb:?}");
    }

    #[test]
    fn rotate_handle_sets_a_rotation() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.selection = vec![id.clone()];

        let rotate_at = Point::new(150.0, 100.0 - dc_render::handles::ROTATE_ARM_LENGTH);
        assert_eq!(e.handle_at(rotate_at), Some(HandleKind::Rotate));

        e.pointer_down(rotate_at, mods(), false, 1, &mut tcx.ctx());
        // Swing a quarter turn around the center at (150,130).
        e.pointer_move(Point::new(260.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(260.0, 130.0), mods(), &mut tcx.ctx());

        let rotation = e.page().shape(&id).unwrap().bounding_box.rotation_degrees();
        assert!(rotation > 0.0, "expected a rotation, got {rotation}");
    }

    #[test]
    fn connector_tool_links_two_shapes() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        assert_eq!(e.page().lines().len(), 1);
        let line = &e.page().lines()[0];
        assert!(line.endpoint1.shape_id().is_some());
        assert!(line.endpoint2.shape_id().is_some());
        assert_eq!(e.tool, Tool::Select, "the tool should revert after one use");
    }

    #[test]
    fn connector_on_empty_canvas_creates_nothing() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;

        e.pointer_down(Point::new(800.0, 600.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(850.0, 650.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(850.0, 650.0), mods(), &mut tcx.ctx());
        assert_eq!(e.page().lines().len(), 0);
    }

    #[test]
    fn shape_tool_click_places_a_default_sized_shape() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.tool = Tool::Shape(ShapeType::Terminator);

        e.pointer_down(Point::new(300.0, 200.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(300.0, 200.0), mods(), &mut tcx.ctx());

        assert_eq!(e.page().shapes().len(), 1);
        let bb = e.page().shapes()[0].bounding_box;
        let (w, h) = ShapeType::Terminator.default_size();
        assert_eq!((bb.w, bb.h), (w, h));
        // And it is centered on the click.
        assert_eq!(bb.center(), Point::new(300.0, 200.0));
    }

    #[test]
    fn shape_tool_drag_sizes_the_shape() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.tool = Tool::Shape(ShapeType::Process);

        e.pointer_down(Point::new(100.0, 100.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(300.0, 250.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(300.0, 250.0), mods(), &mut tcx.ctx());

        let bb = e.page().shapes()[0].bounding_box;
        assert_eq!((bb.x, bb.y, bb.w, bb.h), (100.0, 100.0, 200.0, 150.0));
    }

    #[test]
    fn deleting_a_shape_removes_its_connectors() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        assert_eq!(e.page().lines().len(), 1);

        e.selection = vec![e.page().shapes()[0].id.clone()];
        e.delete_selection();

        assert_eq!(e.page().shapes().len(), 1);
        assert_eq!(e.page().lines().len(), 0, "the dangling connector should go too");
    }

    #[test]
    fn undo_and_redo_walk_the_edit_history() {
        let mut e = Editor::new();
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(20.0, 0.0, 10.0, 10.0));
        assert_eq!(e.page().shapes().len(), 2);

        e.undo();
        assert_eq!(e.page().shapes().len(), 1);
        e.undo();
        assert_eq!(e.page().shapes().len(), 0);

        e.redo();
        assert_eq!(e.page().shapes().len(), 1);
        e.redo();
        assert_eq!(e.page().shapes().len(), 2);
    }

    #[test]
    fn undo_drops_selection_of_vanished_items() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(e.selection, vec![id]);
        e.undo();
        assert!(e.selection.is_empty(), "selection should not name a missing shape");
    }

    #[test]
    fn duplicate_offsets_and_selects_the_copies() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];
        let original = e.page().shapes()[0].bounding_box;

        e.duplicate_selection();
        assert_eq!(e.page().shapes().len(), 3);
        assert_eq!(e.selection.len(), 1);

        let copy = e.page().shape(&e.selection[0]).unwrap().bounding_box;
        assert!(copy.x > original.x && copy.y > original.y, "the copy should be offset");
    }

    #[test]
    fn nudge_moves_the_selection() {
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.selection = vec![id.clone()];
        e.nudge(5.0, -3.0);

        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!((bb.x, bb.y), (105.0, 97.0));
    }

    #[test]
    fn reorder_pushes_the_selection_past_everything() {
        let mut e = editor_with_two_shapes();
        let first = e.page().shapes()[0].id.clone();
        e.selection = vec![first.clone()];

        e.reorder_selection(true);
        let z = e.page().shape(&first).unwrap().resolved_z_index();
        let other = e.page().shapes()[1].resolved_z_index();
        assert!(z > other, "{z} should be above {other}");

        e.reorder_selection(false);
        let z = e.page().shape(&first).unwrap().resolved_z_index();
        assert!(z < other);
    }

    #[test]
    fn styling_applies_to_the_selection_only() {
        let mut e = editor_with_two_shapes();
        let target = e.page().shapes()[0].id.clone();
        let untouched = e.page().shapes()[1].id.clone();
        e.selection = vec![target.clone()];

        e.set_selection_fill(Color::rgb(0xff, 0x00, 0x00));
        let fill = e.page().shape(&target).unwrap().resolved_style().resolved_fill();
        assert_eq!(fill.color(), Some(Color::rgb(0xff, 0, 0)));

        let other = e.page().shape(&untouched).unwrap().resolved_style().resolved_fill();
        assert_eq!(other.color(), Some(Color::WHITE), "the other shape should be unchanged");
    }

    #[test]
    fn styling_skips_types_that_reject_it() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::TextBlock, BoundingBox::new(0.0, 0.0, 50.0, 20.0));
        e.selection = vec![id.clone()];
        e.set_selection_fill(Color::rgb(1, 2, 3));
        assert!(
            e.page().shape(&id).unwrap().style.is_none(),
            "a text block does not accept a style"
        );
    }

    #[test]
    fn text_is_skipped_on_types_that_reject_it() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Hotspot, BoundingBox::new(0.0, 0.0, 50.0, 20.0));
        e.selection = vec![id.clone()];
        e.set_selection_text("nope");
        assert!(e.page().shape(&id).unwrap().text.is_none());
    }

    #[test]
    fn middle_drag_pans_in_any_tool() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Shape(ShapeType::Rectangle);
        let before = e.camera.origin;

        e.pointer_down(Point::new(400.0, 300.0), mods(), true, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 320.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 320.0), mods(), &mut tcx.ctx());

        assert_ne!(e.camera.origin, before, "the view should have scrolled");
        assert_eq!(e.page().shapes().len(), 2, "no shape should have been created");
    }

    #[test]
    fn wheel_scrolls_and_ctrl_wheel_zooms() {
        let mut e = editor_with_two_shapes();
        let origin = e.camera.origin;
        e.wheel(Point::new(400.0, 300.0), 0.0, 50.0, false);
        assert_ne!(e.camera.origin, origin);

        let zoom = e.camera.zoom;
        e.wheel(Point::new(400.0, 300.0), 0.0, 100.0, true);
        assert!(e.camera.zoom > zoom, "ctrl-wheel should zoom in");
    }

    #[test]
    fn zoom_to_fit_frames_the_content() {
        let mut e = editor_with_two_shapes();
        e.camera.zoom = 8.0;
        e.zoom_to_fit();

        // Every shape should now be inside the viewport.
        let visible = e.camera.visible_rect();
        for shape in e.page().shapes() {
            let bb = shape.bounding_box;
            assert!(
                bb.x >= visible.x0 - 1.0 && bb.right() <= visible.x1 + 1.0,
                "{bb:?} is outside {visible:?}"
            );
        }
    }

    #[test]
    fn zoom_to_selection_frames_just_the_selection() {
        let mut e = editor_with_two_shapes();
        let ids: Vec<String> = e.page().shapes().iter().map(|s| s.id.clone()).collect();
        e.selection = vec![ids[0].clone()];
        e.zoom_to_selection();

        let visible = e.camera.visible_rect();
        let selected_bb = e.page().shape(&ids[0]).unwrap().bounding_box;
        assert!(
            selected_bb.x >= visible.x0 - 1.0 && selected_bb.right() <= visible.x1 + 1.0,
            "{selected_bb:?} is outside {visible:?}"
        );
        // A single 100x60 shape in a 1000x700 viewport should end up zoomed
        // in well past what framing both shapes together would need.
        assert!(e.camera.zoom > 2.0, "zoom {} is not tight on the selection", e.camera.zoom);
    }

    #[test]
    fn zoom_to_selection_does_nothing_with_no_selection() {
        let mut e = editor_with_two_shapes();
        e.selection.clear();
        let origin = e.camera.origin;
        let zoom = e.camera.zoom;
        e.zoom_to_selection();
        assert_eq!(e.camera.origin, origin);
        assert_eq!(e.camera.zoom, zoom);
    }

    #[test]
    fn zoom_to_fit_on_an_empty_page_frames_the_sheet() {
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.zoom_to_fit();
        assert!(e.camera.zoom > 0.0 && e.camera.zoom.is_finite());
    }

    #[test]
    fn pages_can_be_added_and_switched() {
        let mut e = editor_with_two_shapes();
        assert_eq!(e.document().pages.len(), 1);

        e.add_page();
        assert_eq!(e.document().pages.len(), 2);
        assert_eq!(e.page_index, 1);
        assert_eq!(e.page().shapes().len(), 0, "the new page starts empty");

        e.select_page(0);
        assert_eq!(e.page().shapes().len(), 2);

        e.select_page(99);
        assert_eq!(e.page_index, 0, "an out-of-range page is ignored");
    }

    #[test]
    fn the_last_page_cannot_be_deleted() {
        let mut e = editor_with_two_shapes();
        assert_eq!(e.document().pages.len(), 1);
        assert!(!e.delete_page(0), "a document must keep at least one page");
        assert_eq!(e.document().pages.len(), 1);
        assert!(!e.can_undo(), "a refused delete records nothing");
    }

    #[test]
    fn deleting_a_page_keeps_the_index_in_range() {
        let mut e = editor_with_two_shapes();
        e.add_page();
        e.add_page();
        assert_eq!(e.page_index, 2);

        assert!(e.delete_page(2));
        assert_eq!(e.document().pages.len(), 2);
        assert_eq!(e.page_index, 1, "the index must not dangle past the end");
    }

    #[test]
    fn deleting_a_page_is_undoable() {
        let mut e = editor_with_two_shapes();
        e.add_page();
        e.delete_page(1);
        assert_eq!(e.document().pages.len(), 1);
        e.undo();
        assert_eq!(e.document().pages.len(), 2);
    }

    #[test]
    fn duplicating_a_page_copies_its_contents() {
        let mut e = editor_with_two_shapes();
        e.duplicate_page(0);

        assert_eq!(e.document().pages.len(), 2);
        assert_eq!(e.page_index, 1, "the copy becomes current");
        assert_eq!(e.page().shapes().len(), 2);
        assert!(e.page().title.ends_with("copy"));
    }

    #[test]
    fn a_duplicated_page_uses_fresh_ids() {
        let mut e = editor_with_two_shapes();
        e.duplicate_page(0);

        let mut ids: Vec<&str> = e
            .document()
            .pages
            .iter()
            .flat_map(|p| p.shapes().iter().map(|s| s.id.as_str()))
            .collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "ids collided across pages");
    }

    #[test]
    fn a_duplicated_pages_connectors_point_within_the_copy() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        e.duplicate_page(0);
        let copy = e.page();
        let line = &copy.lines()[0];
        for endpoint in [&line.endpoint1, &line.endpoint2] {
            let id = endpoint.shape_id().expect("attached");
            assert!(
                copy.shape(id).is_some(),
                "connector points at {id}, which is not on the copied page"
            );
        }
    }

    #[test]
    fn a_duplicated_page_validates() {
        let mut e = editor_with_two_shapes();
        e.duplicate_page(0);
        assert_eq!(
            e.package.validate(),
            vec![],
            "a duplicated page must produce a clean document"
        );
    }

    #[test]
    fn a_page_title_can_be_edited() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let area = kurbo::Rect::new(0.0, 0.0, 96.0, 26.0);

        assert!(e.begin_page_title_edit(0, area, Color::WHITE, &mut tcx.ctx()));
        assert_eq!(e.editing_page_title(), Some(0));
        type_text(&mut e, &mut tcx, "Renamed");
        e.commit_text_edit();

        assert_eq!(e.document().pages[0].title, "Renamed");
    }

    #[test]
    fn cancelling_a_rename_keeps_the_old_title() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let before = e.page().title.clone();

        e.begin_page_title_edit(0, kurbo::Rect::new(0.0, 0.0, 96.0, 26.0), Color::WHITE, &mut tcx.ctx());
        type_text(&mut e, &mut tcx, "Discarded");
        e.cancel_text_edit();

        assert_eq!(e.document().pages[0].title, before);
    }

    #[test]
    fn an_emptied_page_title_is_left_alone() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let before = e.page().title.clone();

        e.begin_page_title_edit(0, kurbo::Rect::new(0.0, 0.0, 96.0, 26.0), Color::WHITE, &mut tcx.ctx());
        e.text_command(TextCommand::SelectAll, &mut tcx.ctx());
        e.text_command(TextCommand::Backspace, &mut tcx.ctx());
        e.commit_text_edit();

        assert_eq!(
            e.document().pages[0].title,
            before,
            "a nameless tab would be unclickable"
        );
    }

    #[test]
    fn title_marks_unsaved_changes() {
        let mut e = Editor::new();
        assert_eq!(e.title(), "Untitled — DiagramCraft");
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        assert!(e.title().starts_with("• "), "got {}", e.title());
    }

    #[test]
    fn save_and_open_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join("dc-editor-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lucid");

        let mut e = editor_with_two_shapes();
        e.save_path(&path).unwrap();
        assert!(!e.dirty, "saving clears the dirty flag");

        let mut reopened = Editor::new();
        reopened.resize_viewport(1000.0, 700.0);
        reopened.open_path(&path).unwrap();

        assert_eq!(reopened.page().shapes().len(), 2);
        assert!(!reopened.dirty);
        assert!(!reopened.can_undo(), "a freshly opened document has no history");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_a_missing_file_reports_an_error() {
        let mut e = Editor::new();
        assert!(e.open_path("/definitely/not/a/real/path.lucid").is_err());
    }

    #[test]
    fn new_document_resets_everything() {
        let mut e = editor_with_two_shapes();
        e.select_all();
        e.new_document();

        assert_eq!(e.page().shapes().len(), 0);
        assert!(e.selection.is_empty());
        assert!(!e.can_undo());
        assert!(!e.dirty);
        assert_eq!(e.camera.zoom, 1.0);
    }

    #[test]
    fn new_document_from_template_loads_its_content_and_resets_state() {
        let mut e = editor_with_two_shapes();
        e.select_all();
        e.path = Some(std::path::PathBuf::from("/tmp/whatever.lucid"));

        let mut template = Document::new();
        template.pages[0].add_shape(Shape::new(
            "from-template",
            ShapeType::Rectangle,
            BoundingBox::new(0.0, 0.0, 50.0, 50.0),
        ));
        e.new_document_from_template(template, "Basic Flowchart");

        assert_eq!(e.page().shapes().len(), 1);
        assert_eq!(e.page().shapes()[0].id, "from-template");
        assert!(e.selection.is_empty());
        assert!(!e.can_undo());
        assert!(!e.dirty);
        assert_eq!(e.camera.zoom, 1.0);
        assert!(e.path.is_none(), "a template is not the file it came from");
        assert_eq!(e.status, "New document from Basic Flowchart");
    }

    #[test]
    fn overlay_reports_the_active_gesture() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.pointer_down(Point::new(50.0, 50.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(300.0, 300.0), mods(), &mut tcx.ctx());

        let overlay = e.overlay();
        assert!(overlay.marquee.is_some(), "a marquee drag should be visible");

        e.pointer_up(Point::new(300.0, 300.0), mods(), &mut tcx.ctx());
        assert!(e.overlay().marquee.is_none());
    }

    // -------------------------------------------------------- text editing

    fn type_text(e: &mut Editor, tcx: &mut TestCx, s: &str) {
        e.text_command(TextCommand::Insert(s.to_string()), &mut tcx.ctx());
    }

    #[test]
    fn double_clicking_a_shape_opens_its_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        assert!(e.is_editing(), "a double click should start editing");
        assert_eq!(e.editing_id(), Some(e.page().shapes()[0].id.as_str()));
    }

    #[test]
    fn a_single_click_does_not_open_the_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        assert!(!e.is_editing());
    }

    #[test]
    fn typing_and_committing_writes_the_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        type_text(&mut e, &mut tcx, "Start here");
        e.commit_text_edit();

        assert!(!e.is_editing());
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "Start here");
        assert!(e.dirty);
    }

    #[test]
    fn committing_a_label_is_one_undo_step() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        for ch in ["a", "b", "c", "d"] {
            type_text(&mut e, &mut tcx, ch);
        }
        e.commit_text_edit();
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "abcd");

        e.undo();
        assert_eq!(
            e.page().shape(&id).unwrap().text_str(),
            "",
            "one undo should revert the whole edit"
        );
    }

    #[test]
    fn cancelling_leaves_the_label_untouched() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.set_selection_text_for_test(&id, "original");
        e.dirty = false;
        e.history.clear();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        type_text(&mut e, &mut tcx, "discard me");
        e.cancel_text_edit();

        assert_eq!(e.page().shape(&id).unwrap().text_str(), "original");
        assert!(!e.can_undo(), "a cancelled edit records nothing");
    }

    #[test]
    fn committing_without_changes_records_nothing() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        e.commit_text_edit();
        assert!(!e.can_undo(), "opening and closing a label is not an edit");
    }

    #[test]
    fn clicking_away_commits_the_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        type_text(&mut e, &mut tcx, "committed");
        // A click on empty canvas.
        e.pointer_down(Point::new(800.0, 600.0), mods(), false, 1, &mut tcx.ctx());

        assert!(!e.is_editing());
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "committed");
    }

    #[test]
    fn clicking_inside_the_label_moves_the_caret_instead_of_the_shape() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        let before = e.page().shape(&id).unwrap().bounding_box;

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        e.pointer_down(Point::new(140.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(300.0, 300.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(300.0, 300.0), mods(), &mut tcx.ctx());

        assert!(e.is_editing(), "still editing");
        let after = e.page().shape(&id).unwrap().bounding_box;
        assert_eq!((after.x, after.y), (before.x, before.y), "the shape must not move");
    }

    #[test]
    fn enter_opens_the_selected_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.selection = vec![id.clone()];

        assert!(e.begin_text_edit_on_selection(&mut tcx.ctx()));
        assert_eq!(e.editing_id(), Some(id.as_str()));
    }

    #[test]
    fn shapes_that_reject_text_cannot_be_edited() {
        let mut tcx = TestCx::new();
        let mut e = Editor::new();
        e.resize_viewport(800.0, 600.0);
        let id = e.create_shape(ShapeType::Hotspot, BoundingBox::new(0.0, 0.0, 80.0, 40.0));
        e.selection = vec![id];

        assert!(!e.begin_text_edit_on_selection(&mut tcx.ctx()));
        assert!(!e.is_editing());
    }

    #[test]
    fn double_clicking_a_connector_opens_a_label_for_it() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        let line_id = e.page().lines()[0].id.clone();

        assert!(e.begin_edit_for(&line_id, true, &mut tcx.ctx()));
        type_text(&mut e, &mut tcx, "yes");
        e.commit_text_edit();

        let labels = e.page().line(&line_id).unwrap().labels().to_vec();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].text, "yes");
    }

    #[test]
    fn the_overlay_names_the_item_being_edited() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        assert_eq!(e.overlay().editing, None);

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        assert!(e.overlay().editing.is_some(), "the renderer must know to hide the static text");
    }

    #[test]
    fn emptying_a_label_removes_the_text_entirely() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.set_selection_text_for_test(&id, "remove me");

        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 2, &mut tcx.ctx());
        e.text_command(TextCommand::SelectAll, &mut tcx.ctx());
        e.text_command(TextCommand::Backspace, &mut tcx.ctx());
        e.commit_text_edit();

        assert!(e.page().shape(&id).unwrap().text.is_none());
    }

    // ------------------------------------------------------- text styling

    fn labelled_editor() -> Editor {
        let mut e = editor_with_two_shapes();
        let id = e.page().shapes()[0].id.clone();
        e.set_selection_text_for_test(&id, "Label");
        e.selection = vec![id];
        e.dirty = false;
        e.history.clear();
        e
    }

    fn label_style(e: &Editor) -> dc_model::TextStyle {
        let id = &e.selection[0];
        let text = e.page().shape(id).unwrap().text_str().to_string();
        dc_model::markup::parse(&text, dc_model::TextStyle::default()).runs[0]
            .style
            .clone()
    }

    #[test]
    fn bold_toggles_on_and_off() {
        let mut e = labelled_editor();
        assert!(!label_style(&e).bold);

        e.toggle_text_style(TextToggle::Bold);
        assert!(label_style(&e).bold, "first press turns it on");

        e.toggle_text_style(TextToggle::Bold);
        assert!(!label_style(&e).bold, "second press turns it off");
    }

    #[test]
    fn each_character_style_is_independent() {
        let mut e = labelled_editor();
        for toggle in [
            TextToggle::Bold,
            TextToggle::Italic,
            TextToggle::Underline,
            TextToggle::Strikethrough,
        ] {
            e.toggle_text_style(toggle);
        }
        let style = label_style(&e);
        assert!(style.bold && style.italic && style.underline && style.strikethrough);
    }

    #[test]
    fn a_mixed_selection_turns_everything_on_first() {
        let mut e = editor_with_two_shapes();
        let a = e.page().shapes()[0].id.clone();
        let b = e.page().shapes()[1].id.clone();
        e.set_selection_text_for_test(&a, "<b>already bold</b>");
        e.set_selection_text_for_test(&b, "plain");
        e.selection = vec![a.clone(), b.clone()];

        e.toggle_text_style(TextToggle::Bold);
        for id in [&a, &b] {
            let text = e.page().shape(id).unwrap().text_str().to_string();
            assert!(
                dc_model::markup::all_runs(&text, &dc_model::TextStyle::default(), |s| s.bold),
                "{id} should be bold"
            );
        }
    }

    #[test]
    fn text_size_scales_and_is_clamped() {
        let mut e = labelled_editor();
        let before = label_style(&e).font_size_pt;

        e.scale_text_size(1.5);
        assert!(label_style(&e).font_size_pt > before);

        // Repeated enlargement must stop at a usable maximum.
        for _ in 0..60 {
            e.scale_text_size(1.5);
        }
        assert!(label_style(&e).font_size_pt <= 144.0);

        for _ in 0..80 {
            e.scale_text_size(0.5);
        }
        assert!(label_style(&e).font_size_pt >= 4.0);
    }

    #[test]
    fn text_color_and_alignment_apply() {
        let mut e = labelled_editor();
        e.set_text_color(Color::rgb(0xdc, 0x26, 0x26));
        assert_eq!(label_style(&e).color, Color::rgb(0xdc, 0x26, 0x26));

        e.set_text_align(dc_model::TextAlign::Left);
        let text = e.page().shape(&e.selection[0]).unwrap().text_str().to_string();
        let parsed = dc_model::markup::parse(&text, dc_model::TextStyle::default());
        assert_eq!(parsed.align, dc_model::TextAlign::Left);

        e.set_vertical_align(dc_model::VerticalAlign::Top);
        let text = e.page().shape(&e.selection[0]).unwrap().text_str().to_string();
        let parsed = dc_model::markup::parse(&text, dc_model::TextStyle::default());
        assert_eq!(parsed.vertical_align, dc_model::VerticalAlign::Top);
    }

    #[test]
    fn font_family_applies_to_the_selection() {
        let mut e = labelled_editor();
        assert_eq!(e.selection_font_family(), Some("Liberation Sans".to_string()));

        e.set_font_family("Georgia");
        // The markup layer round-trips CSS values in lowercase; font lookup
        // is case-insensitive, so this is still the font that was asked for.
        assert_eq!(label_style(&e).font_family, "georgia");
        assert_eq!(e.selection_font_family(), Some("georgia".to_string()));
    }

    #[test]
    fn selection_font_family_reports_none_when_mixed() {
        let mut e = labelled_editor();
        let a = e.selection[0].clone();
        e.set_font_family("Georgia");

        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(200.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&b) {
            shape.text = Some(dc_model::Text::new("other"));
        }
        e.selection = vec![a];
        assert_eq!(e.selection_font_family(), Some("georgia".to_string()));

        e.selection.push(b);
        assert_eq!(
            e.selection_font_family(),
            None,
            "the two shapes disagree, so nothing is reported as shared"
        );
    }

    // ----------------------------------------------------------------- link

    #[test]
    fn setting_and_clearing_a_link() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        assert_eq!(e.selection_link(), None);

        e.set_link_url("https://example.com");
        assert_eq!(
            e.selection_link(),
            Some(dc_model::Action::Url { url: "https://example.com".into(), new_window: None })
        );

        e.clear_link();
        assert_eq!(e.selection_link(), None);
    }

    #[test]
    fn setting_a_link_replaces_any_existing_one() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id.clone()];
        e.set_link_url("https://first.example");
        e.set_link_url("https://second.example");

        let shape = e.page().shape(&id).unwrap();
        assert_eq!(shape.actions.as_ref().map(Vec::len), Some(1), "not accumulating duplicates");
        assert_eq!(
            e.selection_link(),
            Some(dc_model::Action::Url { url: "https://second.example".into(), new_window: None })
        );
    }

    #[test]
    fn a_link_needs_exactly_one_shape_selected() {
        let mut e = Editor::new();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(50.0, 0.0, 10.0, 10.0));
        e.selection = vec![a, b];
        e.set_link_url("https://example.com");
        assert_eq!(e.selection_link(), None, "a multi-selection has no single link to report");
    }

    // ----------------------------------------------------------- custom data

    #[test]
    fn adding_and_editing_custom_data() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id];
        assert!(e.selection_custom_data().is_empty());

        e.add_custom_data();
        assert_eq!(e.selection_custom_data().len(), 1);

        e.set_custom_data_key(0, "owner");
        e.set_custom_data_value(0, "ops");
        let entries = e.selection_custom_data();
        assert_eq!(entries[0].key, "owner");
        assert_eq!(entries[0].value, "ops");
    }

    #[test]
    fn removing_a_custom_data_entry() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        e.selection = vec![id.clone()];
        e.add_custom_data();
        e.set_custom_data_key(0, "a");
        e.add_custom_data();
        e.set_custom_data_key(1, "b");

        e.remove_custom_data(0);
        let entries = e.selection_custom_data();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "b");

        e.remove_custom_data(0);
        assert!(e.selection_custom_data().is_empty());
        assert!(
            e.page().shape(&id).unwrap().custom_data.is_none(),
            "an emptied list should not linger as Some(vec![])"
        );
    }

    #[test]
    fn custom_data_needs_exactly_one_shape_selected() {
        let mut e = Editor::new();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(50.0, 0.0, 10.0, 10.0));
        e.selection = vec![a, b];
        e.add_custom_data();
        assert!(e.selection_custom_data().is_empty());
    }

    // ----------------------------------------------------------------layers

    #[test]
    fn adding_and_renaming_layers() {
        let mut e = Editor::new();
        assert!(e.layers().is_empty());

        let id = e.add_layer();
        assert_eq!(e.layers().len(), 1);
        assert_eq!(e.layers()[0].title.as_deref(), Some("Layer 1"));

        e.rename_layer(&id, "Background");
        assert_eq!(e.layers()[0].title.as_deref(), Some("Background"));
    }

    #[test]
    fn toggling_visibility_and_lock() {
        let mut e = Editor::new();
        let id = e.add_layer();
        assert!(e.layers()[0].is_visible());
        assert!(!e.layers()[0].is_locked());

        e.toggle_layer_visibility(&id);
        assert!(!e.layers()[0].is_visible());
        e.toggle_layer_visibility(&id);
        assert!(e.layers()[0].is_visible());

        e.toggle_layer_lock(&id);
        assert!(e.layers()[0].is_locked());
    }

    #[test]
    fn deleting_a_layer_leaves_its_shapes_alone() {
        let mut e = Editor::new();
        let shape = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let layer = e.add_layer();
        e.selection = vec![shape.clone()];
        e.move_selection_to_layer(&layer);
        assert!(e.layer_of(&shape).is_some());

        e.delete_layer(&layer);
        assert!(e.layers().is_empty());
        assert!(e.page().shape(&shape).is_some(), "the shape itself must survive");
        assert!(e.layer_of(&shape).is_none());
    }

    #[test]
    fn moving_layers_up_and_down() {
        let mut e = Editor::new();
        let a = e.add_layer();
        let b = e.add_layer();
        let c = e.add_layer();
        assert_eq!(layer_ids(&e), vec![a.clone(), b.clone(), c.clone()]);

        e.move_layer(&b, true);
        assert_eq!(layer_ids(&e), vec![b.clone(), a.clone(), c.clone()]);

        e.move_layer(&b, false);
        assert_eq!(layer_ids(&e), vec![a.clone(), b.clone(), c.clone()]);

        // Already at an edge: nothing moves, nothing panics.
        e.move_layer(&a, true);
        assert_eq!(layer_ids(&e), vec![a.clone(), b.clone(), c.clone()]);
        e.move_layer(&c, false);
        assert_eq!(layer_ids(&e), vec![a, b, c]);
    }

    fn layer_ids(e: &Editor) -> Vec<String> {
        e.layers().iter().map(|l| l.id.clone()).collect()
    }

    #[test]
    fn moving_the_selection_between_layers() {
        let mut e = Editor::new();
        let shape = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let first = e.add_layer();
        let second = e.add_layer();
        e.selection = vec![shape.clone()];

        e.move_selection_to_layer(&first);
        assert_eq!(e.layer_of(&shape).map(|l| l.id.clone()), Some(first.clone()));

        e.move_selection_to_layer(&second);
        assert_eq!(e.layer_of(&shape).map(|l| l.id.clone()), Some(second));
        assert!(
            !e.layers().iter().find(|l| l.id == first).unwrap().items().contains(&shape),
            "moving to a new layer should remove it from the old one"
        );
    }

    // ----------------------------------------------------- find and replace

    #[test]
    fn finding_matches_across_shapes_and_lines() {
        let mut e = Editor::new();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&a) {
            shape.text = Some(dc_model::Text::new("Approval step"));
        }
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(200.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&b) {
            shape.text = Some(dc_model::Text::new("Nothing relevant"));
        }

        let matches = e.find_matches("approval");
        assert_eq!(matches, vec![a]);
    }

    #[test]
    fn find_is_case_insensitive_and_ignores_markup() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("<b>APPROVAL</b> step"));
        }
        assert_eq!(e.find_matches("approval"), vec![id]);
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let mut e = Editor::new();
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        assert!(e.find_matches("").is_empty());
        assert!(e.find_matches("   ").is_empty());
    }

    #[test]
    fn selecting_and_framing_a_match() {
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(500.0, 500.0, 100.0, 40.0));
        e.camera.zoom = 8.0;

        e.select_and_frame(&id);
        assert_eq!(e.selection, vec![id.clone()]);
        let visible = e.camera.visible_rect();
        let bb = e.page().shape(&id).unwrap().bounding_box;
        assert!(bb.x >= visible.x0 - 1.0 && bb.right() <= visible.x1 + 1.0);
    }

    #[test]
    fn replacing_the_first_match_in_a_label() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("Approve the approval"));
        }

        e.replace_in(&id, "approval", "request");
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "Approve the request");
    }

    #[test]
    fn replace_matches_ascii_case_insensitively() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("APPROVAL pending"));
        }

        e.replace_in(&id, "approval", "request");
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "request pending");
    }

    #[test]
    fn replacing_a_query_with_no_match_does_nothing() {
        let mut e = Editor::new();
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        if let Some(shape) = e.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("hello"));
        }

        e.replace_in(&id, "goodbye", "hi");
        assert_eq!(e.page().shape(&id).unwrap().text_str(), "hello");
    }

    #[test]
    fn replacing_a_line_label() {
        let mut e = Editor::new();
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 50.0, 40.0));
        let b = e.create_shape(ShapeType::Rectangle, BoundingBox::new(200.0, 0.0, 50.0, 40.0));
        e.create_connector(
            Some(a),
            BoundingBox::new(0.0, 0.0, 50.0, 40.0).center(),
            Some(b),
            BoundingBox::new(200.0, 0.0, 50.0, 40.0).center(),
        );
        let line_id = e.page().lines()[0].id.clone();
        if let Some(line) = e.package.document.pages[0].line_mut(&line_id) {
            line.text = Some(vec![dc_model::LineLabel::new("approval needed")]);
        }

        assert_eq!(e.find_matches("approval"), vec![line_id.clone()]);
        e.replace_in(&line_id, "approval", "sign-off");
        assert_eq!(
            e.page().line(&line_id).unwrap().labels()[0].text,
            "sign-off needed"
        );
    }

    #[test]
    fn styling_preserves_the_text_itself() {
        let mut e = labelled_editor();
        e.toggle_text_style(TextToggle::Bold);
        e.scale_text_size(1.4);
        e.set_text_color(Color::rgb(1, 2, 3));

        let text = e.page().shape(&e.selection[0]).unwrap().text_str().to_string();
        assert_eq!(
            dc_model::markup::parse(&text, dc_model::TextStyle::default()).plain(),
            "Label"
        );
    }

    #[test]
    fn styling_is_undoable() {
        let mut e = labelled_editor();
        e.toggle_text_style(TextToggle::Bold);
        assert!(label_style(&e).bold);
        e.undo();
        assert!(!label_style(&e).bold);
    }

    #[test]
    fn styling_an_empty_label_does_nothing() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];
        e.toggle_text_style(TextToggle::Bold);
        assert!(e.page().shapes()[0].text.is_none());
    }

    #[test]
    fn styling_with_nothing_selected_is_a_no_op() {
        let mut e = labelled_editor();
        e.selection.clear();
        e.toggle_text_style(TextToggle::Bold);
        assert!(!e.can_undo());
    }

    #[test]
    fn connector_labels_can_be_styled_too() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        let line_id = e.page().lines()[0].id.clone();
        if let Some(line) = e.page_mut().line_mut(&line_id) {
            line.text = Some(vec![dc_model::LineLabel::new("yes")]);
        }
        e.selection = vec![line_id.clone()];
        e.toggle_text_style(TextToggle::Bold);

        let label = e.page().line(&line_id).unwrap().labels()[0].text.clone();
        assert!(
            dc_model::markup::all_runs(&label, &dc_model::TextStyle::default(), |s| s.bold),
            "got {label:?}"
        );
    }

    // ----------------------------------------------------------- clipboard

    #[test]
    fn copying_captures_the_selected_shapes() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];

        let fragment = e.copy_fragment();
        assert_eq!(fragment.shapes.len(), 1);
        assert!(fragment.lines.is_empty());
    }

    #[test]
    fn copying_includes_connectors_between_copied_shapes() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        e.select_all();
        let fragment = e.copy_fragment();
        assert_eq!(fragment.shapes.len(), 2);
        assert_eq!(fragment.lines.len(), 1, "the connector should come along");
    }

    #[test]
    fn copying_excludes_connectors_to_uncopied_shapes() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        // Only one end of the connector is selected.
        e.selection = vec![e.page().shapes()[0].id.clone()];
        assert!(
            e.copy_fragment().lines.is_empty(),
            "a half-copied connector would paste dangling"
        );
    }

    #[test]
    fn pasting_adds_offset_copies_with_fresh_ids() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];
        let original = e.page().shapes()[0].clone();

        let fragment = e.copy_fragment();
        e.paste_fragment(&fragment, 10.0);

        assert_eq!(e.page().shapes().len(), 3);
        let pasted = e.page().shape(&e.selection[0]).unwrap();
        assert_ne!(pasted.id, original.id, "the copy needs its own id");
        assert_eq!(pasted.bounding_box.x, original.bounding_box.x + 10.0);
        assert_eq!(pasted.text_str(), original.text_str());
    }

    #[test]
    fn pasting_twice_does_not_collide() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];
        let fragment = e.copy_fragment();

        e.paste_fragment(&fragment, 10.0);
        e.paste_fragment(&fragment, 10.0);
        assert_eq!(e.page().shapes().len(), 4);

        let mut ids: Vec<&str> = e.page().shapes().iter().map(|s| s.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "ids must stay unique");
    }

    #[test]
    fn pasted_connectors_point_at_the_pasted_shapes() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        e.select_all();
        let fragment = e.copy_fragment();
        let originals: Vec<String> =
            e.page().shapes().iter().map(|s| s.id.clone()).collect();
        e.paste_fragment(&fragment, 20.0);

        let pasted_line = e.page().lines().last().unwrap();
        for endpoint in [&pasted_line.endpoint1, &pasted_line.endpoint2] {
            let id = endpoint.shape_id().expect("attached");
            assert!(
                !originals.contains(&id.to_string()),
                "the copy should attach to the pasted shapes, not {id}"
            );
            assert!(e.page().shape(id).is_some(), "and that shape must exist");
        }
    }

    #[test]
    fn pasting_an_empty_fragment_changes_nothing() {
        let mut e = editor_with_two_shapes();
        let before = e.page().shapes().len();
        e.paste_fragment(&crate::clipboard::Fragment::default(), 10.0);
        assert_eq!(e.page().shapes().len(), before);
        assert!(!e.can_undo());
    }

    #[test]
    fn pasting_is_undoable() {
        let mut e = editor_with_two_shapes();
        e.selection = vec![e.page().shapes()[0].id.clone()];
        let fragment = e.copy_fragment();
        e.paste_fragment(&fragment, 10.0);
        assert_eq!(e.page().shapes().len(), 3);

        e.undo();
        assert_eq!(e.page().shapes().len(), 2);
    }

    #[test]
    fn hover_tracks_the_pointer_when_not_dragging() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.pointer_move(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());
        assert!(e.hovered.is_some());

        e.pointer_move(Point::new(900.0, 650.0), mods(), &mut tcx.ctx());
        assert!(e.hovered.is_none());
    }

    // ------------------------------------------------------ page reordering

    /// An editor with three pages named A, B, and C.
    fn editor_with_three_pages() -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(800.0, 600.0);
        e.package.document.pages[0].title = "A".into();
        e.add_page();
        e.package.document.pages[1].title = "B".into();
        e.add_page();
        e.package.document.pages[2].title = "C".into();
        e.page_index = 0;
        e.dirty = false;
        e.history.clear();
        e
    }

    fn titles(e: &Editor) -> Vec<String> {
        e.document().pages.iter().map(|p| p.title.clone()).collect()
    }

    #[test]
    fn a_page_can_be_dragged_later_in_the_order() {
        let mut e = editor_with_three_pages();
        assert!(e.move_page(0, 2));
        assert_eq!(titles(&e), vec!["B", "C", "A"]);
    }

    #[test]
    fn a_page_can_be_dragged_earlier_in_the_order() {
        let mut e = editor_with_three_pages();
        assert!(e.move_page(2, 0));
        assert_eq!(titles(&e), vec!["C", "A", "B"]);
    }

    #[test]
    fn moving_a_page_keeps_it_current() {
        let mut e = editor_with_three_pages();
        e.page_index = 0;
        e.move_page(0, 2);
        assert_eq!(e.page_index, 2);
        assert_eq!(e.page().title, "A", "the dragged page is still the one shown");
    }

    #[test]
    fn moving_another_page_keeps_the_current_one_showing() {
        let mut e = editor_with_three_pages();
        e.page_index = 2; // C
        e.move_page(0, 1); // A moves between B and C
        assert_eq!(titles(&e), vec!["B", "A", "C"]);
        assert_eq!(e.page().title, "C", "the view should not jump");

        let mut back = editor_with_three_pages();
        back.page_index = 0; // A
        back.move_page(2, 0); // C jumps to the front
        assert_eq!(back.page().title, "A");
    }

    #[test]
    fn moving_a_page_is_one_undo_step() {
        let mut e = editor_with_three_pages();
        e.move_page(0, 2);
        assert_eq!(titles(&e), vec!["B", "C", "A"]);
        e.undo();
        assert_eq!(titles(&e), vec!["A", "B", "C"]);
    }

    #[test]
    fn a_move_that_changes_nothing_is_refused() {
        let mut e = editor_with_three_pages();
        assert!(!e.move_page(1, 1));
        assert!(!e.move_page(9, 0), "an index past the end moves nothing");
        assert!(!e.can_undo(), "and nothing was recorded");

        let mut single = Editor::new();
        assert!(!single.move_page(0, 0), "one page cannot be reordered");
    }

    #[test]
    fn a_target_past_the_end_lands_on_the_last_slot() {
        let mut e = editor_with_three_pages();
        assert!(e.move_page(0, 99));
        assert_eq!(titles(&e), vec!["B", "C", "A"]);
    }

    // ------------------------------------------------- per-selection styling

    /// An editor with one labeled, selected shape.
    fn editor_with_label(text: &str) -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(800.0, 600.0);
        let id = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 220.0, 80.0));
        e.set_selection_text_for_test(&id, text);
        e.selection = vec![id];
        e.dirty = false;
        e.history.clear();
        e
    }

    fn label_markup(e: &Editor) -> String {
        e.page().shapes()[0].text_str().to_string()
    }

    #[test]
    fn formatting_applies_to_the_selected_characters_only() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("hello world");
        e.begin_text_edit_on_selection(&mut tcx.ctx());

        // Select the first five characters, then bold them.
        e.text_command(TextCommand::TextStart { select: false }, &mut tcx.ctx());
        for _ in 0..5 {
            e.text_command(
                TextCommand::Right { select: true, word: false },
                &mut tcx.ctx(),
            );
        }
        e.toggle_text_style(TextToggle::Bold);
        e.commit_text_edit();

        let parsed = dc_model::markup::parse(&label_markup(&e), Default::default());
        let bold: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "hello", "got {:?}", label_markup(&e));
        assert_eq!(parsed.plain(), "hello world");
    }

    #[test]
    fn formatting_without_an_open_label_still_styles_the_whole_thing() {
        let mut e = editor_with_label("hello world");
        e.toggle_text_style(TextToggle::Bold);
        assert!(dc_model::markup::all_runs(
            &label_markup(&e),
            &Default::default(),
            |s| s.bold
        ));
    }

    #[test]
    fn text_color_applies_to_a_selection_inside_a_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("red blue");
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        e.text_command(TextCommand::TextStart { select: false }, &mut tcx.ctx());
        for _ in 0..3 {
            e.text_command(
                TextCommand::Right { select: true, word: false },
                &mut tcx.ctx(),
            );
        }
        e.set_text_color(Color::rgb(0xff, 0, 0));
        e.commit_text_edit();

        let parsed = dc_model::markup::parse(&label_markup(&e), Default::default());
        let red = parsed
            .runs
            .iter()
            .find(|r| r.style.color == Color::rgb(0xff, 0, 0))
            .expect("a red run");
        assert_eq!(red.text, "red");
    }

    #[test]
    fn size_changes_apply_to_a_selection_inside_a_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("small BIG");
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        e.text_command(TextCommand::TextEnd { select: false }, &mut tcx.ctx());
        for _ in 0..3 {
            e.text_command(
                TextCommand::Left { select: true, word: false },
                &mut tcx.ctx(),
            );
        }
        e.scale_text_size(2.0);
        e.commit_text_edit();

        let parsed = dc_model::markup::parse(&label_markup(&e), Default::default());
        let big = parsed.runs.iter().find(|r| r.text == "BIG").expect("a BIG run");
        assert!(big.style.font_size_pt > 15.0, "got {}", big.style.font_size_pt);
        let small = parsed.runs.iter().find(|r| r.text == "small ").unwrap();
        assert_eq!(small.style.font_size_pt, 10.0);
    }

    #[test]
    fn the_toggle_state_reports_the_selection() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("<b>bold</b> plain");
        e.begin_text_edit_on_selection(&mut tcx.ctx());

        // Everything selected, and the label is mixed.
        e.text_command(TextCommand::SelectAll, &mut tcx.ctx());
        assert!(!e.text_style_is_on(TextToggle::Bold));

        // Just the bold word.
        e.text_command(TextCommand::TextStart { select: false }, &mut tcx.ctx());
        for _ in 0..4 {
            e.text_command(
                TextCommand::Right { select: true, word: false },
                &mut tcx.ctx(),
            );
        }
        assert!(e.text_style_is_on(TextToggle::Bold));
    }

    #[test]
    fn the_toggle_state_reports_whole_labels_with_nothing_open() {
        let e = editor_with_label("<b>all bold</b>");
        assert!(e.text_style_is_on(TextToggle::Bold));
        assert!(!e.text_style_is_on(TextToggle::Italic));

        let mut mixed = editor_with_label("<b>bold</b> plain");
        assert!(!mixed.text_style_is_on(TextToggle::Bold));
        mixed.clear_selection();
        assert!(!mixed.text_style_is_on(TextToggle::Bold), "nothing selected, nothing on");
    }

    #[test]
    fn mixed_styling_survives_being_edited() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("<b>bold</b> plain");
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        // Append to the end, which used to flatten the whole label.
        e.text_command(TextCommand::TextEnd { select: false }, &mut tcx.ctx());
        e.text_command(TextCommand::Insert("!".into()), &mut tcx.ctx());
        e.commit_text_edit();

        let parsed = dc_model::markup::parse(&label_markup(&e), Default::default());
        assert_eq!(parsed.plain(), "bold plain!");
        let bold: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "bold", "got {:?}", label_markup(&e));
    }

    #[test]
    fn an_untouched_label_is_committed_verbatim() {
        let mut tcx = TestCx::new();
        let original = "<b>bold</b> and <i>italic</i>";
        let mut e = editor_with_label(original);
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        e.commit_text_edit();
        assert_eq!(label_markup(&e), original);
        assert!(!e.can_undo(), "looking at a label is not a change");
    }

    // ----------------------------------------------------------- arranging

    /// An editor with three ragged shapes of different sizes.
    fn editor_with_ragged_shapes() -> Editor {
        let mut e = Editor::new();
        e.resize_viewport(1000.0, 700.0);
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(10.0, 10.0, 100.0, 40.0));
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(200.0, 80.0, 60.0, 90.0));
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(400.0, 30.0, 80.0, 20.0));
        e.select_all();
        e.dirty = false;
        e.history.clear();
        e
    }

    fn boxes(e: &Editor) -> Vec<BoundingBox> {
        e.page().shapes().iter().map(|s| s.bounding_box).collect()
    }

    #[test]
    fn aligning_left_puts_every_shape_on_the_same_edge() {
        let mut e = editor_with_ragged_shapes();
        e.align_selection(Align::Left);
        let xs: Vec<f64> = boxes(&e).iter().map(|b| b.x).collect();
        assert_eq!(xs, vec![10.0, 10.0, 10.0]);
    }

    #[test]
    fn aligning_right_lines_up_the_far_edges() {
        let mut e = editor_with_ragged_shapes();
        e.align_selection(Align::Right);
        let rights: Vec<f64> = boxes(&e).iter().map(|b| b.right()).collect();
        assert_eq!(rights, vec![480.0, 480.0, 480.0]);
    }

    #[test]
    fn aligning_middle_lines_up_the_centres() {
        let mut e = editor_with_ragged_shapes();
        e.align_selection(Align::Middle);
        let centres: Vec<f64> = boxes(&e).iter().map(|b| b.center().y).collect();
        for y in &centres {
            assert!((y - centres[0]).abs() < 1e-9, "got {centres:?}");
        }
    }

    #[test]
    fn aligning_twice_changes_nothing_the_second_time() {
        let mut e = editor_with_ragged_shapes();
        e.align_selection(Align::Top);
        let once = boxes(&e);
        e.align_selection(Align::Top);
        assert_eq!(boxes(&e), once, "aligning should settle, not drift");
    }

    #[test]
    fn aligning_does_not_change_sizes() {
        let mut e = editor_with_ragged_shapes();
        let sizes: Vec<(f64, f64)> = boxes(&e).iter().map(|b| (b.w, b.h)).collect();
        e.align_selection(Align::Left);
        let after: Vec<(f64, f64)> = boxes(&e).iter().map(|b| (b.w, b.h)).collect();
        assert_eq!(sizes, after);
    }

    #[test]
    fn aligning_one_shape_does_nothing() {
        let mut e = editor_with_ragged_shapes();
        e.selection = vec![e.page().shapes()[1].id.clone()];
        let before = boxes(&e);
        e.align_selection(Align::Left);
        assert_eq!(boxes(&e), before);
        assert!(!e.can_undo(), "nothing happened, so nothing was recorded");
    }

    #[test]
    fn aligning_is_one_undo_step() {
        let mut e = editor_with_ragged_shapes();
        let before = boxes(&e);
        e.align_selection(Align::Left);
        assert_ne!(boxes(&e), before);
        e.undo();
        assert_eq!(boxes(&e), before);
    }

    #[test]
    fn distributing_equalizes_the_gaps() {
        let mut e = editor_with_ragged_shapes();
        e.distribute_selection(true);

        let mut sorted = boxes(&e);
        sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        let gaps: Vec<f64> = sorted
            .windows(2)
            .map(|pair| pair[1].x - pair[0].right())
            .collect();
        for gap in &gaps {
            assert!((gap - gaps[0]).abs() < 1e-6, "uneven gaps: {gaps:?}");
        }
    }

    #[test]
    fn distributing_leaves_the_outer_shapes_where_they_were() {
        let mut e = editor_with_ragged_shapes();
        let before = boxes(&e);
        e.distribute_selection(true);
        let after = boxes(&e);
        assert_eq!(after[0].x, before[0].x, "the leftmost should not move");
        assert_eq!(after[2].right(), before[2].right(), "nor the rightmost");
    }

    #[test]
    fn distributing_vertically_works_the_same_way() {
        let mut e = editor_with_ragged_shapes();
        e.distribute_selection(false);
        let mut sorted = boxes(&e);
        sorted.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap());
        let gaps: Vec<f64> = sorted
            .windows(2)
            .map(|pair| pair[1].y - pair[0].bottom())
            .collect();
        for gap in &gaps {
            assert!((gap - gaps[0]).abs() < 1e-6, "uneven gaps: {gaps:?}");
        }
    }

    #[test]
    fn distributing_fewer_than_three_shapes_is_refused() {
        let mut e = editor_with_ragged_shapes();
        e.selection.truncate(2);
        let before = boxes(&e);
        e.distribute_selection(true);
        assert_eq!(boxes(&e), before);
        assert!(e.status.contains("three"), "got {:?}", e.status);
    }

    #[test]
    fn matching_size_uses_the_first_selected_shape() {
        let mut e = editor_with_ragged_shapes();
        e.match_size(true, true);
        let sizes: Vec<(f64, f64)> = boxes(&e).iter().map(|b| (b.w, b.h)).collect();
        assert_eq!(sizes, vec![(100.0, 40.0), (100.0, 40.0), (100.0, 40.0)]);
    }

    #[test]
    fn matching_width_leaves_heights_alone() {
        let mut e = editor_with_ragged_shapes();
        let heights: Vec<f64> = boxes(&e).iter().map(|b| b.h).collect();
        e.match_size(true, false);

        let after = boxes(&e);
        assert!(after.iter().all(|b| b.w == 100.0));
        assert_eq!(after.iter().map(|b| b.h).collect::<Vec<_>>(), heights);
    }

    #[test]
    fn matching_does_not_move_anything() {
        let mut e = editor_with_ragged_shapes();
        let positions: Vec<(f64, f64)> = boxes(&e).iter().map(|b| (b.x, b.y)).collect();
        e.match_size(true, true);
        let after: Vec<(f64, f64)> = boxes(&e).iter().map(|b| (b.x, b.y)).collect();
        assert_eq!(positions, after);
    }

    // -------------------------------------------------------------- groups

    #[test]
    fn grouping_collects_the_selection() {
        let mut e = editor_with_ragged_shapes();
        e.group_selection();
        assert_eq!(e.page().groups().len(), 1);
        assert_eq!(e.page().groups()[0].item_ids().len(), 3);
        assert!(e.selection_is_grouped());
    }

    #[test]
    fn grouping_one_item_is_refused() {
        let mut e = editor_with_ragged_shapes();
        e.selection.truncate(1);
        e.group_selection();
        assert_eq!(e.page().groups().len(), 0);
    }

    #[test]
    fn clicking_one_member_selects_the_whole_group() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_ragged_shapes();
        e.group_selection();
        e.clear_selection();

        // Click inside the first shape.
        e.pointer_down(Point::new(40.0, 30.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(40.0, 30.0), mods(), &mut tcx.ctx());
        assert_eq!(e.selection.len(), 3, "a group moves as one");
    }

    #[test]
    fn dragging_a_group_moves_every_member() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_ragged_shapes();
        e.snap_enabled = false;
        e.snap_to_grid = false;
        e.group_selection();
        e.clear_selection();
        let before = boxes(&e);

        e.pointer_down(Point::new(40.0, 30.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(90.0, 30.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(90.0, 30.0), mods(), &mut tcx.ctx());

        for (after, original) in boxes(&e).iter().zip(&before) {
            assert!((after.x - original.x - 50.0).abs() < 1.0, "{after:?} vs {original:?}");
        }
    }

    #[test]
    fn ungrouping_breaks_the_group_apart() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_ragged_shapes();
        e.group_selection();
        e.ungroup_selection();
        assert_eq!(e.page().groups().len(), 0);

        e.clear_selection();
        e.pointer_down(Point::new(40.0, 30.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_up(Point::new(40.0, 30.0), mods(), &mut tcx.ctx());
        assert_eq!(e.selection.len(), 1, "the members are independent again");
    }

    #[test]
    fn regrouping_moves_members_out_of_their_old_group() {
        let mut e = editor_with_ragged_shapes();
        e.group_selection();
        let ids: Vec<String> = e.page().shapes().iter().map(|s| s.id.clone()).collect();

        // Group two of the three into a new group.
        e.selection = ids[..2].to_vec();
        e.group_selection();
        assert_eq!(e.page().groups().len(), 1, "a one-member group is dropped");
        assert_eq!(e.page().groups()[0].item_ids().len(), 2);
    }

    #[test]
    fn grouping_is_undoable() {
        let mut e = editor_with_ragged_shapes();
        e.group_selection();
        assert_eq!(e.page().groups().len(), 1);
        e.undo();
        assert_eq!(e.page().groups().len(), 0);
    }

    // ------------------------------------------------------------ selecting

    #[test]
    fn selecting_shapes_and_lines_separately() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        e.select_all_shapes();
        assert_eq!(e.selection.len(), 2);
        e.select_all_lines();
        assert_eq!(e.selection.len(), 1);
    }

    #[test]
    fn selecting_the_same_fill_gathers_matching_shapes() {
        let mut e = Editor::new();
        e.resize_viewport(800.0, 600.0);
        let red = Color::rgb(0xff, 0, 0);
        let a = e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        let b = e.create_shape(ShapeType::Decision, BoundingBox::new(30.0, 0.0, 10.0, 10.0));
        let c = e.create_shape(ShapeType::Rectangle, BoundingBox::new(60.0, 0.0, 10.0, 10.0));

        e.selection = vec![a.clone(), b.clone()];
        e.set_selection_fill(red);
        e.selection = vec![a.clone()];
        e.select_matching(SelectBy::FillColor);

        assert_eq!(e.selection.len(), 2, "two shapes are red");
        assert!(e.selection.contains(&a) && e.selection.contains(&b));
        assert!(!e.selection.contains(&c));
    }

    #[test]
    fn selecting_the_same_type_gathers_matching_shapes() {
        let mut e = Editor::new();
        e.resize_viewport(800.0, 600.0);
        e.create_shape(ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        e.create_shape(ShapeType::Decision, BoundingBox::new(30.0, 0.0, 10.0, 10.0));
        let third = e.create_shape(ShapeType::Rectangle, BoundingBox::new(60.0, 0.0, 10.0, 10.0));

        e.selection = vec![third];
        e.select_matching(SelectBy::ShapeType);
        assert_eq!(e.selection.len(), 2, "two rectangles");
    }

    #[test]
    fn selecting_matching_with_nothing_selected_does_nothing() {
        let mut e = editor_with_two_shapes();
        e.select_matching(SelectBy::FillColor);
        assert!(e.selection.is_empty());
    }

    // ------------------------------------------------ reading the selection

    #[test]
    fn the_selections_properties_are_reported_when_they_agree() {
        let mut e = editor_with_ragged_shapes();
        let red = Color::rgb(0xdc, 0x26, 0x26);
        e.set_selection_fill(Color::WHITE);
        e.set_selection_stroke(red, Some(3.0));
        e.set_selection_stroke_style(dc_model::StrokeStyle::Dashed);

        assert_eq!(e.selection_fill(), Some(Color::WHITE));
        assert_eq!(e.selection_stroke_color(), Some(red));
        assert_eq!(e.selection_stroke_width(), Some(3.0));
        assert_eq!(
            e.selection_stroke_style(),
            Some(dc_model::StrokeStyle::Dashed)
        );
    }

    #[test]
    fn a_selection_that_disagrees_reports_nothing() {
        let mut e = editor_with_ragged_shapes();
        let ids: Vec<String> = e.page().shapes().iter().map(|s| s.id.clone()).collect();

        e.selection = vec![ids[0].clone()];
        e.set_selection_fill(Color::WHITE);
        e.selection = vec![ids[1].clone()];
        e.set_selection_fill(Color::rgb(0xff, 0, 0));

        e.selection = ids;
        assert_eq!(e.selection_fill(), None, "two fills is no fill");
    }

    #[test]
    fn nothing_selected_reports_nothing() {
        let e = editor_with_ragged_shapes();
        let mut empty = e;
        empty.clear_selection();
        assert_eq!(empty.selection_fill(), None);
        assert_eq!(empty.selection_stroke_color(), None);
        assert_eq!(empty.selection_stroke_width(), None);
        assert_eq!(empty.selection_line_type(), None);
    }

    #[test]
    fn the_connector_routing_is_read_from_connectors_only() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());

        // A connector selected on its own reports its routing.
        assert_eq!(e.selection_line_type(), Some(LineType::Elbow));
        e.set_selection_line_type(LineType::Curved);
        assert_eq!(e.selection_line_type(), Some(LineType::Curved));

        // A shape has no routing, so a shape-only selection reports none.
        e.selection = vec![e.page().shapes()[0].id.clone()];
        assert_eq!(e.selection_line_type(), None);
    }

    #[test]
    fn the_dash_style_can_be_set_and_read_back() {
        let mut e = editor_with_two_shapes();
        e.select_all();
        for style in [
            dc_model::StrokeStyle::Dashed,
            dc_model::StrokeStyle::Dotted,
            dc_model::StrokeStyle::Solid,
        ] {
            e.set_selection_stroke_style(style);
            assert_eq!(e.selection_stroke_style(), Some(style));
        }
    }

    #[test]
    fn setting_the_dash_leaves_the_colour_and_weight_alone() {
        let mut e = editor_with_two_shapes();
        e.select_all();
        let green = Color::rgb(0x05, 0x96, 0x69);
        e.set_selection_stroke(green, Some(5.0));
        e.set_selection_stroke_style(dc_model::StrokeStyle::Dotted);

        assert_eq!(e.selection_stroke_color(), Some(green));
        assert_eq!(e.selection_stroke_width(), Some(5.0));
    }

    #[test]
    fn setting_the_dash_is_undoable() {
        let mut e = editor_with_two_shapes();
        e.select_all();
        e.set_selection_stroke_style(dc_model::StrokeStyle::Dashed);
        assert_eq!(e.selection_stroke_style(), Some(dc_model::StrokeStyle::Dashed));

        e.undo();
        assert_eq!(e.selection_stroke_style(), Some(dc_model::StrokeStyle::Solid));
    }

    #[test]
    fn the_text_colour_reports_the_highlighted_characters() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("red blue");
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        e.text_command(TextCommand::TextStart { select: false }, &mut tcx.ctx());
        for _ in 0..3 {
            e.text_command(
                TextCommand::Right { select: true, word: false },
                &mut tcx.ctx(),
            );
        }
        let red = Color::rgb(0xdc, 0x26, 0x26);
        e.set_text_color(red);

        assert_eq!(e.selection_text_color(), Some(red), "the highlighted run");

        e.text_command(TextCommand::SelectAll, &mut tcx.ctx());
        assert_eq!(e.selection_text_color(), None, "a mixed run reports none");
    }

    // ----------------------------------------------------------- endpoints

    /// An editor with one connector between two shapes, selected.
    fn editor_with_connector() -> Editor {
        let mut tcx = TestCx::new();
        let mut e = editor_with_two_shapes();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(150.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(450.0, 130.0), mods(), &mut tcx.ctx());
        e.dirty = false;
        e.history.clear();
        e
    }

    #[test]
    fn each_end_takes_its_own_terminator() {
        let mut e = editor_with_connector();
        e.set_endpoint_style(LineEnd::Start, EndpointStyle::ClosedCircle);
        e.set_endpoint_style(LineEnd::End, EndpointStyle::HollowArrow);

        let line = &e.page().lines()[0];
        assert_eq!(*line.endpoint1.style(), EndpointStyle::ClosedCircle);
        assert_eq!(*line.endpoint2.style(), EndpointStyle::HollowArrow);

        assert_eq!(
            e.selection_endpoint_style(LineEnd::Start),
            Some(EndpointStyle::ClosedCircle)
        );
        assert_eq!(
            e.selection_endpoint_style(LineEnd::End),
            Some(EndpointStyle::HollowArrow)
        );
    }

    #[test]
    fn reversing_a_connector_swaps_both_ends() {
        let mut e = editor_with_connector();
        e.set_endpoint_style(LineEnd::Start, EndpointStyle::None);
        e.set_endpoint_style(LineEnd::End, EndpointStyle::Arrow);

        let before = e.page().lines()[0].clone();
        e.swap_endpoints();
        let after = &e.page().lines()[0];

        assert_eq!(after.endpoint1, before.endpoint2);
        assert_eq!(after.endpoint2, before.endpoint1);
        assert_eq!(*after.endpoint1.style(), EndpointStyle::Arrow);
    }

    #[test]
    fn reversing_twice_puts_a_connector_back() {
        let mut e = editor_with_connector();
        let before = e.page().lines()[0].clone();
        e.swap_endpoints();
        e.swap_endpoints();
        assert_eq!(e.page().lines()[0], before);
    }

    #[test]
    fn endpoint_commands_ignore_a_shape_only_selection() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_endpoint_style(LineEnd::End, EndpointStyle::Arrow);
        assert!(!e.can_undo(), "nothing happened, so nothing was recorded");
        assert_eq!(e.selection_endpoint_style(LineEnd::End), None);
    }

    #[test]
    fn what_is_selected_is_reported_by_kind() {
        let mut e = editor_with_connector();
        assert!(e.selection_has_lines());
        assert!(!e.selection_has_shapes());

        e.select_all_shapes();
        assert!(!e.selection_has_lines());
        assert!(e.selection_has_shapes());

        e.select_all();
        assert!(e.selection_has_lines() && e.selection_has_shapes());
    }

    // ------------------------------------------------------ shape geometry

    #[test]
    fn opacity_is_set_and_read_as_a_percentage() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_opacity(50.0);

        assert_eq!(e.selection_opacity(), Some(50.0));
        assert_eq!(e.page().shapes()[0].opacity_factor(), 0.5);
    }

    #[test]
    fn full_opacity_is_stored_as_no_value_at_all() {
        // The format's own default is opaque, so writing a hundred would add
        // a field every shape would otherwise not carry.
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_opacity(40.0);
        assert!(e.page().shapes()[0].opacity.is_some());

        e.set_opacity(100.0);
        assert_eq!(e.page().shapes()[0].opacity, None);
        assert_eq!(e.selection_opacity(), Some(100.0), "and still reads as full");
    }

    #[test]
    fn opacity_is_clamped_to_the_published_range() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_opacity(-20.0);
        assert_eq!(e.selection_opacity(), Some(0.0));
    }

    #[test]
    fn corner_rounding_is_stored_at_the_formats_doubled_scale() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_corner_rounding(10.0);

        assert_eq!(e.selection_corner_rounding(), Some(10.0));
        assert_eq!(
            e.page().shapes()[0].resolved_style().rounding,
            Some(20.0),
            "the format stores double the pixel value"
        );
    }

    #[test]
    fn square_corners_drop_the_field_rather_than_storing_zero() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.set_corner_rounding(10.0);
        e.set_corner_rounding(0.0);
        assert_eq!(e.page().shapes()[0].resolved_style().rounding, None);
        assert_eq!(e.selection_corner_rounding(), Some(0.0));
    }

    #[test]
    fn rotating_accumulates_and_wraps() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();

        e.rotate_selection(90.0);
        assert_eq!(e.page().shapes()[0].bounding_box.rotation_degrees(), 90.0);

        e.rotate_selection(90.0);
        assert_eq!(e.page().shapes()[0].bounding_box.rotation_degrees(), 180.0);

        e.rotate_selection(180.0);
        assert_eq!(
            e.page().shapes()[0].bounding_box.rotation_degrees(),
            0.0,
            "a full turn comes back to square"
        );
    }

    #[test]
    fn rotating_left_is_the_opposite_of_rotating_right() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        e.rotate_selection(90.0);
        e.rotate_selection(-90.0);
        assert_eq!(e.page().shapes()[0].bounding_box.rotation_degrees(), 0.0);
    }

    #[test]
    fn flipping_mirrors_the_arrangement_about_its_own_box() {
        let mut e = editor_with_ragged_shapes();
        let before = boxes(&e);
        let bounds = e.selection_bounds().unwrap();

        e.flip_selection(true);
        let after = boxes(&e);

        // The selection as a whole occupies the same span.
        let span = after
            .iter()
            .map(|b| b.aabb())
            .reduce(|a, b| a.union(&b))
            .unwrap();
        assert!((span.x - bounds.x).abs() < 1e-9);
        assert!((span.right() - bounds.right()).abs() < 1e-9);

        // The leftmost shape is now where the rightmost one was.
        assert!(after[0].x > before[0].x, "the first shape should have moved right");
    }

    #[test]
    fn flipping_twice_puts_everything_back() {
        let mut e = editor_with_ragged_shapes();
        let before = boxes(&e);
        e.flip_selection(true);
        e.flip_selection(true);
        assert_eq!(boxes(&e), before);

        e.flip_selection(false);
        e.flip_selection(false);
        assert_eq!(boxes(&e), before);
    }

    #[test]
    fn ordering_moves_one_step_or_all_the_way() {
        let mut e = editor_with_two_shapes();
        let first = e.page().shapes()[0].id.clone();
        e.selection = vec![first.clone()];

        e.nudge_z_order(true);
        assert_eq!(e.page().shape(&first).unwrap().resolved_z_index(), 1);
        e.nudge_z_order(true);
        assert_eq!(e.page().shape(&first).unwrap().resolved_z_index(), 2);
        e.nudge_z_order(false);
        assert_eq!(e.page().shape(&first).unwrap().resolved_z_index(), 1);

        // And the whole-way commands still go past everything.
        e.reorder_selection(false);
        let lowest = e
            .page()
            .shapes()
            .iter()
            .map(|s| s.resolved_z_index())
            .min()
            .unwrap();
        assert_eq!(e.page().shape(&first).unwrap().resolved_z_index(), lowest);
    }

    // ---------------------------------------------------------- change shape

    #[test]
    fn changing_a_shape_keeps_its_box_style_and_text() {
        let mut e = editor_with_label("Approved?");
        let id = e.selection[0].clone();
        e.set_selection_fill(Color::rgb(0xfe, 0xf3, 0xc7));
        let before = e.page().shape(&id).unwrap().clone();

        e.change_shape_type(ShapeType::Decision);
        let after = e.page().shape(&id).unwrap();

        assert_eq!(after.shape_type, ShapeType::Decision);
        assert_eq!(after.bounding_box, before.bounding_box);
        assert_eq!(after.text_str(), "Approved?");
        assert_eq!(after.resolved_style().resolved_fill(), before.resolved_style().resolved_fill());
    }

    #[test]
    fn changing_to_a_type_that_rejects_text_drops_the_label() {
        // Keeping it would leave a shape carrying a field the format does not
        // allow it to have.
        let mut e = editor_with_label("gone");
        e.change_shape_type(ShapeType::Or);
        assert_eq!(e.page().shapes()[0].text, None);
    }

    #[test]
    fn changing_a_shape_is_undoable() {
        let mut e = editor_with_label("x");
        e.change_shape_type(ShapeType::Database);
        assert_eq!(e.page().shapes()[0].shape_type, ShapeType::Database);
        e.undo();
        assert_eq!(e.page().shapes()[0].shape_type, ShapeType::Rectangle);
    }

    // ----------------------------------------------------------- paint style

    #[test]
    fn a_style_can_be_lifted_off_one_shape_and_painted_onto_another() {
        let mut e = editor_with_two_shapes();
        let ids: Vec<String> = e.page().shapes().iter().map(|s| s.id.clone()).collect();
        let amber = Color::rgb(0xfe, 0xf3, 0xc7);

        e.selection = vec![ids[0].clone()];
        e.set_selection_fill(amber);
        e.set_opacity(50.0);
        assert!(e.copy_style());

        e.selection = vec![ids[1].clone()];
        e.paste_style();

        let painted = e.page().shape(&ids[1]).unwrap();
        assert_eq!(painted.resolved_style().resolved_fill().color(), Some(amber));
        assert_eq!(painted.opacity_factor(), 0.5);
    }

    #[test]
    fn painting_a_style_leaves_the_geometry_and_text_alone() {
        let mut e = editor_with_two_shapes();
        let ids: Vec<String> = e.page().shapes().iter().map(|s| s.id.clone()).collect();
        e.set_selection_text_for_test(&ids[1], "keep me");

        e.selection = vec![ids[0].clone()];
        e.set_selection_fill(Color::rgb(0xd1, 0xfa, 0xe5));
        e.copy_style();

        let before = e.page().shape(&ids[1]).unwrap().bounding_box;
        e.selection = vec![ids[1].clone()];
        e.paste_style();

        let after = e.page().shape(&ids[1]).unwrap();
        assert_eq!(after.bounding_box, before);
        assert_eq!(after.text_str(), "keep me");
        assert_eq!(after.shape_type, ShapeType::Decision, "and its type");
    }

    #[test]
    fn painting_with_nothing_copied_says_so_and_changes_nothing() {
        let mut e = editor_with_two_shapes();
        e.select_all_shapes();
        assert!(!e.has_copied_style());

        e.paste_style();
        assert!(!e.can_undo());
        assert!(e.status.contains("No style"), "got {:?}", e.status);
    }

    #[test]
    fn a_connectors_style_can_be_painted_onto_another_connector() {
        let mut e = editor_with_connector();
        let red = Color::rgb(0xdc, 0x26, 0x26);
        e.set_selection_stroke(red, Some(3.0));
        assert!(e.copy_style());

        // A second connector, drawn the other way.
        let mut tcx = TestCx::new();
        e.tool = Tool::Connector;
        e.pointer_down(Point::new(450.0, 130.0), mods(), false, 1, &mut tcx.ctx());
        e.pointer_move(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());
        e.pointer_up(Point::new(150.0, 130.0), mods(), &mut tcx.ctx());

        e.paste_style();
        let second = e.page().lines()[1].resolved_stroke();
        assert_eq!(second.resolved_color(), red);
        assert_eq!(second.resolved_width(), 3.0);
    }

    // ----------------------------------------------------------- page setup

    #[test]
    fn page_size_can_be_changed_and_read_back() {
        let mut e = Editor::new();
        assert_eq!(e.page_size_name(), "letter");

        e.set_page_size(dc_model::PageSize {
            size_type: "a4".into(),
            format: None,
            w: None,
            h: None,
        });
        assert_eq!(e.page_size_name(), "a4");
        assert!(!e.page_is_infinite());
    }

    #[test]
    fn switching_to_an_infinite_canvas_is_reported() {
        let mut e = Editor::new();
        e.set_infinite_canvas(true);
        assert!(e.page_is_infinite());
        assert_eq!(e.page_size_name(), "infinite");

        e.set_infinite_canvas(false);
        assert!(!e.page_is_infinite());
    }

    #[test]
    fn page_setup_is_undoable() {
        let mut e = Editor::new();
        e.set_infinite_canvas(true);
        e.undo();
        assert!(!e.page_is_infinite());
    }

    #[test]
    fn alignment_applies_to_an_open_label() {
        let mut tcx = TestCx::new();
        let mut e = editor_with_label("text");
        e.begin_text_edit_on_selection(&mut tcx.ctx());
        e.set_text_align(dc_model::TextAlign::Left);
        e.commit_text_edit();

        assert_eq!(
            dc_model::markup::parse(&label_markup(&e), Default::default()).align,
            dc_model::TextAlign::Left
        );
    }
}
