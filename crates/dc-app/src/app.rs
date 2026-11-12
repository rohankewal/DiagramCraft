//! Window, GPU surface, and event loop.
//!
//! Scenes are assembled in three stages so that device pixels, logical
//! interface pixels, and document units each stay in their own space:
//!
//! 1. The document is encoded into `doc_scene` in canvas-local logical pixels.
//! 2. `ui_scene` receives the document, offset to the canvas rectangle, then
//!    the chrome on top.
//! 3. `scene` receives `ui_scene` scaled by the display's scale factor, which
//!    is what the GPU rasterizes.
//!
//! Keeping the interface in logical pixels means the toolbar is the same
//! physical size on a high-density display as on a low-density one, without
//! any metric in the interface code needing to know about density.

use crate::editor::{Editor, Mods, Tool};
use crate::menu::ViewState;
use crate::textedit::{TextCommand, TextCtx};
use crate::ui::{Chrome, RegionKind, SettingChange, TabDrag, UiAction};
use dc_model::Point;
use dc_render::theme::{Metrics, Theme};
use kurbo::{Affine, Vec2};
use std::num::NonZeroUsize;
use std::sync::Arc;
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, AaSupport, RenderParams, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// Whether the menu bar is drawn inside the window.
///
/// macOS has a system menu bar and the commands go there instead; everywhere
/// else the window draws its own.
pub const MENUBAR_IN_WINDOW: bool = !cfg!(target_os = "macos");

/// Something that happened outside the window and has to be carried into the
/// event loop.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// A system menu item was chosen, by its id.
    MenuItem(String),
}

/// Pixels nudged per arrow key press, and the multiplier when shift is held.
const NUDGE_STEP: f64 = 1.0;
const NUDGE_COARSE: f64 = 10.0;
/// Clicks closer together than this, and within the slop, count as a sequence.
const DOUBLE_CLICK_MS: u128 = 450;
const DOUBLE_CLICK_SLOP: f64 = 4.0;
/// How far a page tab must be dragged sideways before it starts moving, rather
/// than the press being treated as a click that selects the page.
const TAB_DRAG_THRESHOLD: f64 = 6.0;

enum Surface {
    Pending,
    Live {
        window: Arc<Window>,
        surface: RenderSurface<'static>,
    },
}

pub struct App {
    context: RenderContext,
    /// One Vello renderer per device, indexed by the surface's device id.
    renderers: Vec<Option<vello::Renderer>>,
    surface: Surface,

    scene: Scene,
    ui_scene: Scene,
    doc_scene: Scene,

    document_renderer: dc_render::Renderer,
    editor: Editor,
    clipboard: crate::clipboard::Clipboard,
    metrics: Metrics,
    /// Interface state the menus reflect: grid, theme, palette library.
    view: ViewState,

    /// The menu currently dropped down, by index.
    open_menu: Option<usize>,
    /// A right-click context menu, and where it is anchored.
    context_menu: Option<(crate::ui::ContextTarget, kurbo::Point)>,
    /// The system menu bar, where the platform has one.
    #[cfg(target_os = "macos")]
    native_menus: Option<crate::native_menu::NativeMenus>,
    /// Lets the menu bar, which fires outside the event loop, wake it.
    proxy: Option<winit::event_loop::EventLoopProxy<AppEvent>>,

    /// A page tab being dragged to a new position.
    tab_drag: Option<TabDrag>,
    /// Where a press on a page tab started, so a click can be told from a drag.
    tab_press: Option<(usize, f64)>,
    /// A shape tile pressed but not yet resolved into a click or a drag onto
    /// the canvas.
    shape_tile_press: bool,
    /// A guide being pulled off a ruler: true for a vertical one.
    guide_drag: Option<bool>,

    /// Window size in logical pixels.
    window_size: Vec2,
    scale_factor: f64,

    pointer: kurbo::Point,
    hovered_region: Option<usize>,
    mods: Mods,
    space_held: bool,
    /// The tool to restore when a space-bar pan ends.
    tool_before_pan: Option<Tool>,
    dragging_canvas: bool,

    /// Where and when the last click landed, for detecting double clicks.
    last_click: Option<(kurbo::Point, std::time::Instant)>,
    click_streak: u32,
    /// When the previous frame was drawn, for the caret blink clock.
    last_frame: std::time::Instant,
    /// Set by the Quit command; the event loop exits on the next turn.
    quit_requested: bool,
    /// Whether preference changes are written to the settings file.
    persist_settings: bool,
}

impl App {
    /// An application with the user's saved preferences, which it writes back
    /// as they change.
    pub fn new() -> Self {
        let mut app = App::with_settings(crate::settings::Settings::load());
        app.persist_settings = true;
        app
    }

    /// An application with a given set of preferences, kept to itself.
    ///
    /// Tests build through here for two reasons: what they assert must not
    /// depend on the settings file on the machine running them, and running
    /// them must not overwrite it.
    pub fn with_settings(settings: crate::settings::Settings) -> Self {
        let mut app = App::empty();
        app.view.settings = settings;
        app.apply_settings();
        app
    }

    fn empty() -> Self {
        App {
            context: RenderContext::new(),
            renderers: Vec::new(),
            surface: Surface::Pending,
            scene: Scene::new(),
            ui_scene: Scene::new(),
            doc_scene: Scene::new(),
            document_renderer: dc_render::Renderer::new(),
            editor: Editor::new(),
            clipboard: crate::clipboard::Clipboard::new(),
            // Where the system owns the menu bar, the window gives back the
            // strip the in-window one would have taken.
            metrics: Metrics {
                menubar_height: if MENUBAR_IN_WINDOW {
                    Metrics::default().menubar_height
                } else {
                    0.0
                },
                ..Metrics::default()
            },
            view: ViewState::default(),
            open_menu: None,
            context_menu: None,
            #[cfg(target_os = "macos")]
            native_menus: None,
            proxy: None,
            tab_drag: None,
            tab_press: None,
            shape_tile_press: false,
            guide_drag: None,
            window_size: Vec2::new(1440.0, 900.0),
            scale_factor: 1.0,
            pointer: kurbo::Point::ZERO,
            hovered_region: None,
            mods: Mods::default(),
            space_held: false,
            tool_before_pan: None,
            dragging_canvas: false,
            last_click: None,
            click_streak: 0,
            last_frame: std::time::Instant::now(),
            quit_requested: false,
            persist_settings: false,
        }
    }

    /// Borrows the renderer's Parley contexts for text editing.
    ///
    /// Taking them from the renderer rather than owning a second set keeps the
    /// editor's line breaking identical to the one used for painting.
    fn text_ctx(renderer: &mut dc_render::Renderer) -> TextCtx<'_> {
        let (fonts, layouts) = renderer.text_engine().contexts();
        TextCtx::new(fonts, layouts)
    }

    /// How many clicks this press continues, for double-click detection.
    fn click_count(&mut self) -> u32 {
        let now = std::time::Instant::now();
        let continues = self.last_click.is_some_and(|(p, t)| {
            now.duration_since(t).as_millis() < DOUBLE_CLICK_MS
                && p.distance(self.pointer) < DOUBLE_CLICK_SLOP
        });
        self.click_streak = if continues { self.click_streak + 1 } else { 1 };
        self.last_click = Some((self.pointer, now));
        self.click_streak
    }

    /// Opens a document at startup, as when launched with a file argument.
    ///
    /// A Visio drawing is imported rather than refused, so dragging a `.vsdx`
    /// onto the application does what the user meant.
    pub fn open_initial(&mut self, path: &std::path::Path) {
        self.open_any(path);
    }

    /// Decodes the open document's bundled images.
    ///
    /// Called on every document change rather than lazily per frame, so the
    /// cost lands once at load instead of inside the render loop.
    fn sync_images(&mut self) {
        let assets: Vec<(String, Vec<u8>)> = self
            .editor
            .package
            .images
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect();
        self.document_renderer.images.load(assets);
    }

    pub fn run(mut self) -> Result<(), winit::error::EventLoopError> {
        let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
        event_loop.set_control_flow(ControlFlow::Wait);
        // The system menu bar fires on its own thread of control, outside the
        // event loop; the proxy is how it gets back in.
        self.proxy = Some(event_loop.create_proxy());
        event_loop.run_app(&mut self)
    }

    /// Puts the icon on the dock, where the platform has one.
    ///
    /// macOS has no per-window icon — `set_window_icon` is a no-op there —
    /// and an unbundled binary shows a generic placeholder until the
    /// application sets one for itself.
    #[cfg(target_os = "macos")]
    fn set_dock_icon(&self) {
        use objc2::ClassType;
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::{MainThreadMarker, NSData};

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let data = NSData::with_bytes(crate::icon::EMBEDDED_PNG);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            log::warn!("the embedded icon could not be read as an image");
            return;
        };
        // SAFETY: the application object belongs to the main thread, which
        // `MainThreadMarker` proves we are on, and the image outlives the
        // call because AppKit retains it.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn set_dock_icon(&self) {}

    /// Builds the system menu bar, where the platform has one.
    #[cfg(target_os = "macos")]
    fn install_native_menus(&mut self) {
        if self.native_menus.is_some() {
            return;
        }
        let menus = crate::menu::menus(&self.editor, &self.view);
        match crate::native_menu::NativeMenus::install(&menus) {
            Ok(installed) => self.native_menus = Some(installed),
            Err(e) => {
                // A missing menu bar is a degraded interface, not a reason to
                // refuse to open: every command still has a keyboard binding.
                log::error!("could not install the menu bar: {e}");
                return;
            }
        }

        if let Some(proxy) = self.proxy.clone() {
            muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
                let _ = proxy.send_event(AppEvent::MenuItem(event.id.0.clone()));
            }));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn install_native_menus(&mut self) {}

    /// Brings the system menu bar in line with the current state.
    #[cfg(target_os = "macos")]
    fn sync_native_menus(&mut self) {
        let menus = crate::menu::menus(&self.editor, &self.view);
        if let Some(native) = self.native_menus.as_mut() {
            native.sync(&menus);
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn sync_native_menus(&mut self) {}

    /// Runs the command a system menu item stands for.
    fn on_menu_item(&mut self, id: &str) {
        #[cfg(target_os = "macos")]
        if let Some(action) = self.native_menus.as_ref().and_then(|n| n.action(id)) {
            self.perform(action);
            self.update_title();
            self.request_redraw();
            return;
        }
        let _ = id;
    }

    fn chrome(&self) -> Chrome {
        Chrome::build_for(
            self.window_size,
            &self.editor,
            &self.metrics,
            &self.view,
            self.open_menu,
            self.tab_drag,
            self.context_menu,
        )
    }

    /// Keeps the editor's camera sized to the canvas rectangle.
    fn sync_canvas_viewport(&mut self) {
        let chrome = self.chrome();
        let size = chrome.layout.canvas_size();
        self.editor.resize_viewport(size.x, size.y);
    }

    fn request_redraw(&self) {
        if let Surface::Live { window, .. } = &self.surface {
            window.request_redraw();
        }
    }

    fn update_title(&self) {
        if let Surface::Live { window, .. } = &self.surface {
            window.set_title(&self.editor.title());
        }
    }

    /// Moves Next (`delta == 1`) or Previous (`delta == -1`) among the
    /// current query's matches, wrapping around either end.
    fn step_find_match(&mut self, delta: i64) {
        let matches = self.editor.find_matches(&self.view.find_query);
        if matches.is_empty() {
            return;
        }
        let len = matches.len() as i64;
        let current = self.view.find_match_index as i64;
        let next = ((current + delta) % len + len) % len;
        self.view.find_match_index = next as usize;
        self.editor.select_and_frame(&matches[self.view.find_match_index]);
    }

    /// Applies the typed link text to the selection and leaves the field.
    /// An empty field removes the link rather than setting an empty URL.
    fn commit_link_edit(&mut self) {
        let url = self.view.link_edit.trim();
        if url.is_empty() {
            self.editor.clear_link();
        } else {
            self.editor.set_link_url(url);
        }
        self.view.link_edit_focused = false;
    }

    /// Leaves the field without changing the shape's link.
    fn cancel_link_edit(&mut self) {
        self.view.link_edit_focused = false;
    }

    /// Applies the typed text to whichever custom-data field was focused.
    fn commit_custom_data_edit(&mut self) {
        match self.view.custom_data_focus.take() {
            Some(crate::ui::CustomDataFocus::Key(i)) => {
                self.editor.set_custom_data_key(i, &self.view.custom_data_edit);
            }
            Some(crate::ui::CustomDataFocus::Value(i)) => {
                self.editor.set_custom_data_value(i, &self.view.custom_data_edit);
            }
            None => {}
        }
    }

    /// Leaves the field without changing the entry.
    fn cancel_custom_data_edit(&mut self) {
        self.view.custom_data_focus = None;
    }

    // ------------------------------------------------------------- rendering

    fn render(&mut self) {
        // A frame is where state settles, so it is also where the system menu
        // bar is brought back in line with it. This happens before the surface
        // is borrowed, because it reads the whole application.
        self.sync_native_menus();

        let Surface::Live { surface, window } = &mut self.surface else {
            return;
        };
        let width = surface.config.width;
        let height = surface.config.height;
        if width == 0 || height == 0 {
            return;
        }

        // Advance the caret blink by however long the last frame took.
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        self.editor.tick(elapsed);
        // The shape under an open label may have moved since the last frame.
        self.editor.refresh_text_area();

        let chrome = Chrome::build_for(
            self.window_size,
            &self.editor,
            &self.metrics,
            &self.view,
            self.open_menu,
            self.tab_drag,
            self.context_menu,
        );
        let canvas = chrome.layout.canvas;

        // 1. The document, in canvas-local logical pixels.
        self.document_renderer.draw(
            &mut self.doc_scene,
            self.editor.document(),
            self.editor.page_index,
            &self.editor.camera,
            &self.editor.overlay(),
        );

        // A label on the canvas is drawn in document space so it pans and
        // zooms with its shape; a page title belongs to the chrome and is
        // drawn later, in window space.
        let editing_tab = self.editor.editing_page_title().is_some();
        if !editing_tab {
            let camera = self.editor.camera.transform();
            Self::draw_text_edit(
                &mut self.doc_scene,
                &mut self.editor,
                &mut self.document_renderer,
                camera,
            );
        }

        // 2. Document into the interface scene, then chrome over the top. The
        //    chrome is drawn second so content that overflows the canvas is
        //    covered by the panels rather than bleeding into them.
        self.ui_scene.reset();
        self.ui_scene.append(
            &self.doc_scene,
            Some(Affine::translate((canvas.x0, canvas.y0))),
        );
        // The chrome needs the theme and the renderer's text engine at once.
        // A theme is a hundred bytes of color, so copying it per frame is
        // cheaper than threading a second borrow through the renderer.
        let theme = self.document_renderer.theme.clone();
        chrome.draw(
            &mut self.ui_scene,
            &theme,
            &self.metrics,
            self.document_renderer.text_engine(),
            self.hovered_region,
            &self.editor.status,
            &self.editor.camera,
        );

        if editing_tab {
            Self::draw_text_edit(
                &mut self.ui_scene,
                &mut self.editor,
                &mut self.document_renderer,
                Affine::IDENTITY,
            );
        }

        // 3. Scale logical pixels to device pixels.
        self.scene.reset();
        self.scene
            .append(&self.ui_scene, Some(Affine::scale(self.scale_factor)));

        let device_handle = &self.context.devices[surface.dev_id];
        let renderer = self.renderers[surface.dev_id]
            .as_mut()
            .expect("a renderer is created with the surface");

        let base = self.document_renderer.theme.canvas;
        let params = RenderParams {
            base_color: peniko::Color::from_rgba8(base.r, base.g, base.b, base.a),
            width,
            height,
            antialiasing_method: AaConfig::Area,
        };

        if let Err(e) = renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &self.scene,
            &surface.target_view,
            &params,
        ) {
            log::error!("render failed: {e}");
            return;
        }

        // None of these states are fatal: a frame is skipped and the surface
        // reconfigured where that is the documented remedy.
        let frame = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Usable now, but reconfigure so the next frame is optimal.
                self.context.configure_surface(surface);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                log::warn!("surface outdated or lost; reconfiguring");
                self.context.configure_surface(surface);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            device_handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("diagramcraft.blit"),
                });
        surface
            .blitter
            .copy(&device_handle.device, &mut encoder, &surface.target_view, &view);
        device_handle.queue.submit([encoder.finish()]);
        window.pre_present_notify();
        // Presentation goes through the queue in this wgpu version.
        device_handle.queue.present(frame);

        // A blinking caret is the only thing that animates, so frames are
        // requested continuously while editing and not otherwise.
        if self.editor.is_editing() {
            if let Surface::Live { window, .. } = &self.surface {
                window.request_redraw();
            }
        }
    }

    // ----------------------------------------------------------------- events

    /// Paints the selection highlight, glyphs, and caret of an open label.
    fn draw_text_edit(
        scene: &mut Scene,
        editor: &mut Editor,
        renderer: &mut dc_render::Renderer,
        transform: Affine,
    ) {
        if editor.text_edit.is_none() {
            return;
        }
        let theme = renderer.theme.clone();
        let mut cx = Self::text_ctx(renderer);
        if let Some(edit) = editor.text_edit.as_mut() {
            crate::textedit::draw_session(scene, edit, &theme, transform, &mut cx);
        }
    }

    fn on_resize(&mut self, physical_width: u32, physical_height: u32) {
        if physical_width == 0 || physical_height == 0 {
            return;
        }
        if let Surface::Live { surface, .. } = &mut self.surface {
            self.context
                .resize_surface(surface, physical_width, physical_height);
        }
        self.window_size = Vec2::new(
            physical_width as f64 / self.scale_factor,
            physical_height as f64 / self.scale_factor,
        );
        self.sync_canvas_viewport();
    }

    /// Converts a window position into canvas-local coordinates.
    fn canvas_point(&self, window_point: kurbo::Point, chrome: &Chrome) -> Point {
        Point::new(
            window_point.x - chrome.layout.canvas.x0,
            window_point.y - chrome.layout.canvas.y0,
        )
    }

    fn on_pointer_move(&mut self) {
        // A press on a tab becomes a drag once the pointer has travelled far
        // enough that it cannot be a click any more.
        if let Some((from, start_x)) = self.tab_press {
            if self.tab_drag.is_none() && (self.pointer.x - start_x).abs() >= TAB_DRAG_THRESHOLD {
                self.tab_drag = Some(TabDrag { from, pointer_x: self.pointer.x });
            }
        }
        if let Some(drag) = self.tab_drag.as_mut() {
            drag.pointer_x = self.pointer.x;
            self.hovered_region = None;
            self.request_redraw();
            return;
        }

        let chrome = self.chrome();

        // A press on a shape tile rides along with the pointer rather than
        // sizing a box: the shape drops at its default size wherever the
        // button comes back up, so there is nothing to update here.
        if self.shape_tile_press {
            self.request_redraw();
            return;
        }

        self.hovered_region = chrome.hit(self.pointer);

        // Sliding across the bar with a menu open switches to the menu under
        // the pointer, which is what every menu bar does.
        if self.open_menu.is_some() {
            if let Some(index) = self.hovered_region {
                if let UiAction::OpenMenu(menu) = chrome.regions[index].action {
                    if self.open_menu != Some(menu) {
                        self.open_menu = Some(menu);
                    }
                }
            }
            self.request_redraw();
            return;
        }

        if self.dragging_canvas || chrome.is_over_canvas(self.pointer) {
            let local = self.canvas_point(self.pointer, &chrome);
            let mods = self.mods;
            let mut cx = Self::text_ctx(&mut self.document_renderer);
            self.editor.pointer_move(local, mods, &mut cx);
        } else {
            self.editor.hovered = None;
        }
        self.request_redraw();
    }

    /// Which ruler the pointer is over, as whether it yields a vertical
    /// guide. The top ruler runs across the page, so dragging from it gives
    /// a vertical line, and the left ruler gives a horizontal one.
    fn ruler_under_pointer(&self, chrome: &crate::ui::Chrome) -> Option<bool> {
        if chrome.layout.ruler_top.is_some_and(|r| r.contains(self.pointer)) {
            return Some(true);
        }
        if chrome.layout.ruler_left.is_some_and(|r| r.contains(self.pointer)) {
            return Some(false);
        }
        None
    }

    fn on_pointer_down(&mut self, button: MouseButton) {
        let chrome = self.chrome();
        self.shape_tile_press = false;

        if button == MouseButton::Right {
            self.editor.commit_text_edit();
            self.open_menu = None;
            self.context_menu = None;

            // Right-clicking a ruler takes the nearest guide off that axis,
            // which is how one is removed once it has been placed.
            if let Some(vertical) = self.ruler_under_pointer(&chrome) {
                let local = self.canvas_point(self.pointer, &chrome);
                let doc_point = self.editor.camera.screen_to_doc(local);
                let at = if vertical { doc_point.x } else { doc_point.y };
                let slop = self.editor.camera.screen_to_doc_scalar(6.0);
                self.editor.remove_guide_near(at, vertical, slop);
                self.request_redraw();
                return;
            }

            if let Some(index) = chrome.hit(self.pointer) {
                // A page tab gets its own menu, for the page it names rather
                // than whichever page happens to be active.
                if let UiAction::SelectPage(page) = chrome.regions[index].action {
                    self.context_menu = Some((crate::ui::ContextTarget::Page(page), self.pointer));
                }
            } else if chrome.layout.canvas.contains(self.pointer) {
                // Right-clicking settles the selection first, the same way
                // every other diagramming tool does, so the menu that opens
                // always matches what it is about to act on.
                let local = self.canvas_point(self.pointer, &chrome);
                let doc_point = self.editor.camera.screen_to_doc(local);
                self.editor.select_for_context_menu(doc_point);
                self.context_menu = Some((crate::ui::ContextTarget::Canvas, self.pointer));
            }
            self.request_redraw();
            return;
        }

        if button == MouseButton::Left {
            // Clicking anything other than the search field itself moves
            // focus away from it, the same way a native text field blurs.
            if self.view.palette_search_focused {
                let still_the_field = chrome
                    .hit(self.pointer)
                    .is_some_and(|i| chrome.regions[i].action == UiAction::FocusPaletteSearch);
                if !still_the_field {
                    self.view.palette_search_focused = false;
                }
            }

            // A click away from the link field commits it, the way leaving
            // any other field does.
            if self.view.link_edit_focused {
                let still_the_field = chrome
                    .hit(self.pointer)
                    .is_some_and(|i| chrome.regions[i].action == UiAction::FocusLinkField);
                if !still_the_field {
                    self.commit_link_edit();
                }
            }

            // Same deal for whichever custom-data field is focused: clicking
            // a different one commits this one before the new one takes over.
            if let Some(focus) = self.view.custom_data_focus {
                let expected = match focus {
                    crate::ui::CustomDataFocus::Key(i) => UiAction::FocusCustomDataKey(i),
                    crate::ui::CustomDataFocus::Value(i) => UiAction::FocusCustomDataValue(i),
                };
                let still_the_field =
                    chrome.hit(self.pointer).is_some_and(|i| chrome.regions[i].action == expected);
                if !still_the_field {
                    self.commit_custom_data_edit();
                }
            }

            // The find fields have no staged value to commit, just focus
            // to drop on a click elsewhere.
            if self.view.find_query_focused
                && !chrome.hit(self.pointer).is_some_and(|i| chrome.regions[i].action == UiAction::FocusFindQuery)
            {
                self.view.find_query_focused = false;
            }
            if self.view.find_replacement_focused
                && !chrome
                    .hit(self.pointer)
                    .is_some_and(|i| chrome.regions[i].action == UiAction::FocusFindReplacement)
            {
                self.view.find_replacement_focused = false;
            }

            // A click inside an open page-title edit drives its caret.
            if self.editor.editing_page_title().is_some() {
                let inside = self
                    .editor
                    .text_edit
                    .as_ref()
                    .is_some_and(|e| e.area().contains(self.pointer));
                if inside {
                    let point = Point::new(self.pointer.x, self.pointer.y);
                    let shift = self.mods.shift;
                    let mut cx = Self::text_ctx(&mut self.document_renderer);
                    if let Some(edit) = self.editor.text_edit.as_mut() {
                        edit.click(point, shift, &mut cx);
                    }
                    self.request_redraw();
                    return;
                }
            }

            if let Some(index) = chrome.hit(self.pointer) {
                let region = &chrome.regions[index];
                let action = region.action.clone();
                let rect = region.rect;

                // Leaving the canvas finishes whatever was being typed —
                // unless the control is one that acts on the open label, in
                // which case committing first would throw away the selection
                // it exists to work on.
                if !action.keeps_label_open() {
                    self.editor.commit_text_edit();
                }

                // A menu title toggles its menu rather than running a command.
                if let UiAction::OpenMenu(menu) = action {
                    self.click_count();
                    self.open_menu = if self.open_menu == Some(menu) {
                        None
                    } else {
                        Some(menu)
                    };
                    self.request_redraw();
                    return;
                }

                // A second click on the current page tab renames it; a press
                // and sideways drag reorders the pages.
                if let UiAction::SelectPage(page) = action {
                    let clicks = self.click_count();
                    if clicks >= 2 {
                        let surface = self.document_renderer.theme.panel;
                        let mut cx = Self::text_ctx(&mut self.document_renderer);
                        self.editor.begin_page_title_edit(page, rect, surface, &mut cx);
                        self.request_redraw();
                        return;
                    }
                    // Selecting waits for the release, because this press may
                    // turn out to be the start of a drag that reorders pages.
                    self.tab_press = Some((page, self.pointer.x));
                    self.open_menu = None;
                    self.context_menu = None;
                    self.request_redraw();
                    return;
                }
                self.click_count();

                // Running a command closes whatever menu it came from.
                self.open_menu = None;
                self.context_menu = None;
                self.perform(action);
                // A press on a shape tile selects the tool, same as a click
                // would, but also remembers that the press might keep going
                // straight onto the canvas as one continuous drag-and-drop.
                self.shape_tile_press = matches!(region.kind, RegionKind::ShapeTile(_));
                self.update_title();
                self.request_redraw();
                return;
            }

            if self.context_menu.is_some() {
                // Same deal as the open menu below: a miss inside the panel
                // is swallowed rather than treated as a dismissal.
                if chrome.covered_by_context_menu(self.pointer) {
                    return;
                }
                self.context_menu = None;
                self.request_redraw();
                return;
            }

            if self.open_menu.is_some() {
                // A press on a rule or a dimmed command is swallowed: the menu
                // is opaque, and dismissing on a miss inside it would make the
                // menu feel like it was closing at random.
                if chrome.covered_by_open_menu(self.pointer) {
                    return;
                }
                // A click anywhere else dismisses the menu without acting on
                // whatever is underneath it.
                self.open_menu = None;
                self.request_redraw();
                return;
            }
        }

        // A press on a ruler pulls a guide out of it, the way every drawing
        // tool with rulers behaves. It is checked before the canvas because
        // the strips sit outside the canvas rectangle.
        if let Some(vertical) = self.ruler_under_pointer(&chrome) {
            self.guide_drag = Some(vertical);
            self.request_redraw();
            return;
        }

        if !chrome.layout.canvas.contains(self.pointer) {
            return;
        }

        let middle = button == MouseButton::Middle || self.space_held;
        let local = self.canvas_point(self.pointer, &chrome);
        let clicks = self.click_count();
        let mods = self.mods;
        self.dragging_canvas = true;
        {
            let mut cx = Self::text_ctx(&mut self.document_renderer);
            self.editor.pointer_down(local, mods, middle, clicks, &mut cx);
        }
        self.update_title();
        self.request_redraw();
    }

    fn on_pointer_up(&mut self) {
        // Finishing a tab drag drops the page into the slot the marker showed.
        if let Some(drag) = self.tab_drag.take() {
            self.tab_press = None;
            let chrome = self.chrome();
            if let Some(to) = chrome.tab_index_at(drag.pointer_x) {
                self.perform(UiAction::MovePage { from: drag.from, to });
            }
            self.request_redraw();
            return;
        }
        // A press on a tab that never became a drag is a plain click.
        if let Some((page, _)) = self.tab_press.take() {
            self.editor.select_page(page);
            self.request_redraw();
            return;
        }

        // A shape tile dropped over the canvas places the shape at its
        // default size, centered wherever the button came back up — a
        // drag-and-drop, not a box drawn out by the gesture's path.
        // A guide is placed where the drag let go, and dropped if it never
        // left the ruler it came from.
        if let Some(vertical) = self.guide_drag.take() {
            let chrome = self.chrome();
            if chrome.layout.canvas.contains(self.pointer) {
                let local = self.canvas_point(self.pointer, &chrome);
                let doc_point = self.editor.camera.screen_to_doc(local);
                let at = if vertical { doc_point.x } else { doc_point.y };
                self.editor.add_guide(at, vertical);
            }
            self.request_redraw();
            return;
        }

        if self.shape_tile_press {
            self.shape_tile_press = false;
            let chrome = self.chrome();
            if chrome.layout.canvas.contains(self.pointer) {
                if let Tool::Shape(shape_type) = self.editor.tool.clone() {
                    let local = self.canvas_point(self.pointer, &chrome);
                    let doc_point = self.editor.camera.screen_to_doc(local);
                    let (w, h) = shape_type.default_size();
                    let box_ = dc_model::BoundingBox::new(doc_point.x - w / 2.0, doc_point.y - h / 2.0, w, h);
                    self.editor.create_shape(shape_type, box_);
                    self.editor.tool = Tool::Select;
                }
            }
            self.update_title();
            self.request_redraw();
            return;
        }

        if !self.dragging_canvas {
            return;
        }
        let chrome = self.chrome();
        let local = self.canvas_point(self.pointer, &chrome);
        let mods = self.mods;
        {
            let mut cx = Self::text_ctx(&mut self.document_renderer);
            self.editor.pointer_up(local, mods, &mut cx);
        }
        self.dragging_canvas = false;
        self.update_title();
        self.request_redraw();
    }

    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        let chrome = self.chrome();
        if !chrome.layout.canvas.contains(self.pointer) {
            return;
        }
        let (dx, dy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (x as f64 * 24.0, y as f64 * 24.0),
            MouseScrollDelta::PixelDelta(p) => (p.x / self.scale_factor, p.y / self.scale_factor),
        };
        let local = self.canvas_point(self.pointer, &chrome);
        self.editor.wheel(local, dx, dy, self.mods.command);
        self.request_redraw();
    }

    /// Keyboard input.
    ///
    /// `text` is the character the platform produced for this key, which is
    /// what must be inserted: deriving characters from key codes mishandles
    /// dead keys, alternate layouts, and shifted symbols.
    fn on_key(&mut self, key: &Key, state: ElementState, text: Option<&str>) {
        // The palette search is a plain field, not a document label, so it
        // takes keystrokes without going anywhere near `TextEdit`.
        if self.view.palette_search_focused {
            if state == ElementState::Pressed {
                match key {
                    Key::Named(NamedKey::Escape) => {
                        self.view.palette_search.clear();
                        self.view.palette_search_focused = false;
                    }
                    Key::Named(NamedKey::Backspace) => {
                        self.view.palette_search.pop();
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.view.palette_search_focused = false;
                    }
                    _ => {
                        if let Some(text) = text {
                            self.view.palette_search.push_str(text);
                        }
                    }
                }
                self.request_redraw();
            }
            return;
        }

        // Same deal for the link field: a plain string, not a label.
        if self.view.link_edit_focused {
            if state == ElementState::Pressed {
                match key {
                    Key::Named(NamedKey::Escape) => self.cancel_link_edit(),
                    Key::Named(NamedKey::Backspace) => {
                        self.view.link_edit.pop();
                    }
                    Key::Named(NamedKey::Enter) => self.commit_link_edit(),
                    _ => {
                        if let Some(text) = text {
                            self.view.link_edit.push_str(text);
                        }
                    }
                }
                self.request_redraw();
            }
            return;
        }

        if self.view.custom_data_focus.is_some() {
            if state == ElementState::Pressed {
                match key {
                    Key::Named(NamedKey::Escape) => self.cancel_custom_data_edit(),
                    Key::Named(NamedKey::Backspace) => {
                        self.view.custom_data_edit.pop();
                    }
                    Key::Named(NamedKey::Enter) => self.commit_custom_data_edit(),
                    _ => {
                        if let Some(text) = text {
                            self.view.custom_data_edit.push_str(text);
                        }
                    }
                }
                self.request_redraw();
            }
            return;
        }

        // The find query is live, like the palette search: there is no
        // separate "applied" value for it to diverge from, so Enter steps
        // to the next match instead of committing anything.
        if self.view.find_query_focused {
            if state == ElementState::Pressed {
                match key {
                    Key::Named(NamedKey::Escape) => self.view.find_query_focused = false,
                    Key::Named(NamedKey::Backspace) => {
                        self.view.find_query.pop();
                        self.view.find_match_index = 0;
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.step_find_match(if self.mods.shift { -1 } else { 1 });
                    }
                    _ => {
                        if let Some(text) = text {
                            self.view.find_query.push_str(text);
                            self.view.find_match_index = 0;
                        }
                    }
                }
                self.request_redraw();
            }
            return;
        }

        if self.view.find_replacement_focused {
            if state == ElementState::Pressed {
                match key {
                    Key::Named(NamedKey::Escape) => self.view.find_replacement_focused = false,
                    Key::Named(NamedKey::Backspace) => {
                        self.view.find_replacement.pop();
                    }
                    Key::Named(NamedKey::Enter) => self.perform(UiAction::ReplaceCurrentMatch),
                    _ => {
                        if let Some(text) = text {
                            self.view.find_replacement.push_str(text);
                        }
                    }
                }
                self.request_redraw();
            }
            return;
        }

        if self.editor.is_editing() {
            if state == ElementState::Pressed && self.on_key_while_editing(key, text) {
                self.update_title();
                self.request_redraw();
                return;
            }
            if self.editor.is_editing() {
                // Swallow everything else so stray shortcuts cannot act on the
                // document while the user is mid-word.
                return;
            }
        }

        if let Key::Named(NamedKey::Space) = key {
            let held = state.is_pressed();
            if held != self.space_held {
                self.space_held = held;
                if held {
                    self.tool_before_pan = Some(self.editor.tool.clone());
                } else if let Some(tool) = self.tool_before_pan.take() {
                    self.editor.tool = tool;
                }
            }
            return;
        }

        if state != ElementState::Pressed {
            return;
        }

        // Where the system owns the menu bar, it also owns every command-key
        // shortcut: the menu matches the key before the window ever sees it.
        // Handling it here as well is how a shortcut comes to fire twice, so
        // this layer stands aside rather than claiming it a second time —
        // and stands aside entirely, because falling through to the
        // unmodified bindings would make command-V pick the select tool.
        if self.mods.command && !MENUBAR_IN_WINDOW {
            return;
        }

        let cmd = self.mods.command;
        let shift = self.mods.shift;

        match key {
            Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) => {
                self.editor.delete_selection();
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::F2) => {
                let mut cx = Self::text_ctx(&mut self.document_renderer);
                self.editor.begin_text_edit_on_selection(&mut cx);
            }
            Key::Named(NamedKey::Escape) => {
                // Escape dismisses whatever is in front: the settings screen,
                // then an open menu, then the selection, then the tool.
                if self.view.settings_open {
                    self.view.settings_open = false;
                } else if self.context_menu.take().is_some() {
                } else if self.open_menu.take().is_some() {
                } else if self.editor.tool == Tool::Select {
                    self.editor.clear_selection();
                } else {
                    self.editor.tool = Tool::Select;
                }
            }
            Key::Named(NamedKey::ArrowLeft) => self.nudge(-1.0, 0.0, shift),
            Key::Named(NamedKey::ArrowRight) => self.nudge(1.0, 0.0, shift),
            Key::Named(NamedKey::ArrowUp) => self.nudge(0.0, -1.0, shift),
            Key::Named(NamedKey::ArrowDown) => self.nudge(0.0, 1.0, shift),

            Key::Character(c) => {
                let lower = c.to_lowercase();
                match (cmd, lower.as_str()) {
                    (true, "z") if shift => self.editor.redo(),
                    (true, "z") => self.editor.undo(),
                    (true, "y") => self.editor.redo(),
                    (true, "s") if shift => self.perform(UiAction::SaveAs),
                    (true, "s") => self.perform(UiAction::Save),
                    (true, "o") => self.perform(UiAction::Open),
                    (true, "n") => self.perform(UiAction::NewDocument),
                    (true, "a") => self.editor.select_all(),
                    (true, "f") => self.perform(UiAction::ToggleFind),
                    (true, "d") => self.editor.duplicate_selection(),
                    (true, "b") => {
                        self.editor.toggle_text_style(crate::editor::TextToggle::Bold)
                    }
                    (true, "i") => {
                        self.editor.toggle_text_style(crate::editor::TextToggle::Italic)
                    }
                    (true, "u") => {
                        self.editor.toggle_text_style(crate::editor::TextToggle::Underline)
                    }
                    (true, "c") if self.mods.alt => self.perform(UiAction::CopyStyle),
                    (true, "v") if self.mods.alt => self.perform(UiAction::PasteStyle),
                    (true, "c") => self.copy_selection(false),
                    (true, "x") if shift => {
                        self.editor
                            .toggle_text_style(crate::editor::TextToggle::Strikethrough)
                    }
                    (true, "x") => self.copy_selection(true),
                    (true, "v") => self.paste(),
                    (true, "g") if shift => self.editor.ungroup_selection(),
                    (true, "g") => self.editor.group_selection(),
                    (true, "e") if shift => self.perform(UiAction::ExportPng),
                    (true, "0") => self.editor.reset_zoom(),
                    (true, "1") => self.editor.zoom_to_fit(),
                    (true, "=") | (true, "+") => {
                        self.perform(UiAction::ZoomIn);
                    }
                    (true, "-") => self.perform(UiAction::ZoomOut),
                    (true, "]") if shift => self.editor.reorder_selection(true),
                    (true, "[") if shift => self.editor.reorder_selection(false),
                    (true, "]") => self.editor.nudge_z_order(true),
                    (true, "[") => self.editor.nudge_z_order(false),
                    (true, ".") if shift => self.editor.scale_text_size(1.15),
                    // Shift-command-comma shrinks text; command-comma alone is
                    // the settings, so the guarded arm has to come first.
                    (true, ",") if shift => self.editor.scale_text_size(1.0 / 1.15),
                    (true, ",") => self.perform(UiAction::OpenSettings),

                    // Unmodified single keys select tools.
                    (false, "v") => self.editor.tool = Tool::Select,
                    (false, "h") => self.editor.tool = Tool::Pan,
                    (false, "c") => self.editor.tool = Tool::Connector,
                    (false, "r") => {
                        self.editor.tool = Tool::Shape(dc_model::ShapeType::Rectangle);
                    }
                    (false, "t") => {
                        self.editor.tool = Tool::Shape(dc_model::ShapeType::TextBlock);
                    }
                    (false, "d") => {
                        self.editor.tool = Tool::Shape(dc_model::ShapeType::Decision);
                    }
                    (false, "p") => {
                        self.editor.tool = Tool::Shape(dc_model::ShapeType::Process);
                    }
                    (false, "f") => self.editor.tool = Tool::Freehand,
                    _ => {}
                }
            }
            _ => {}
        }

        self.update_title();
        self.request_redraw();
    }

    /// Handles a key while a label is open. Returns whether it was consumed.
    fn on_key_while_editing(&mut self, key: &Key, text: Option<&str>) -> bool {
        let shift = self.mods.shift;
        // Alt is the word-wise modifier on macOS, Control elsewhere; accepting
        // either keeps one binding set working across platforms.
        let word = self.mods.alt || self.mods.command;
        let command = self.mods.command;

        let text_command = match key {
            Key::Named(NamedKey::Escape) => {
                self.editor.cancel_text_edit();
                return true;
            }
            Key::Named(NamedKey::Enter) if !shift => {
                self.editor.commit_text_edit();
                return true;
            }
            Key::Named(NamedKey::Tab) => {
                self.editor.commit_text_edit();
                return true;
            }
            Key::Named(NamedKey::Enter) => TextCommand::Newline,
            Key::Named(NamedKey::Backspace) => {
                if word { TextCommand::BackspaceWord } else { TextCommand::Backspace }
            }
            Key::Named(NamedKey::Delete) => {
                if word { TextCommand::DeleteWord } else { TextCommand::Delete }
            }
            Key::Named(NamedKey::ArrowLeft) => TextCommand::Left { select: shift, word },
            Key::Named(NamedKey::ArrowRight) => TextCommand::Right { select: shift, word },
            // Command-Up and Command-Down jump to the ends of the label, so
            // these guarded arms must precede the plain ones.
            Key::Named(NamedKey::ArrowUp) if command => TextCommand::TextStart { select: shift },
            Key::Named(NamedKey::ArrowDown) if command => TextCommand::TextEnd { select: shift },
            Key::Named(NamedKey::ArrowUp) => TextCommand::Up { select: shift },
            Key::Named(NamedKey::ArrowDown) => TextCommand::Down { select: shift },
            // Command-Home and Command-End reach the whole label, matching
            // the platform convention for text fields.
            Key::Named(NamedKey::Home) if command => TextCommand::TextStart { select: shift },
            Key::Named(NamedKey::End) if command => TextCommand::TextEnd { select: shift },
            Key::Named(NamedKey::Home) => TextCommand::LineStart { select: shift },
            Key::Named(NamedKey::End) => TextCommand::LineEnd { select: shift },
            Key::Named(NamedKey::Space) => TextCommand::Insert(" ".into()),

            Key::Character(c) if command => {
                match c.to_lowercase().as_str() {
                    "a" => TextCommand::SelectAll,
                    "c" => {
                        self.copy_selected_text();
                        return true;
                    }
                    "x" => {
                        self.copy_selected_text();
                        TextCommand::Backspace
                    }
                    "v" => match self.clipboard_text() {
                        Some(pasted) => TextCommand::Insert(pasted),
                        None => return true,
                    },
                    // Styling shortcuts apply to the whole label on commit.
                    "b" | "i" | "u" => return true,
                    // Saving mid-edit should capture what was typed.
                    "s" => {
                        self.editor.commit_text_edit();
                        self.perform(UiAction::Save);
                        return true;
                    }
                    "z" => {
                        self.editor.cancel_text_edit();
                        self.editor.undo();
                        return true;
                    }
                    _ => return true,
                }
            }

            _ => match text {
                // Control characters are not insertable text.
                Some(t) if !t.is_empty() && !t.chars().any(char::is_control) => {
                    TextCommand::Insert(t.to_string())
                }
                _ => return false,
            },
        };

        let mut cx = Self::text_ctx(&mut self.document_renderer);
        self.editor.text_command(text_command, &mut cx)
    }

    // ------------------------------------------------------------- clipboard

    /// Copies the text selected inside an open label.
    fn copy_selected_text(&mut self) {
        if let Some(selected) = self
            .editor
            .text_edit
            .as_ref()
            .and_then(|e| e.selected_text())
        {
            self.clipboard.set_text(&selected);
        }
    }

    /// Plain text from the clipboard, with line endings normalized.
    fn clipboard_text(&mut self) -> Option<String> {
        let text = self.clipboard.text()?;
        (!text.is_empty()).then(|| text.replace("\r\n", "\n").replace('\r', "\n"))
    }

    /// Copies the selected shapes, optionally removing them.
    fn copy_selection(&mut self, cut: bool) {
        let fragment = self.editor.copy_fragment();
        if fragment.is_empty() {
            return;
        }
        self.clipboard.set_fragment(&fragment);
        let count = fragment.shapes.len();
        if cut {
            self.editor.delete_selection();
            self.editor.status = format!("Cut {count} shape(s)");
        } else {
            self.editor.status = format!("Copied {count} shape(s)");
        }
    }

    /// Pastes shapes, or plain text into the selected shape's label.
    fn paste(&mut self) {
        if let Some(fragment) = self.clipboard.fragment() {
            let offset = self.editor.grid_spacing.max(8.0);
            self.editor.paste_fragment(&fragment, offset);
            return;
        }

        // Text on the clipboard becomes the label of whatever is selected.
        if let Some(text) = self.clipboard_text() {
            if !self.editor.selection.is_empty() {
                self.editor.set_selection_text(&text);
                self.editor.status = "Pasted text".into();
            }
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64, coarse: bool) {
        let step = if coarse { NUDGE_COARSE } else { NUDGE_STEP };
        self.editor.nudge(dx * step, dy * step);
    }

    fn perform(&mut self, action: UiAction) {
        // A command chosen from a menu reaches the same place a shortcut
        // does, so the commands that mean something different while a label
        // is open have to mean that here too. Otherwise copying inside a
        // label would copy the shape around it.
        if self.editor.is_editing() && self.perform_while_editing(&action) {
            self.update_title();
            self.request_redraw();
            return;
        }

        match action {
            UiAction::NoOp | UiAction::OpenMenu(_) => {}

            UiAction::SelectTool(tool) => self.editor.tool = tool,
            UiAction::NewDocument => {
                self.editor.new_document();
                self.sync_images();
            }

            UiAction::OpenTemplateGallery => self.view.template_gallery_open = true,
            UiAction::CloseTemplateGallery => self.view.template_gallery_open = false,
            UiAction::NewFromTemplate(index) => {
                if let Some(t) = crate::templates::ALL.get(index) {
                    self.editor.new_document_from_template((t.build)(), t.name);
                    self.sync_images();
                }
                self.view.template_gallery_open = false;
            }

            UiAction::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Diagram", &["lucid"])
                    .add_filter("Visio drawing", &["vsdx", "vsdm"])
                    .pick_file()
                {
                    self.open_any(&path);
                }
            }

            UiAction::ImportVsdx => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Visio drawing", &["vsdx", "vsdm"])
                    .pick_file()
                {
                    if let Err(e) = self.editor.import_vsdx(&path) {
                        self.editor.status = format!("Could not import: {e}");
                    }
                    self.sync_images();
                    self.sync_canvas_viewport();
                }
            }

            UiAction::ExportPng => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .set_file_name("diagram.png")
                    .save_file()
                {
                    let page = self.editor.page_index;
                    match crate::export::render_png(&self.editor.package, page, 2.0, &path) {
                        Ok((w, h)) => {
                            self.editor.status = format!("Exported {w}×{h} PNG");
                        }
                        Err(e) => self.editor.status = format!("Could not export: {e}"),
                    }
                }
            }

            UiAction::ExportVector { pdf } => {
                let (label, extension) = if pdf { ("PDF", "pdf") } else { ("SVG", "svg") };
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(&format!("{label} document"), &[extension])
                    .set_file_name(&format!("diagram.{extension}"))
                    .save_file()
                {
                    let page = self.editor.page_index;
                    let document = &self.editor.package.document;
                    let options = dc_export::Options::default();
                    let written = if pdf {
                        dc_export::write_pdf(document, page, options, &path)
                    } else {
                        dc_export::write_svg(document, page, options, &path)
                    };
                    self.editor.status = match written {
                        Ok(()) => format!("Exported {label}"),
                        Err(e) => format!("Could not export: {e}"),
                    };
                }
            }

            UiAction::InsertImage => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg", "gif", "webp"])
                    .pick_file()
                {
                    if let Err(e) = self.editor.insert_image(&path) {
                        self.editor.status = format!("Could not insert: {e}");
                    }
                    self.sync_images();
                }
            }

            UiAction::LinkData { source, key_column } => {
                self.editor.link_data(&source, &key_column);
            }

            UiAction::SetPageSize(name) => {
                self.editor.set_page_size(dc_model::PageSize {
                    size_type: name,
                    format: None,
                    w: None,
                    h: None,
                });
            }
            UiAction::ToggleInfiniteCanvas => {
                let infinite = self.editor.page_is_infinite();
                self.editor.set_infinite_canvas(!infinite);
            }

            UiAction::Quit => self.quit_requested = true,

            UiAction::Save => {
                match self.editor.save() {
                    Some(Err(e)) => self.editor.status = format!("Could not save: {e}"),
                    Some(Ok(())) => {}
                    // No path yet, so fall through to Save As.
                    None => self.perform(UiAction::SaveAs),
                }
            }

            UiAction::SaveAs => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Diagram", &["lucid"])
                    .set_file_name("diagram.lucid")
                    .save_file()
                {
                    if let Err(e) = self.editor.save_path(&path) {
                        self.editor.status = format!("Could not save: {e}");
                    }
                }
            }

            UiAction::Undo => self.editor.undo(),
            UiAction::Redo => self.editor.redo(),

            UiAction::Cut => self.copy_selection(true),
            UiAction::Copy => self.copy_selection(false),
            UiAction::Paste => self.paste(),

            UiAction::SelectAll => self.editor.select_all(),
            UiAction::SelectNone => self.editor.clear_selection(),
            UiAction::SelectAllShapes => self.editor.select_all_shapes(),
            UiAction::SelectAllLines => self.editor.select_all_lines(),
            UiAction::SelectMatching(by) => self.editor.select_matching(by),

            UiAction::SelectLibrary(library) => self.view.library = library,
            UiAction::FocusPaletteSearch => self.view.palette_search_focused = true,

            UiAction::FocusLinkField => {
                self.view.link_edit = match self.editor.selection_link() {
                    Some(dc_model::Action::Url { url, .. }) => url,
                    _ => String::new(),
                };
                self.view.link_edit_focused = true;
            }
            UiAction::ClearLink => {
                self.editor.clear_link();
                self.view.link_edit.clear();
                self.view.link_edit_focused = false;
            }
            UiAction::OpenLink => match self.editor.selection_link() {
                Some(dc_model::Action::Url { url, .. }) => open_url(&url),
                Some(dc_model::Action::GotoPage { page_id }) => {
                    if let Some(index) =
                        self.editor.document().pages.iter().position(|p| p.id == page_id)
                    {
                        self.editor.select_page(index);
                    }
                }
                _ => {}
            },

            UiAction::AddCustomData => self.editor.add_custom_data(),
            UiAction::RemoveCustomData(index) => {
                self.editor.remove_custom_data(index);
                if self.view.custom_data_focus == Some(crate::ui::CustomDataFocus::Key(index))
                    || self.view.custom_data_focus == Some(crate::ui::CustomDataFocus::Value(index))
                {
                    self.view.custom_data_focus = None;
                    self.view.custom_data_edit.clear();
                }
            }
            UiAction::FocusCustomDataKey(index) => {
                let current = self.editor.selection_custom_data();
                self.view.custom_data_edit = current.get(index).map(|e| e.key.clone()).unwrap_or_default();
                self.view.custom_data_focus = Some(crate::ui::CustomDataFocus::Key(index));
            }
            UiAction::FocusCustomDataValue(index) => {
                let current = self.editor.selection_custom_data();
                self.view.custom_data_edit = current.get(index).map(|e| e.value.clone()).unwrap_or_default();
                self.view.custom_data_focus = Some(crate::ui::CustomDataFocus::Value(index));
            }

            UiAction::ToggleLayersPanel => {
                self.view.layers_panel_open = !self.view.layers_panel_open;
            }
            UiAction::AddLayer => {
                self.editor.add_layer();
            }
            UiAction::DeleteLayer(id) => self.editor.delete_layer(&id),
            UiAction::ToggleLayerVisibility(id) => self.editor.toggle_layer_visibility(&id),
            UiAction::ToggleLayerLock(id) => self.editor.toggle_layer_lock(&id),
            UiAction::MoveSelectionToLayer(id) => self.editor.move_selection_to_layer(&id),

            UiAction::ToggleFind => {
                self.view.find_open = !self.view.find_open;
                if !self.view.find_open {
                    self.view.find_query_focused = false;
                    self.view.find_replacement_focused = false;
                }
            }
            UiAction::FocusFindQuery => self.view.find_query_focused = true,
            UiAction::FocusFindReplacement => self.view.find_replacement_focused = true,
            UiAction::FindNext => self.step_find_match(1),
            UiAction::FindPrevious => self.step_find_match(-1),
            UiAction::ReplaceCurrentMatch => {
                let matches = self.editor.find_matches(&self.view.find_query);
                if let Some(id) = matches.get(self.view.find_match_index) {
                    self.editor.replace_in(id, &self.view.find_query, &self.view.find_replacement);
                }
            }

            UiAction::Align(how) => self.editor.align_selection(how),
            UiAction::Distribute(horizontal) => self.editor.distribute_selection(horizontal),
            UiAction::MatchSize(w, h) => self.editor.match_size(w, h),
            UiAction::SetLocked(locked) => self.editor.set_selection_locked(locked),
            UiAction::UnlockAll => self.editor.unlock_all(),
            UiAction::TidyLayout(vertical) => {
                self.editor.tidy_layout(vertical);
            }
            UiAction::FitContainer => self.editor.fit_container_to_contents(),
            UiAction::ClearGuides => self.editor.clear_guides(),
            UiAction::GuidesFromSelection => self.editor.guides_from_selection(),
            UiAction::ColorByField(field) => {
                self.editor.color_by_field(&field);
            }
            UiAction::ApplyFormatRules => {
                self.editor.apply_format_rules();
            }
            UiAction::ClearFormatRules => self.editor.clear_format_rules(),

            UiAction::Group => self.editor.group_selection(),
            UiAction::Ungroup => self.editor.ungroup_selection(),

            UiAction::ZoomIn => {
                let step = self.view.settings.zoom_step;
                self.editor.zoom_by(step);
            }
            UiAction::ZoomOut => {
                let step = self.view.settings.zoom_step;
                self.editor.zoom_by(1.0 / step);
            }
            UiAction::ZoomFit => self.editor.zoom_to_fit(),
            UiAction::ZoomToSelection => self.editor.zoom_to_selection(),
            UiAction::ActualSize => self.editor.reset_zoom(),

            // The View menu's toggles are the same preferences the settings
            // screen holds, so they go through the same door.
            UiAction::ToggleGrid => {
                let on = !self.view.settings.show_grid;
                self.change_setting(SettingChange::ShowGrid(on));
            }
            UiAction::TogglePageBounds => {
                let on = !self.view.settings.show_page_bounds;
                self.change_setting(SettingChange::ShowPageBounds(on));
            }
            UiAction::ToggleRouting => {
                let on = !self.view.settings.route_around_shapes;
                self.change_setting(SettingChange::RouteAroundShapes(on));
            }
            UiAction::ToggleSnap => {
                let on = !self.view.settings.snap_to_objects;
                self.change_setting(SettingChange::SnapToObjects(on));
            }
            UiAction::ToggleTheme => {
                let on = !self.view.settings.dark_theme;
                self.change_setting(SettingChange::DarkTheme(on));
            }

            UiAction::OpenSettings => {
                self.editor.commit_text_edit();
                self.open_menu = None;
                self.context_menu = None;
                self.view.settings_open = true;
            }
            UiAction::CloseSettings => self.view.settings_open = false,
            UiAction::ChangeSetting(change) => self.change_setting(change),

            UiAction::ShowShortcuts => {
                self.editor.status = SHORTCUT_HINT.into();
            }
            UiAction::ShowAbout => {
                self.editor.status = format!(
                    "DiagramCraft {} — open-source diagramming in pure Rust",
                    env!("CARGO_PKG_VERSION")
                );
            }

            UiAction::SetFill(color) => self.editor.set_selection_fill(color),
            UiAction::SetStroke(color) => self.editor.set_selection_stroke(color, None),
            UiAction::SetStrokeWidth(w) => {
                // Width alone should not change the color, so reuse whatever
                // the first selected item already has.
                let existing = self
                    .editor
                    .selection
                    .first()
                    .and_then(|id| self.editor.page().shape(id))
                    .map(|s| s.effective_stroke().resolved_color())
                    .unwrap_or(dc_model::Color::rgb(0x33, 0x3a, 0x45));
                self.editor.set_selection_stroke(existing, Some(w));
            }
            UiAction::SetStrokeStyle(style) => self.editor.set_selection_stroke_style(style),
            UiAction::SetLineType(t) => self.editor.set_selection_line_type(t),

            UiAction::ToggleText(which) => self.editor.toggle_text_style(which),
            UiAction::ScaleTextSize(factor) => self.editor.scale_text_size(factor),
            UiAction::SetTextAlign(align) => self.editor.set_text_align(align),
            UiAction::SetVerticalAlign(align) => self.editor.set_vertical_align(align),
            UiAction::SetTextColor(color) => self.editor.set_text_color(color),
            UiAction::SetFontFamily(family) => self.editor.set_font_family(family),
            UiAction::EditLabel => {
                let mut cx = Self::text_ctx(&mut self.document_renderer);
                self.editor.begin_text_edit_on_selection(&mut cx);
            }

            UiAction::Delete => self.editor.delete_selection(),
            UiAction::Duplicate => self.editor.duplicate_selection(),
            UiAction::BringToFront => self.editor.reorder_selection(true),
            UiAction::BringForward => self.editor.nudge_z_order(true),
            UiAction::SendBackward => self.editor.nudge_z_order(false),
            UiAction::SendToBack => self.editor.reorder_selection(false),

            UiAction::SetEndpointStyle(end, style) => {
                self.editor.set_endpoint_style(end, style);
            }
            UiAction::SwapEndpoints => self.editor.swap_endpoints(),

            UiAction::SetOpacity(percent) => self.editor.set_opacity(percent),
            UiAction::SetCornerRounding(radius) => self.editor.set_corner_rounding(radius),
            UiAction::Rotate(degrees) => self.editor.rotate_selection(degrees),
            UiAction::Flip(horizontal) => self.editor.flip_selection(horizontal),
            UiAction::ChangeShapeType(t) => self.editor.change_shape_type(t),
            UiAction::CopyStyle => {
                self.editor.copy_style();
            }
            UiAction::PasteStyle => self.editor.paste_style(),

            UiAction::SelectPage(i) => self.editor.select_page(i),
            UiAction::AddPage => self.editor.add_page(),
            UiAction::DeletePage(i) => {
                self.editor.delete_page(i);
            }
            UiAction::DuplicatePage(i) => self.editor.duplicate_page(i),
            UiAction::MovePage { from, to } => {
                self.editor.move_page(from, to);
            }
            UiAction::RenamePage(i) => {
                // The tab bar is where a page title is edited, so the command
                // opens the tab rather than a dialog.
                let chrome = self.chrome();
                if let Some(rect) = chrome.tab_rects.get(i).copied() {
                    let surface = self.document_renderer.theme.panel;
                    let mut cx = Self::text_ctx(&mut self.document_renderer);
                    self.editor.begin_page_title_edit(i, rect, surface, &mut cx);
                }
            }
        }

        self.update_title();
    }

    /// Records a preference, applies it, and writes it out.
    ///
    /// Saving on every change rather than on closing the screen means a
    /// preference survives even if the program never gets a clean exit.
    fn change_setting(&mut self, change: SettingChange) {
        change.apply(&mut self.view.settings);
        self.apply_settings();

        if self.persist_settings {
            if let Err(e) = self.view.settings.save() {
                log::warn!("could not save settings: {e}");
                self.editor.status = format!("Could not save settings: {e}");
            }
        }
    }

    /// Pushes the preferences into the parts of the editor that honour them.
    ///
    /// One place does this, so a preference cannot be read from the settings
    /// in one spot and from a stale copy in another.
    fn apply_settings(&mut self) {
        let s = &self.view.settings;

        let options = &mut self.document_renderer.options;
        options.show_grid = s.show_grid;
        options.grid_style = s.grid_style;
        options.grid_spacing = s.grid_spacing;
        options.show_page_bounds = s.show_page_bounds;
        options.route_around_shapes = s.route_around_shapes;

        self.document_renderer.theme = if s.dark_theme {
            Theme::dark()
        } else {
            Theme::light()
        };

        self.editor.snap_enabled = s.snap_to_objects;
        self.editor.snap_to_grid = s.snap_to_grid;
        self.editor.snap_threshold_px = s.snap_distance;
        self.editor.grid_spacing = s.grid_spacing;
        self.editor.default_line_type = s.default_line_type;
        self.editor.default_units = s.default_units;
        self.editor.default_page_size = s.default_page_size.clone();
    }

    /// Runs a command against the open label rather than the document.
    ///
    /// Returns whether it was handled here. Formatting commands are absent on
    /// purpose: they already act on the selected characters when a label is
    /// open, so they need no second path.
    fn perform_while_editing(&mut self, action: &UiAction) -> bool {
        match action {
            UiAction::Copy => {
                self.copy_selected_text();
                true
            }
            UiAction::Cut => {
                self.copy_selected_text();
                let mut cx = Self::text_ctx(&mut self.document_renderer);
                self.editor.text_command(TextCommand::Backspace, &mut cx);
                true
            }
            UiAction::Paste => {
                if let Some(text) = self.clipboard_text() {
                    let mut cx = Self::text_ctx(&mut self.document_renderer);
                    self.editor.text_command(TextCommand::Insert(text), &mut cx);
                }
                true
            }
            UiAction::SelectAll => {
                let mut cx = Self::text_ctx(&mut self.document_renderer);
                self.editor.text_command(TextCommand::SelectAll, &mut cx);
                true
            }
            UiAction::SelectNone => {
                self.editor.cancel_text_edit();
                true
            }
            // Undoing mid-edit would otherwise leave the caret inside a label
            // the document no longer has.
            UiAction::Undo => {
                self.editor.cancel_text_edit();
                self.editor.undo();
                true
            }
            UiAction::Redo => {
                self.editor.cancel_text_edit();
                self.editor.redo();
                true
            }
            // Everything that leaves the document: finish the label first, so
            // what was typed is in what gets written or drawn.
            UiAction::Save | UiAction::SaveAs | UiAction::ExportPng | UiAction::Open
            | UiAction::ImportVsdx | UiAction::NewDocument | UiAction::OpenTemplateGallery
            | UiAction::NewFromTemplate(_) => {
                self.editor.commit_text_edit();
                false
            }
            _ => false,
        }
    }

    /// Opens a file, choosing the reader from its extension.
    fn open_any(&mut self, path: &std::path::Path) {
        let outcome = if dc_vsdx::is_vsdx_path(path) {
            self.editor.import_vsdx(path).map_err(|e| e.to_string())
        } else {
            self.editor.open_path(path).map_err(|e| e.to_string())
        };
        if let Err(e) = outcome {
            self.editor.status = format!("Could not open {}: {e}", path.display());
            log::error!("could not open {}: {e}", path.display());
        }
        self.sync_images();
        self.sync_canvas_viewport();
    }
}

/// Opens a URL in the system's default browser.
///
/// Shelled out per platform rather than pulled in as a dependency — the
/// same reasoning `native_menu.rs` uses for platform differences elsewhere
/// in this app.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

/// The window icon, for the platforms that have one.
///
/// An icon that will not decode is not worth refusing to start over, so a
/// failure here leaves the window with the system default.
fn window_icon() -> Option<winit::window::Icon> {
    let (rgba, size) = crate::icon::decoded()?;
    match winit::window::Icon::from_rgba(rgba, size, size) {
        Ok(icon) => Some(icon),
        Err(e) => {
            log::warn!("could not build the window icon: {e}");
            None
        }
    }
}

/// What the Help menu's shortcut command puts in the status bar.
const SHORTCUT_HINT: &str = "V select · H pan · C connect · ⌘Z undo · ⌘S save · \
⌘G group · ⌘1 fit · ⌘0 actual size · drag a page tab to reorder";

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::MenuItem(id) => self.on_menu_item(&id),
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Both of these belong to the application rather than to the window,
        // so they are set once the application exists and survive the window
        // being recreated.
        self.install_native_menus();
        self.set_dock_icon();

        if matches!(self.surface, Surface::Live { .. }) {
            return;
        }

        let attributes = Window::default_attributes()
            .with_window_icon(window_icon())
            .with_title("Untitled — DiagramCraft")
            .with_inner_size(winit::dpi::LogicalSize::new(1440.0, 900.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(640.0, 480.0));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                log::error!("could not create a window: {e}");
                event_loop.exit();
                return;
            }
        };

        self.scale_factor = window.scale_factor();
        let physical = window.inner_size();
        self.window_size = Vec2::new(
            physical.width as f64 / self.scale_factor,
            physical.height as f64 / self.scale_factor,
        );

        let surface = pollster::block_on(self.context.create_surface(
            window.clone(),
            physical.width.max(1),
            physical.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ));
        let surface = match surface {
            Ok(surface) => surface,
            Err(e) => {
                log::error!("could not create a GPU surface: {e}");
                event_loop.exit();
                return;
            }
        };

        // One renderer per device, created lazily.
        self.renderers.resize_with(self.context.devices.len(), || None);
        let device_id = surface.dev_id;
        if self.renderers[device_id].is_none() {
            let renderer = vello::Renderer::new(
                &self.context.devices[device_id].device,
                RendererOptions {
                    use_cpu: false,
                    antialiasing_support: AaSupport::area_only(),
                    num_init_threads: NonZeroUsize::new(1),
                    ..Default::default()
                },
            );
            match renderer {
                Ok(renderer) => self.renderers[device_id] = Some(renderer),
                Err(e) => {
                    log::error!("could not create the Vello renderer: {e}");
                    event_loop.exit();
                    return;
                }
            }
        }

        self.surface = Surface::Live { window, surface };
        self.sync_canvas_viewport();
        self.editor.status = "Drag a shape from the left panel to begin".into();
        self.update_title();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        if self.quit_requested {
            event_loop.exit();
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                self.on_resize(size.width, size.height);
                self.request_redraw();
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor;
                if let Surface::Live { window, .. } = &self.surface {
                    let physical = window.inner_size();
                    self.on_resize(physical.width, physical.height);
                }
                self.request_redraw();
            }

            WindowEvent::ModifiersChanged(new) => {
                let state: ModifiersState = new.state();
                self.mods = Mods {
                    shift: state.shift_key(),
                    // Treat Command and Control alike so one binding set works
                    // on macOS, Windows, and Linux.
                    command: state.super_key() || state.control_key(),
                    alt: state.alt_key(),
                };
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = kurbo::Point::new(
                    position.x / self.scale_factor,
                    position.y / self.scale_factor,
                );
                self.on_pointer_move();
            }

            WindowEvent::CursorLeft { .. } => {
                self.hovered_region = None;
                self.editor.hovered = None;
                self.request_redraw();
            }

            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => self.on_pointer_down(button),
                ElementState::Released => self.on_pointer_up(),
            },

            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta),

            WindowEvent::KeyboardInput { event, .. } => {
                let text = event.text.clone();
                self.on_key(&event.logical_key, event.state, text.as_deref());
            }

            WindowEvent::RedrawRequested => self.render(),

            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::LineType;

    impl App {
        /// Test shim: derives the typed text from the key the way a platform
        /// would, so tests can drive key handling with one argument.
        fn on_key_compat(&mut self, key: &Key, state: ElementState) {
            let text = match key {
                Key::Character(c) => Some(c.to_string()),
                Key::Named(NamedKey::Space) => Some(" ".to_string()),
                _ => None,
            };
            self.on_key(key, state, text.as_deref());
        }
    }

    /// Constructing the application must not require a GPU, so that the whole
    /// non-rendering surface stays testable in CI.
    fn app() -> App {
        App::with_settings(crate::settings::Settings::default())
    }

    #[test]
    fn a_new_app_has_no_surface_yet() {
        let a = app();
        assert!(matches!(a.surface, Surface::Pending));
        assert!(a.renderers.is_empty());
    }

    #[test]
    fn canvas_point_subtracts_the_panel_offset() {
        let a = app();
        let chrome = a.chrome();
        let window_point =
            kurbo::Point::new(chrome.layout.canvas.x0 + 12.0, chrome.layout.canvas.y0 + 34.0);
        assert_eq!(a.canvas_point(window_point, &chrome), Point::new(12.0, 34.0));
    }

    #[test]
    fn syncing_the_viewport_matches_the_canvas() {
        let mut a = app();
        a.sync_canvas_viewport();
        let expected = a.chrome().layout.canvas_size();
        assert_eq!(a.editor.camera.viewport, expected);
        assert!(expected.x > 0.0 && expected.y > 0.0);
    }

    #[test]
    fn toggling_the_theme_swaps_the_palette() {
        let mut a = app();
        assert!(!a.document_renderer.theme.is_dark());
        a.perform(UiAction::ToggleTheme);
        assert!(a.document_renderer.theme.is_dark());
        a.perform(UiAction::ToggleTheme);
        assert!(!a.document_renderer.theme.is_dark());
    }

    #[test]
    fn toggling_the_grid_flips_the_render_option() {
        let mut a = app();
        let before = a.document_renderer.options.show_grid;
        a.perform(UiAction::ToggleGrid);
        assert_eq!(a.document_renderer.options.show_grid, !before);
    }

    #[test]
    fn toggling_snap_flips_the_editor_flag() {
        let mut a = app();
        let before = a.editor.snap_enabled;
        a.perform(UiAction::ToggleSnap);
        assert_eq!(a.editor.snap_enabled, !before);
    }

    #[test]
    fn tool_actions_change_the_active_tool() {
        let mut a = app();
        a.perform(UiAction::SelectTool(Tool::Connector));
        assert_eq!(a.editor.tool, Tool::Connector);
    }

    #[test]
    fn page_actions_add_and_switch_pages() {
        let mut a = app();
        a.perform(UiAction::AddPage);
        assert_eq!(a.editor.document().pages.len(), 2);
        a.perform(UiAction::SelectPage(0));
        assert_eq!(a.editor.page_index, 0);
    }

    #[test]
    fn undo_and_redo_actions_reach_the_editor() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        assert_eq!(a.editor.page().shapes().len(), 1);

        a.perform(UiAction::Undo);
        assert_eq!(a.editor.page().shapes().len(), 0);
        a.perform(UiAction::Redo);
        assert_eq!(a.editor.page().shapes().len(), 1);
    }

    #[test]
    fn stroke_width_keeps_the_existing_color() {
        let mut a = app();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        let red = dc_model::Color::rgb(0xff, 0, 0);
        a.perform(UiAction::SetStroke(red));
        a.perform(UiAction::SetStrokeWidth(4.0));

        let stroke = a.editor.page().shape(&id).unwrap().effective_stroke();
        assert_eq!(stroke.resolved_color(), red, "the color should be preserved");
        assert_eq!(stroke.resolved_width(), 4.0);
    }

    #[test]
    fn line_type_action_reaches_connectors() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 40.0, 20.0),
        );
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(200.0, 0.0, 40.0, 20.0),
        );
        a.editor.tool = Tool::Connector;
        {
            let mut cx = App::text_ctx(&mut a.document_renderer);
            a.editor.pointer_down(Point::new(20.0, 10.0), Mods::default(), false, 1, &mut cx);
            a.editor.pointer_move(Point::new(220.0, 10.0), Mods::default(), &mut cx);
            a.editor.pointer_up(Point::new(220.0, 10.0), Mods::default(), &mut cx);
        }

        let line_id = a.editor.page().lines()[0].id.clone();
        a.editor.selection = vec![line_id.clone()];
        a.perform(UiAction::SetLineType(LineType::Curved));
        assert_eq!(a.editor.page().line(&line_id).unwrap().line_type, LineType::Curved);
    }

    #[test]
    fn space_bar_enters_and_leaves_pan_mode() {
        let mut a = app();
        a.editor.tool = Tool::Connector;

        a.on_key_compat(&Key::Named(NamedKey::Space), ElementState::Pressed);
        assert!(a.space_held);
        assert_eq!(a.tool_before_pan, Some(Tool::Connector));

        a.on_key_compat(&Key::Named(NamedKey::Space), ElementState::Released);
        assert!(!a.space_held);
        assert_eq!(a.editor.tool, Tool::Connector, "the tool should be restored");
    }

    #[test]
    fn escape_clears_selection_then_returns_to_select() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!(!a.editor.selection.is_empty());

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert!(a.editor.selection.is_empty());

        a.editor.tool = Tool::Connector;
        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert_eq!(a.editor.tool, Tool::Select);
    }

    #[test]
    fn arrow_keys_nudge_by_one_and_shift_by_ten() {
        let mut a = app();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(100.0, 100.0, 10.0, 10.0),
        );

        a.on_key_compat(&Key::Named(NamedKey::ArrowRight), ElementState::Pressed);
        assert_eq!(a.editor.page().shape(&id).unwrap().bounding_box.x, 101.0);

        a.mods.shift = true;
        a.on_key_compat(&Key::Named(NamedKey::ArrowRight), ElementState::Pressed);
        assert_eq!(a.editor.page().shape(&id).unwrap().bounding_box.x, 111.0);
    }

    #[test]
    fn delete_key_removes_the_selection() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        a.on_key_compat(&Key::Named(NamedKey::Delete), ElementState::Pressed);
        assert_eq!(a.editor.page().shapes().len(), 0);
    }

    #[test]
    fn single_letter_keys_pick_tools() {
        let mut a = app();
        for (ch, expected) in [
            ("v", Tool::Select),
            ("h", Tool::Pan),
            ("c", Tool::Connector),
            ("r", Tool::Shape(dc_model::ShapeType::Rectangle)),
            ("d", Tool::Shape(dc_model::ShapeType::Decision)),
            ("p", Tool::Shape(dc_model::ShapeType::Process)),
            ("t", Tool::Shape(dc_model::ShapeType::TextBlock)),
        ] {
            a.on_key_compat(&Key::Character(ch.into()), ElementState::Pressed);
            assert_eq!(a.editor.tool, expected, "key {ch}");
        }
    }

    #[test]
    fn command_z_undoes_and_shift_command_z_redoes() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        press_shortcut(&mut a, "⌘Z");
        assert_eq!(a.editor.page().shapes().len(), 0);

        press_shortcut(&mut a, "Shift ⌘Z");
        assert_eq!(a.editor.page().shapes().len(), 1);
    }

    #[test]
    fn command_a_selects_all_and_command_d_duplicates() {
        let mut a = app();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        press_shortcut(&mut a, "⌘A");
        assert_eq!(a.editor.selection.len(), 1);

        press_shortcut(&mut a, "⌘D");
        assert_eq!(a.editor.page().shapes().len(), 2);
    }

    #[test]
    fn keyboard_zoom_commands_change_the_camera() {
        let mut a = app();
        a.sync_canvas_viewport();

        let start = a.editor.camera.zoom;
        press_shortcut(&mut a, "⌘+");
        assert!(a.editor.camera.zoom > start);

        press_shortcut(&mut a, "⌘0");
        assert_eq!(a.editor.camera.zoom, 1.0);
    }

    #[test]
    fn letter_tool_keys_are_ignored_while_command_is_held() {
        let mut a = app();
        a.mods.command = true;
        a.on_key_compat(&Key::Character("r".into()), ElementState::Pressed);
        assert_eq!(a.editor.tool, Tool::Select, "command-R is not the shape tool");
    }

    #[test]
    fn resizing_updates_the_logical_window_size() {
        let mut a = app();
        a.scale_factor = 2.0;
        a.on_resize(2880, 1800);
        assert_eq!(a.window_size, Vec2::new(1440.0, 900.0));
    }

    #[test]
    fn a_zero_sized_resize_is_ignored() {
        let mut a = app();
        let before = a.window_size;
        a.on_resize(0, 0);
        assert_eq!(a.window_size, before);
    }

    #[test]
    fn clicking_a_toolbar_button_performs_its_action() {
        let mut a = app();
        let chrome = a.chrome();
        let index = chrome
            .regions
            .iter()
            .position(|r| r.action == UiAction::SelectTool(Tool::Connector))
            .unwrap();
        a.pointer = chrome.regions[index].rect.center();

        a.on_pointer_down(MouseButton::Left);
        assert_eq!(a.editor.tool, Tool::Connector);
        assert!(!a.dragging_canvas, "a chrome click is not a canvas drag");
    }

    #[test]
    fn clicking_the_canvas_starts_a_canvas_drag() {
        let mut a = app();
        a.sync_canvas_viewport();
        let chrome = a.chrome();
        a.pointer = chrome.layout.canvas.center();

        a.on_pointer_down(MouseButton::Left);
        assert!(a.dragging_canvas);

        a.on_pointer_up();
        assert!(!a.dragging_canvas);
    }

    #[test]
    fn clicking_a_panel_does_not_start_a_canvas_drag() {
        let mut a = app();
        a.sync_canvas_viewport();
        let chrome = a.chrome();
        // An empty spot low in the left panel, below the shape tiles.
        a.pointer = kurbo::Point::new(chrome.layout.left_panel.x0 + 4.0, chrome.layout.left_panel.y1 - 4.0);

        a.on_pointer_down(MouseButton::Left);
        assert!(!a.dragging_canvas);
    }

    #[test]
    fn drawing_a_shape_through_the_canvas_works_end_to_end() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.editor.tool = Tool::Shape(dc_model::ShapeType::Process);

        let chrome = a.chrome();
        let start = chrome.layout.canvas.center();
        a.pointer = start;
        a.on_pointer_down(MouseButton::Left);
        a.pointer = kurbo::Point::new(start.x + 120.0, start.y + 60.0);
        a.on_pointer_move();
        a.on_pointer_up();

        assert_eq!(a.editor.page().shapes().len(), 1);
        let bb = a.editor.page().shapes()[0].bounding_box;
        assert_eq!((bb.w, bb.h), (120.0, 60.0));
    }

    #[test]
    fn dragging_a_shape_tile_straight_onto_the_canvas_drops_it_there() {
        // The status hint says "drag a shape from the left panel to begin",
        // so a press-drag-release that never lets go of the button must
        // place the shape, not just arm the tool for a later click.
        let mut a = app();
        a.sync_canvas_viewport();
        let chrome = a.chrome();
        let tile = chrome
            .regions
            .iter()
            .find(|r| matches!(r.kind, crate::ui::RegionKind::ShapeTile(_)))
            .expect("a shape tile");
        let shape_type = match &tile.kind {
            crate::ui::RegionKind::ShapeTile(t) => t.clone(),
            _ => unreachable!(),
        };
        a.pointer = tile.rect.center();

        a.on_pointer_down(MouseButton::Left);
        assert!(a.editor.page().shapes().is_empty(), "pressing alone places nothing yet");

        // The pointer crosses onto the canvas, then keeps moving to settle
        // on the spot the user actually wants — this must not get read as
        // a box being drawn out from where it first crossed in.
        let entry = chrome.layout.canvas.center();
        a.pointer = entry;
        a.on_pointer_move();
        let drop = kurbo::Point::new(entry.x + 150.0, entry.y + 80.0);
        a.pointer = drop;
        a.on_pointer_move();
        a.on_pointer_up();

        assert_eq!(
            a.editor.page().shapes().len(),
            1,
            "dragging from the tile onto the canvas should drop a shape"
        );
        let bb = a.editor.page().shapes()[0].bounding_box;
        let (w, h) = shape_type.default_size();
        assert_eq!(
            (bb.w, bb.h),
            (w, h),
            "a drag-and-drop places the shape at its default size, not a size drawn by the gesture"
        );
        let expected_center = a.editor.camera.screen_to_doc(a.canvas_point(drop, &chrome));
        assert!(
            (bb.center().x - expected_center.x).abs() < 0.01 && (bb.center().y - expected_center.y).abs() < 0.01,
            "the shape should be centered where the pointer let go, not where it entered the canvas"
        );
    }

    #[test]
    fn wheel_over_a_panel_does_not_scroll_the_canvas() {
        let mut a = app();
        a.sync_canvas_viewport();
        let before = a.editor.camera.origin;
        a.pointer = kurbo::Point::new(10.0, 10.0);
        a.on_wheel(MouseScrollDelta::LineDelta(0.0, 3.0));
        assert_eq!(a.editor.camera.origin, before);
    }

    #[test]
    fn wheel_over_the_canvas_scrolls_it() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.pointer = a.chrome().layout.canvas.center();
        let before = a.editor.camera.origin;
        a.on_wheel(MouseScrollDelta::LineDelta(0.0, 3.0));
        assert_ne!(a.editor.camera.origin, before);
    }

    // -------------------------------------------------- right-click menu

    /// Where a window-space point lands in document coordinates, so a test
    /// shape can be placed exactly under a click rather than guessing.
    fn doc_point_at(a: &App, window_point: kurbo::Point) -> Point {
        let chrome = a.chrome();
        let local = a.canvas_point(window_point, &chrome);
        a.editor.camera.screen_to_doc(local)
    }

    #[test]
    fn right_clicking_blank_canvas_opens_the_canvas_menu() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.pointer = a.chrome().layout.canvas.center();

        a.on_pointer_down(MouseButton::Right);

        let (target, at) = a.context_menu.expect("a context menu should be up");
        assert_eq!(target, crate::ui::ContextTarget::Canvas);
        assert_eq!(at, a.pointer);
        assert!(a.editor.selection.is_empty());
    }

    #[test]
    fn right_clicking_a_shape_selects_it_and_opens_its_menu() {
        let mut a = app();
        a.sync_canvas_viewport();
        let center = a.chrome().layout.canvas.center();
        let doc = doc_point_at(&a, center);
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(doc.x - 50.0, doc.y - 30.0, 100.0, 60.0),
        );
        a.editor.selection.clear();

        a.pointer = center;
        a.on_pointer_down(MouseButton::Right);

        assert_eq!(a.editor.selection, vec![id]);
        let (target, _) = a.context_menu.expect("a context menu should be up");
        assert_eq!(target, crate::ui::ContextTarget::Canvas);
    }

    #[test]
    fn right_clicking_inside_a_multi_selection_leaves_it_intact() {
        let mut a = app();
        a.sync_canvas_viewport();
        let center = a.chrome().layout.canvas.center();
        let doc = doc_point_at(&a, center);
        let first = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(doc.x - 10.0, doc.y - 10.0, 20.0, 20.0));
        let second = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(doc.x + 200.0, doc.y - 10.0, 20.0, 20.0),
        );
        a.editor.selection = vec![first.clone(), second.clone()];

        a.pointer = center;
        a.on_pointer_down(MouseButton::Right);

        let mut selection = a.editor.selection.clone();
        selection.sort();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(selection, expected, "right-clicking a member of the selection must not shrink it");
    }

    #[test]
    fn right_clicking_a_page_tab_opens_its_menu_without_switching_pages() {
        let mut a = app_with_window_menus();
        a.editor.add_page();
        let active_before = a.editor.page_index;

        let chrome = a.chrome();
        let tab = chrome
            .regions
            .iter()
            .find(|r| r.action == UiAction::SelectPage(0))
            .expect("the first tab");
        a.pointer = tab.rect.center();

        a.on_pointer_down(MouseButton::Right);

        assert_eq!(a.editor.page_index, active_before, "a right click must not switch pages");
        let (target, _) = a.context_menu.expect("a context menu should be up");
        assert_eq!(target, crate::ui::ContextTarget::Page(0));
    }

    #[test]
    fn clicking_a_context_menu_entry_runs_it_and_closes_the_menu() {
        let mut a = app();
        a.sync_canvas_viewport();
        let center = a.chrome().layout.canvas.center();
        let doc = doc_point_at(&a, center);
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(doc.x - 50.0, doc.y - 30.0, 100.0, 60.0),
        );
        a.pointer = center;
        a.on_pointer_down(MouseButton::Right);
        a.on_pointer_up();

        let chrome = a.chrome();
        let entry = chrome
            .regions
            .iter()
            .find(|r| r.label == "Delete")
            .expect("a Delete entry");
        a.pointer = entry.rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(a.context_menu.is_none(), "running a command closes the menu");
        assert!(a.editor.page().shape(&id).is_none(), "the shape should be gone");
    }

    #[test]
    fn clicking_outside_the_context_menu_dismisses_it() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.pointer = a.chrome().layout.canvas.center();
        a.on_pointer_down(MouseButton::Right);
        a.on_pointer_up();
        assert!(a.context_menu.is_some());

        a.pointer = kurbo::Point::new(2.0, 2.0);
        a.on_pointer_down(MouseButton::Left);
        assert!(a.context_menu.is_none());
    }

    #[test]
    fn escape_closes_the_context_menu_before_touching_the_selection() {
        let mut a = app();
        a.sync_canvas_viewport();
        let center = a.chrome().layout.canvas.center();
        let doc = doc_point_at(&a, center);
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(doc.x - 50.0, doc.y - 30.0, 100.0, 60.0),
        );
        a.pointer = center;
        a.on_pointer_down(MouseButton::Right);
        a.on_pointer_up();
        assert!(a.context_menu.is_some());

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert!(a.context_menu.is_none());
        assert_eq!(a.editor.selection, vec![id], "the selection should survive");
    }

    // ------------------------------------------------------ palette search

    fn search_field_rect(a: &App) -> kurbo::Rect {
        a.chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::FocusPaletteSearch)
            .expect("a search field")
            .rect
    }

    #[test]
    fn clicking_the_search_field_focuses_it_and_typing_fills_it() {
        let mut a = app();
        a.pointer = search_field_rect(&a).center();
        a.on_pointer_down(MouseButton::Left);
        assert!(a.view.palette_search_focused);

        for c in ['s', 't', 'a', 'r'] {
            a.on_key_compat(&Key::Character(c.to_string().into()), ElementState::Pressed);
        }
        assert_eq!(a.view.palette_search, "star");
    }

    #[test]
    fn backspace_removes_the_last_character() {
        let mut a = app();
        a.view.palette_search_focused = true;
        a.view.palette_search = "star".into();
        a.on_key_compat(&Key::Named(NamedKey::Backspace), ElementState::Pressed);
        assert_eq!(a.view.palette_search, "sta");
    }

    #[test]
    fn escape_clears_the_search_and_unfocuses_it() {
        let mut a = app();
        a.view.palette_search_focused = true;
        a.view.palette_search = "star".into();
        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert!(a.view.palette_search.is_empty());
        assert!(!a.view.palette_search_focused);
    }

    #[test]
    fn enter_unfocuses_but_keeps_the_query() {
        let mut a = app();
        a.view.palette_search_focused = true;
        a.view.palette_search = "star".into();
        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);
        assert_eq!(a.view.palette_search, "star");
        assert!(!a.view.palette_search_focused);
    }

    #[test]
    fn clicking_elsewhere_unfocuses_the_search_field_but_keeps_its_text() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.view.palette_search_focused = true;
        a.view.palette_search = "star".into();

        a.pointer = a.chrome().layout.canvas.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.palette_search_focused);
        assert_eq!(a.view.palette_search, "star");
    }

    #[test]
    fn typed_characters_do_not_leak_into_shortcuts_while_searching() {
        // Bare "c" normally selects the Connector tool; if it did that here
        // too, the key would have reached the shortcut dispatcher instead
        // of landing only in the query.
        let mut a = app();
        a.view.palette_search_focused = true;
        a.editor.tool = Tool::Select;
        a.on_key_compat(&Key::Character("c".into()), ElementState::Pressed);
        assert_eq!(a.view.palette_search, "c");
        assert_eq!(a.editor.tool, Tool::Select, "a shortcut must not have fired");
    }

    // ------------------------------------------------------------ link field

    fn link_field_rect(a: &App) -> kurbo::Rect {
        a.chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::FocusLinkField)
            .expect("a link field")
            .rect
    }

    #[test]
    fn clicking_the_link_field_focuses_it_and_typing_fills_it() {
        let mut a = app();
        a.sync_canvas_viewport();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];

        a.pointer = link_field_rect(&a).center();
        a.on_pointer_down(MouseButton::Left);
        assert!(a.view.link_edit_focused);

        for c in "https://x".chars() {
            a.on_key_compat(&Key::Character(c.to_string().into()), ElementState::Pressed);
        }
        assert_eq!(a.view.link_edit, "https://x");
    }

    #[test]
    fn enter_commits_the_typed_link() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.view.link_edit_focused = true;
        a.view.link_edit = "https://example.com".into();

        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);

        assert!(!a.view.link_edit_focused);
        assert_eq!(
            a.editor.selection_link(),
            Some(dc_model::Action::Url { url: "https://example.com".into(), new_window: None })
        );
    }

    #[test]
    fn escape_cancels_without_touching_the_link() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.editor.set_link_url("https://kept.example");
        a.view.link_edit_focused = true;
        a.view.link_edit = "https://discarded.example".into();

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);

        assert!(!a.view.link_edit_focused);
        assert_eq!(
            a.editor.selection_link(),
            Some(dc_model::Action::Url { url: "https://kept.example".into(), new_window: None })
        );
    }

    #[test]
    fn clicking_elsewhere_commits_the_link_field() {
        let mut a = app();
        a.sync_canvas_viewport();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.view.link_edit_focused = true;
        a.view.link_edit = "https://example.com".into();

        a.pointer = kurbo::Point::new(2.0, 2.0);
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.link_edit_focused);
        assert_eq!(
            a.editor.selection_link(),
            Some(dc_model::Action::Url { url: "https://example.com".into(), new_window: None })
        );
    }

    #[test]
    fn an_empty_field_committed_removes_the_link() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.editor.set_link_url("https://example.com");
        a.view.link_edit_focused = true;
        a.view.link_edit = String::new();

        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);
        assert_eq!(a.editor.selection_link(), None);
    }

    #[test]
    fn clicking_remove_link_clears_it() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.editor.set_link_url("https://example.com");

        let remove = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::ClearLink)
            .expect("a Remove Link button")
            .rect;
        a.pointer = remove.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.selection_link(), None);
    }

    // ------------------------------------------------------- custom data

    #[test]
    fn clicking_add_field_then_typing_a_key_and_value() {
        let mut a = app();
        a.sync_canvas_viewport();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];

        let add = a.chrome().regions.iter().find(|r| r.action == UiAction::AddCustomData).unwrap().rect;
        a.pointer = add.center();
        a.on_pointer_down(MouseButton::Left);
        assert_eq!(a.editor.selection_custom_data().len(), 1);

        let key_rect =
            a.chrome().regions.iter().find(|r| r.action == UiAction::FocusCustomDataKey(0)).unwrap().rect;
        a.pointer = key_rect.center();
        a.on_pointer_down(MouseButton::Left);
        assert_eq!(a.view.custom_data_focus, Some(crate::ui::CustomDataFocus::Key(0)));
        for c in "owner".chars() {
            a.on_key_compat(&Key::Character(c.to_string().into()), ElementState::Pressed);
        }
        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);

        assert_eq!(a.editor.selection_custom_data()[0].key, "owner");
        assert!(a.view.custom_data_focus.is_none());
    }

    #[test]
    fn clicking_a_different_field_commits_the_one_before_it() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.editor.add_custom_data();
        a.view.custom_data_focus = Some(crate::ui::CustomDataFocus::Key(0));
        a.view.custom_data_edit = "owner".into();

        let value_rect = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::FocusCustomDataValue(0))
            .unwrap()
            .rect;
        a.pointer = value_rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.selection_custom_data()[0].key, "owner", "the key should have been saved");
        assert_eq!(a.view.custom_data_focus, Some(crate::ui::CustomDataFocus::Value(0)));
    }

    #[test]
    fn removing_a_field_clears_its_focus_if_it_was_focused() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 60.0));
        a.editor.selection = vec![id];
        a.editor.add_custom_data();
        a.view.custom_data_focus = Some(crate::ui::CustomDataFocus::Key(0));
        a.view.custom_data_edit = "half-typed".into();

        let remove =
            a.chrome().regions.iter().find(|r| r.action == UiAction::RemoveCustomData(0)).unwrap().rect;
        a.pointer = remove.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(a.editor.selection_custom_data().is_empty());
        assert!(a.view.custom_data_focus.is_none());
    }

    // --------------------------------------------------------- layers panel

    #[test]
    fn toggling_the_layers_panel_from_the_view_menu() {
        let mut a = app();
        assert!(!a.view.layers_panel_open);
        a.perform(UiAction::ToggleLayersPanel);
        assert!(a.view.layers_panel_open);
        a.perform(UiAction::ToggleLayersPanel);
        assert!(!a.view.layers_panel_open);
    }

    #[test]
    fn clicking_add_layer_creates_one() {
        let mut a = app();
        a.view.layers_panel_open = true;
        assert!(a.editor.layers().is_empty());

        let add = a.chrome().regions.iter().find(|r| r.action == UiAction::AddLayer).unwrap().rect;
        a.pointer = add.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.layers().len(), 1);
    }

    #[test]
    fn clicking_vis_and_lock_toggles_the_layer() {
        let mut a = app();
        a.view.layers_panel_open = true;
        let id = a.editor.add_layer();

        let vis_rect = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::ToggleLayerVisibility(id.clone()))
            .unwrap()
            .rect;
        a.pointer = vis_rect.center();
        a.on_pointer_down(MouseButton::Left);
        assert!(!a.editor.layers()[0].is_visible());

        let lock_rect = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::ToggleLayerLock(id.clone()))
            .unwrap()
            .rect;
        a.pointer = lock_rect.center();
        a.on_pointer_down(MouseButton::Left);
        assert!(a.editor.layers()[0].is_locked());
    }

    #[test]
    fn clicking_use_moves_the_selection_into_the_layer() {
        let mut a = app();
        a.view.layers_panel_open = true;
        let id = a.editor.add_layer();
        let shape = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0));
        a.editor.selection = vec![shape.clone()];

        let use_rect = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::MoveSelectionToLayer(id.clone()))
            .unwrap()
            .rect;
        a.pointer = use_rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.layer_of(&shape).map(|l| l.id.clone()), Some(id));
    }

    #[test]
    fn clicking_del_removes_the_layer() {
        let mut a = app();
        a.view.layers_panel_open = true;
        let id = a.editor.add_layer();

        let del_rect = a.chrome().regions.iter().find(|r| r.action == UiAction::DeleteLayer(id.clone())).unwrap().rect;
        a.pointer = del_rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(a.editor.layers().is_empty());
    }

    #[test]
    fn a_click_inside_the_empty_part_of_the_layers_panel_does_not_reach_the_canvas() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.view.layers_panel_open = true;
        let before = a.editor.page().shapes().len();

        let panel = a.chrome().layers_panel.expect("a layers panel");
        a.pointer = kurbo::Point::new(panel.x0 + 4.0, panel.y1 - 4.0);
        a.on_pointer_down(MouseButton::Left);
        a.on_pointer_up();

        assert_eq!(a.editor.page().shapes().len(), before, "nothing should have been drawn on the canvas");
    }

    // --------------------------------------------------------- find and replace

    #[test]
    fn toggling_find_from_the_edit_menu_or_shortcut() {
        let mut a = app();
        assert!(!a.view.find_open);
        a.perform(UiAction::ToggleFind);
        assert!(a.view.find_open);
        a.perform(UiAction::ToggleFind);
        assert!(!a.view.find_open);
    }

    #[test]
    fn typing_a_query_and_stepping_through_matches() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.view.find_open = true;
        let a_id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        a.editor.package.document.pages[0].shape_mut(&a_id).unwrap().text =
            Some(dc_model::Text::new("Approval A"));
        let b_id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(300.0, 0.0, 100.0, 40.0),
        );
        a.editor.package.document.pages[0].shape_mut(&b_id).unwrap().text =
            Some(dc_model::Text::new("Approval B"));

        let field = a.chrome().regions.iter().find(|r| r.action == UiAction::FocusFindQuery).unwrap().rect;
        a.pointer = field.center();
        a.on_pointer_down(MouseButton::Left);
        assert!(a.view.find_query_focused);
        for c in "approval".chars() {
            a.on_key_compat(&Key::Character(c.to_string().into()), ElementState::Pressed);
        }

        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);
        let first = a.editor.selection.clone();
        assert_eq!(first.len(), 1);
        assert!(first[0] == a_id || first[0] == b_id);

        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);
        let second = a.editor.selection.clone();
        assert_ne!(first, second, "Next should move to the other match");

        a.mods.shift = true;
        a.on_key_compat(&Key::Named(NamedKey::Enter), ElementState::Pressed);
        assert_eq!(a.editor.selection, first, "Shift-Enter should step backward again");
    }

    #[test]
    fn clicking_replace_rewrites_the_current_match() {
        let mut a = app();
        let id = a
            .editor
            .create_shape(dc_model::ShapeType::Rectangle, dc_model::BoundingBox::new(0.0, 0.0, 100.0, 40.0));
        a.editor.package.document.pages[0].shape_mut(&id).unwrap().text =
            Some(dc_model::Text::new("Approval pending"));
        a.view.find_open = true;
        a.view.find_query = "approval".into();
        a.view.find_replacement = "Sign-off".into();

        let replace_rect =
            a.chrome().regions.iter().find(|r| r.action == UiAction::ReplaceCurrentMatch).unwrap().rect;
        a.pointer = replace_rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.page().shape(&id).unwrap().text_str(), "Sign-off pending");
    }

    #[test]
    fn clicking_elsewhere_unfocuses_the_find_query() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.view.find_open = true;
        a.view.find_query_focused = true;
        a.view.find_query = "approval".into();

        a.pointer = a.chrome().layout.canvas.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.find_query_focused);
        assert_eq!(a.view.find_query, "approval", "the query itself is not cleared by blurring");
    }

    // ------------------------------------------------ editing through keys

    /// An app with one labelled shape, selected and open for editing.
    fn app_editing() -> App {
        let mut a = app();
        a.sync_canvas_viewport();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(40.0, 40.0, 200.0, 80.0),
        );
        a.editor.selection = vec![id];
        let mut cx = App::text_ctx(&mut a.document_renderer);
        a.editor.begin_text_edit_on_selection(&mut cx);
        a
    }

    #[test]
    fn typing_reaches_the_open_label() {
        let mut a = app_editing();
        for ch in ["H", "i"] {
            a.on_key(&Key::Character(ch.into()), ElementState::Pressed, Some(ch));
        }
        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "Hi");
    }

    #[test]
    fn space_types_a_space_instead_of_panning() {
        let mut a = app_editing();
        a.on_key(&Key::Character("a".into()), ElementState::Pressed, Some("a"));
        a.on_key(&Key::Named(NamedKey::Space), ElementState::Pressed, Some(" "));
        a.on_key(&Key::Character("b".into()), ElementState::Pressed, Some("b"));

        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "a b");
        assert!(!a.space_held, "space must not arm panning while typing");
    }

    #[test]
    fn enter_commits_and_shift_enter_adds_a_line() {
        let mut a = app_editing();
        a.on_key(&Key::Character("a".into()), ElementState::Pressed, Some("a"));

        a.mods.shift = true;
        a.on_key(&Key::Named(NamedKey::Enter), ElementState::Pressed, None);
        assert!(a.editor.is_editing(), "shift-enter stays in the label");
        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "a\n");

        a.mods.shift = false;
        a.on_key(&Key::Named(NamedKey::Enter), ElementState::Pressed, None);
        assert!(!a.editor.is_editing(), "enter finishes the label");
    }

    #[test]
    fn escape_abandons_the_edit() {
        let mut a = app_editing();
        a.on_key(&Key::Character("x".into()), ElementState::Pressed, Some("x"));
        a.on_key(&Key::Named(NamedKey::Escape), ElementState::Pressed, None);

        assert!(!a.editor.is_editing());
        assert_eq!(a.editor.page().shapes()[0].text_str(), "", "nothing was written");
    }

    #[test]
    fn shortcuts_do_not_reach_the_document_while_typing() {
        let mut a = app_editing();
        let shapes_before = a.editor.page().shapes().len();

        // "r" would normally pick the rectangle tool, and delete would remove
        // the shape; both must be swallowed by the open label.
        a.on_key(&Key::Character("r".into()), ElementState::Pressed, Some("r"));
        a.on_key(&Key::Named(NamedKey::Delete), ElementState::Pressed, None);

        assert_eq!(a.editor.tool, Tool::Select, "the tool must not change");
        assert_eq!(a.editor.page().shapes().len(), shapes_before);
        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "r");
    }

    #[test]
    fn backspace_edits_text_rather_than_deleting_the_shape() {
        let mut a = app_editing();
        a.on_key(&Key::Character("a".into()), ElementState::Pressed, Some("a"));
        a.on_key(&Key::Character("b".into()), ElementState::Pressed, Some("b"));
        a.on_key(&Key::Named(NamedKey::Backspace), ElementState::Pressed, None);

        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "a");
        assert_eq!(a.editor.page().shapes().len(), 1);
    }

    #[test]
    fn arrows_move_the_caret_rather_than_nudging() {
        let mut a = app_editing();
        for ch in ["a", "c"] {
            a.on_key(&Key::Character(ch.into()), ElementState::Pressed, Some(ch));
        }
        let before = a.editor.page().shapes()[0].bounding_box;

        a.on_key(&Key::Named(NamedKey::ArrowLeft), ElementState::Pressed, None);
        a.on_key(&Key::Character("b".into()), ElementState::Pressed, Some("b"));

        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "abc");
        let after = a.editor.page().shapes()[0].bounding_box;
        assert_eq!((after.x, after.y), (before.x, before.y), "the shape must not move");
    }

    #[test]
    fn enter_opens_the_label_when_not_already_editing() {
        let mut a = app();
        a.sync_canvas_viewport();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 120.0, 60.0),
        );
        a.editor.selection = vec![id];

        a.on_key(&Key::Named(NamedKey::Enter), ElementState::Pressed, None);
        assert!(a.editor.is_editing());
    }

    #[test]
    fn control_characters_are_not_inserted() {
        let mut a = app_editing();
        // A platform may report a control character for some keys.
        a.on_key(&Key::Character("\u{1}".into()), ElementState::Pressed, Some("\u{1}"));
        assert_eq!(a.editor.text_edit.as_ref().unwrap().text(), "");
    }

    #[test]
    fn bold_shortcut_applies_to_the_selection() {
        let mut a = app();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 120.0, 60.0),
        );
        if let Some(shape) = a.editor.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("label"));
        }
        a.editor.selection = vec![id.clone()];

        press_shortcut(&mut a, "⌘B");

        let text = a.editor.page().shape(&id).unwrap().text_str().to_string();
        assert!(
            dc_model::markup::all_runs(&text, &dc_model::TextStyle::default(), |s| s.bold),
            "got {text:?}"
        );
    }

    #[test]
    fn page_controls_add_duplicate_and_delete() {
        let mut a = app();
        a.perform(UiAction::AddPage);
        assert_eq!(a.editor.document().pages.len(), 2);

        a.perform(UiAction::DuplicatePage(0));
        assert_eq!(a.editor.document().pages.len(), 3);

        a.perform(UiAction::DeletePage(0));
        assert_eq!(a.editor.document().pages.len(), 2);
    }

    // ------------------------------------------------------- the menu bar

    /// An app whose menu bar is drawn inside the window.
    ///
    /// That is the layout everywhere except macOS, where the system owns the
    /// bar; these tests force it so the in-window path stays covered on every
    /// machine the suite runs on.
    fn app_with_window_menus() -> App {
        let mut a = app();
        a.metrics.menubar_height = Metrics::default().menubar_height;
        a.sync_canvas_viewport();
        a
    }

    /// Presses a shortcut the way this platform delivers it.
    ///
    /// Where the window draws its own menus the key reaches the key handler;
    /// where the system owns the bar the key never arrives, and the matching
    /// menu item fires instead. A test says what the user pressed and this
    /// finds the path, so the same assertion holds either way.
    fn press_shortcut(a: &mut App, shortcut: &str) {
        let wanted = crate::menu::Shortcut::parse(shortcut)
            .unwrap_or_else(|| panic!("{shortcut:?} is not a shortcut"));
        assert!(wanted.is_claimable(), "{shortcut:?} is not a command shortcut");

        if MENUBAR_IN_WINDOW {
            let key = match wanted.key {
                crate::menu::ShortcutKey::Char(c) => {
                    Key::Character(c.to_lowercase().to_string().into())
                }
                other => panic!("{other:?} is not typed as a character"),
            };
            a.mods.command = true;
            a.mods.shift = wanted.shift;
            a.on_key_compat(&key, ElementState::Pressed);
            a.mods = Mods::default();
            return;
        }

        // The system bar dispatches by matching the accelerator, so the test
        // does the same and then runs what it found.
        let menus = crate::menu::menus(&a.editor, &a.view);
        let found = menus
            .iter()
            .flat_map(|m| m.items.iter())
            .find(|i| crate::menu::Shortcut::parse(&i.shortcut) == Some(wanted))
            .unwrap_or_else(|| panic!("no menu item carries {shortcut:?}"));
        assert!(found.enabled, "{shortcut:?} is on a dimmed command");
        let action = found.action.clone();
        a.perform(action);
    }

    /// Puts the pointer on a menu title and presses.
    fn click_menu_title(a: &mut App, index: usize) {
        let chrome = a.chrome();
        let region = chrome
            .regions
            .iter()
            .find(|r| r.action == UiAction::OpenMenu(index))
            .expect("a menu title");
        a.pointer = region.rect.center();
        a.on_pointer_down(MouseButton::Left);
    }

    /// Clicks a named entry in the open menu.
    fn click_menu_entry(a: &mut App, label: &str) {
        let chrome = a.chrome();
        let region = chrome
            .regions
            .iter()
            .find(|r| {
                r.label == label && matches!(r.kind, crate::ui::RegionKind::MenuEntry { .. })
            })
            .unwrap_or_else(|| panic!("no {label:?} in the open menu"));
        a.pointer = region.rect.center();
        a.on_pointer_down(MouseButton::Left);
    }

    #[test]
    fn clicking_a_menu_title_opens_and_closes_it() {
        let mut a = app_with_window_menus();
        assert_eq!(a.open_menu, None);

        click_menu_title(&mut a, 0);
        assert_eq!(a.open_menu, Some(0));

        click_menu_title(&mut a, 0);
        assert_eq!(a.open_menu, None, "clicking the title again closes it");
    }

    #[test]
    fn clicking_another_title_switches_menus() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 0);
        click_menu_title(&mut a, 2);
        assert_eq!(a.open_menu, Some(2));
    }

    #[test]
    fn sliding_across_the_bar_with_a_menu_open_switches_to_it() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 0);

        let chrome = a.chrome();
        let view = chrome
            .regions
            .iter()
            .find(|r| r.action == UiAction::OpenMenu(6))
            .unwrap();
        a.pointer = view.rect.center();
        a.on_pointer_move();
        assert_eq!(a.open_menu, Some(6));
    }

    #[test]
    fn choosing_a_command_runs_it_and_closes_the_menu() {
        let mut a = app_with_window_menus();
        assert_eq!(a.editor.document().pages.len(), 1);

        click_menu_title(&mut a, 3); // Insert
        click_menu_entry(&mut a, "Page");

        assert_eq!(a.editor.document().pages.len(), 2);
        assert_eq!(a.open_menu, None);
    }

    #[test]
    fn a_dimmed_command_does_nothing() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 1); // Edit

        // Undo is unavailable in a fresh document; pressing it must not close
        // the menu or change anything.
        let chrome = a.chrome();
        let undo = chrome.regions.iter().find(|r| r.label == "Undo").unwrap();
        a.pointer = undo.rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.open_menu, Some(1), "a dimmed command should not dismiss");
    }

    #[test]
    fn clicking_the_canvas_dismisses_an_open_menu_without_drawing() {
        let mut a = app_with_window_menus();
        a.editor.tool = Tool::Shape(dc_model::ShapeType::Rectangle);
        click_menu_title(&mut a, 0);

        a.pointer = a.chrome().layout.canvas.center();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.open_menu, None);
        assert!(!a.dragging_canvas, "the dismissing click must not start a shape");
        assert_eq!(a.editor.page().shapes().len(), 0);
    }

    #[test]
    fn escape_closes_an_open_menu_before_touching_the_selection() {
        let mut a = app_with_window_menus();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        click_menu_title(&mut a, 0);

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert_eq!(a.open_menu, None);
        assert!(!a.editor.selection.is_empty(), "the selection should survive");

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert!(a.editor.selection.is_empty());
    }

    #[test]
    fn a_point_inside_an_open_menu_is_not_canvas() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 3);

        let chrome = a.chrome();
        let (_, panel) = chrome.open_menu.unwrap();
        assert!(!chrome.is_over_canvas(panel.center()));
    }

    #[test]
    fn view_commands_reach_the_renderer_through_the_menu() {
        let mut a = app_with_window_menus();
        let before = a.document_renderer.options.show_grid;

        click_menu_title(&mut a, 6); // View
        click_menu_entry(&mut a, "Show Grid");

        assert_eq!(a.document_renderer.options.show_grid, !before);
        assert_eq!(a.view.settings.show_grid, !before, "the menu check follows the renderer");
    }

    #[test]
    fn the_theme_command_swaps_the_palette_and_the_check() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 6);
        click_menu_entry(&mut a, "Dark Theme");

        assert!(a.view.settings.dark_theme);
        assert!(a.document_renderer.theme.is_dark());
    }

    #[test]
    fn choosing_a_library_changes_the_palette() {
        let mut a = app();
        a.sync_canvas_viewport();
        assert_eq!(a.view.library, dc_model::ShapeLibrary::Standard);

        a.perform(UiAction::SelectLibrary(dc_model::ShapeLibrary::Bpmn));
        assert_eq!(a.view.library, dc_model::ShapeLibrary::Bpmn);

        let offered: Vec<String> = a
            .chrome()
            .regions
            .iter()
            .filter_map(|r| match &r.kind {
                crate::ui::RegionKind::ShapeTile(t) => Some(t.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert!(offered.contains(&"bpmnGateway".to_string()), "got {offered:?}");
    }

    #[test]
    fn arrange_commands_reach_the_editor() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 40.0, 20.0),
        );
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(100.0, 60.0, 40.0, 20.0),
        );
        a.editor.select_all();

        a.perform(UiAction::Align(crate::editor::Align::Left));
        let xs: Vec<f64> = a
            .editor
            .page()
            .shapes()
            .iter()
            .map(|s| s.bounding_box.x)
            .collect();
        assert_eq!(xs, vec![0.0, 0.0]);

        a.perform(UiAction::Group);
        assert!(a.editor.selection_is_grouped());
        a.perform(UiAction::Ungroup);
        assert!(!a.editor.selection_is_grouped());
    }

    // -------------------------------------------- where the menus live

    #[test]
    fn the_window_gives_up_its_menu_strip_to_a_system_menu_bar() {
        let a = app();
        let menubar = a.chrome().layout.menubar;
        if MENUBAR_IN_WINDOW {
            assert!(menubar.height() > 10.0, "the window should draw its menus");
        } else {
            assert_eq!(menubar.height(), 0.0, "the system draws them instead");
        }
    }

    #[test]
    fn a_window_without_a_menu_strip_draws_no_titles() {
        let a = app();
        let titles = a
            .chrome()
            .regions
            .iter()
            .filter(|r| matches!(r.kind, crate::ui::RegionKind::MenuTitle { .. }))
            .count();
        if MENUBAR_IN_WINDOW {
            assert_eq!(titles, 8);
        } else {
            assert_eq!(titles, 0, "a system menu bar leaves none in the window");
        }
    }

    #[test]
    fn the_canvas_reclaims_the_space_the_menu_strip_would_have_taken() {
        let mut native = app();
        native.metrics.menubar_height = 0.0;
        let mut in_window = app();
        in_window.metrics.menubar_height = Metrics::default().menubar_height;

        assert!(
            native.chrome().layout.canvas.height()
                > in_window.chrome().layout.canvas.height(),
            "dropping the strip should give the drawing area the room"
        );
    }

    #[test]
    fn command_keys_are_handled_exactly_once() {
        // Where the system owns the menu bar it matches the accelerator
        // before the window sees the key, so the key handler must stand
        // aside. Were it to act too, every shortcut would fire twice.
        let mut a = app();
        a.sync_canvas_viewport();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        assert_eq!(a.editor.page().shapes().len(), 1);

        a.mods.command = true;
        a.on_key_compat(&Key::Character("z".into()), ElementState::Pressed);
        a.mods = Mods::default();

        let shapes = a.editor.page().shapes().len();
        if MENUBAR_IN_WINDOW {
            assert_eq!(shapes, 0, "the key handler owns the shortcut here");
        } else {
            assert_eq!(shapes, 1, "the menu bar owns it; the key must do nothing");
        }
    }

    #[test]
    fn a_command_key_never_falls_through_to_a_tool_binding() {
        // Command-V must not pick the select tool just because `v` does.
        for (ch, tool) in [
            ("v", Tool::Select),
            ("c", Tool::Connector),
            ("r", Tool::Shape(dc_model::ShapeType::Rectangle)),
            ("p", Tool::Shape(dc_model::ShapeType::Process)),
        ] {
            let mut a = app();
            a.editor.tool = Tool::Pan;
            a.mods.command = true;
            a.on_key_compat(&Key::Character(ch.into()), ElementState::Pressed);
            assert_eq!(
                a.editor.tool,
                Tool::Pan,
                "command-{ch} should not select {tool:?}"
            );
        }
    }

    #[test]
    fn copying_inside_a_label_takes_the_text_not_the_shape() {
        // A menu command has to mean what the same shortcut means while a
        // label is open, because on macOS the menu is the only path to it.
        let mut a = app_editing();
        for ch in ["h", "i"] {
            a.on_key(&Key::Character(ch.into()), ElementState::Pressed, Some(ch));
        }
        let mut cx = App::text_ctx(&mut a.document_renderer);
        a.editor.text_command(TextCommand::SelectAll, &mut cx);
        drop(cx);

        a.perform(UiAction::Copy);
        assert!(a.editor.is_editing(), "copying must not close the label");
        assert_eq!(a.editor.page().shapes().len(), 1, "and must not touch shapes");

        a.perform(UiAction::Cut);
        assert_eq!(
            a.editor.text_edit.as_ref().unwrap().text(),
            "",
            "cutting should remove the selected text"
        );
        assert_eq!(a.editor.page().shapes().len(), 1, "not the shape");
    }

    /// Opens a label, types `text`, and selects `word_len` characters back
    /// from the end.
    fn app_with_selected_word(text: &str, word_len: usize) -> App {
        let mut a = app_editing();
        for ch in text.chars() {
            let s = ch.to_string();
            a.on_key(&Key::Character(s.clone().into()), ElementState::Pressed, Some(&s));
        }
        let mut cx = App::text_ctx(&mut a.document_renderer);
        a.editor.text_command(TextCommand::TextEnd { select: false }, &mut cx);
        for _ in 0..word_len {
            a.editor
                .text_command(TextCommand::Left { select: true, word: false }, &mut cx);
        }
        drop(cx);
        a
    }

    /// The runs of the shape's stored label, as text and whether each is bold.
    fn label_runs(a: &App) -> Vec<(String, bool)> {
        let markup = a.editor.page().shapes()[0].text_str().to_string();
        dc_model::markup::parse(&markup, Default::default())
            .runs
            .into_iter()
            .filter(|r| !r.text.is_empty())
            .map(|r| (r.text, r.style.bold))
            .collect()
    }

    #[test]
    fn the_bold_button_styles_the_selection_not_the_whole_label() {
        // The bug this guards: every chrome click used to commit the open
        // label first, so by the time the button's action ran there was no
        // selection left and the whole label was styled.
        let mut a = app_with_selected_word("hello world", 5);
        assert_eq!(
            a.editor.text_edit.as_ref().unwrap().selected_text().as_deref(),
            Some("world")
        );

        let chrome = a.chrome();
        let bold = chrome
            .regions
            .iter()
            .find(|r| r.action == UiAction::ToggleText(crate::editor::TextToggle::Bold))
            .expect("a bold button");
        a.pointer = bold.rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(a.editor.is_editing(), "the label must stay open");
        a.editor.commit_text_edit();
        assert_eq!(
            label_runs(&a),
            vec![("hello ".to_string(), false), ("world".to_string(), true)]
        );
    }

    #[test]
    fn the_bold_command_styles_the_selection_not_the_whole_label() {
        // The same through the menu, which is the only path on macOS.
        let mut a = app_with_selected_word("hello world", 5);
        a.perform(UiAction::ToggleText(crate::editor::TextToggle::Bold));

        assert!(a.editor.is_editing());
        a.editor.commit_text_edit();
        assert_eq!(
            label_runs(&a),
            vec![("hello ".to_string(), false), ("world".to_string(), true)]
        );
    }

    #[test]
    fn the_text_color_swatch_colors_the_selection_only() {
        let mut a = app_with_selected_word("red blue", 4);
        let red = dc_model::Color::rgb(0xdc, 0x26, 0x26);

        let chrome = a.chrome();
        let swatch = chrome
            .regions
            .iter()
            .find(|r| r.action == UiAction::SetTextColor(red))
            .expect("a text colour swatch");
        a.pointer = swatch.rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(a.editor.is_editing(), "the label must stay open");
        a.editor.commit_text_edit();

        let markup = a.editor.page().shapes()[0].text_str().to_string();
        let parsed = dc_model::markup::parse(&markup, Default::default());
        let colored: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.color == red)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(colored, "blue", "got {markup:?}");
    }

    #[test]
    fn the_selection_survives_so_two_styles_can_be_applied_in_a_row() {
        let mut a = app_with_selected_word("hello world", 5);
        let press = |a: &mut App, toggle: crate::editor::TextToggle| {
            let chrome = a.chrome();
            let button = chrome
                .regions
                .iter()
                .find(|r| r.action == UiAction::ToggleText(toggle))
                .expect("a style button");
            a.pointer = button.rect.center();
            a.on_pointer_down(MouseButton::Left);
        };

        press(&mut a, crate::editor::TextToggle::Bold);
        press(&mut a, crate::editor::TextToggle::Italic);
        assert_eq!(
            a.editor.text_edit.as_ref().unwrap().selected_text().as_deref(),
            Some("world"),
            "the selection should still be there for the second press"
        );

        a.editor.commit_text_edit();
        let markup = a.editor.page().shapes()[0].text_str().to_string();
        let parsed = dc_model::markup::parse(&markup, Default::default());
        let styled: String = parsed
            .runs
            .iter()
            .filter(|r| r.style.bold && r.style.italic)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(styled, "world", "got {markup:?}");
    }

    #[test]
    fn a_control_that_is_not_about_text_still_finishes_the_label() {
        // Clicking away from the text — onto a shape tile, say — should
        // commit, because the user has moved on.
        let mut a = app_with_selected_word("hi", 2);
        let chrome = a.chrome();
        let tile = chrome
            .regions
            .iter()
            .find(|r| matches!(r.kind, crate::ui::RegionKind::ShapeTile(_)))
            .expect("a shape tile");
        a.pointer = tile.rect.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.editor.is_editing(), "the label should have been committed");
        assert_eq!(a.editor.page().shapes()[0].text_str(), "hi");
    }

    #[test]
    fn formatting_with_no_label_open_still_styles_the_whole_thing() {
        // The fallback has to keep working: with a shape selected and no
        // label open, there is no range, so the command takes the label.
        let mut a = app();
        let id = a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 120.0, 60.0),
        );
        if let Some(shape) = a.editor.package.document.pages[0].shape_mut(&id) {
            shape.text = Some(dc_model::Text::new("hello world"));
        }
        a.editor.selection = vec![id];

        a.perform(UiAction::ToggleText(crate::editor::TextToggle::Bold));
        assert_eq!(label_runs(&a), vec![("hello world".to_string(), true)]);
    }

    #[test]
    fn select_all_inside_a_label_selects_the_text() {
        let mut a = app_editing();
        for ch in ["a", "b"] {
            a.on_key(&Key::Character(ch.into()), ElementState::Pressed, Some(ch));
        }
        a.perform(UiAction::SelectAll);
        assert_eq!(
            a.editor.text_edit.as_ref().unwrap().selected_text().as_deref(),
            Some("ab")
        );
    }

    #[test]
    fn undoing_mid_edit_abandons_the_label_first() {
        let mut a = app_editing();
        a.on_key(&Key::Character("x".into()), ElementState::Pressed, Some("x"));
        a.perform(UiAction::Undo);

        assert!(!a.editor.is_editing(), "the caret must not outlive the label");
        assert_eq!(a.editor.page().shapes().len(), 0, "and the undo should land");
    }

    #[test]
    fn a_command_that_leaves_the_document_commits_the_label_first() {
        // Tested one level down from `perform`, because the commands that do
        // this all go on to open a file dialog.
        for leaving in [
            UiAction::Save,
            UiAction::SaveAs,
            UiAction::ExportPng,
            UiAction::Open,
            UiAction::ImportVsdx,
            UiAction::NewDocument,
        ] {
            let mut a = app_editing();
            for ch in ["o", "k"] {
                a.on_key(&Key::Character(ch.into()), ElementState::Pressed, Some(ch));
            }

            let handled = a.perform_while_editing(&leaving);
            assert!(!handled, "{leaving:?} should still run after committing");
            assert!(!a.editor.is_editing(), "{leaving:?} left the label open");
            assert_eq!(
                a.editor.page().shapes()[0].text_str(),
                "ok",
                "{leaving:?} lost what was typed"
            );
        }
    }

    // -------------------------------------------------------- the settings

    #[test]
    fn the_settings_screen_opens_and_closes() {
        let mut a = app();
        a.sync_canvas_viewport();
        assert!(!a.view.settings_open);

        a.perform(UiAction::OpenSettings);
        assert!(a.view.settings_open);
        assert!(a.chrome().settings_sheet.is_some());

        a.perform(UiAction::CloseSettings);
        assert!(!a.view.settings_open);
    }

    #[test]
    fn the_settings_shortcut_opens_it_on_this_platform() {
        let mut a = app_with_window_menus();
        press_shortcut(&mut a, "⌘,");
        assert!(a.view.settings_open);
    }

    #[test]
    fn escape_closes_the_settings_before_anything_else() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 10.0, 10.0),
        );
        a.perform(UiAction::OpenSettings);

        a.on_key_compat(&Key::Named(NamedKey::Escape), ElementState::Pressed);
        assert!(!a.view.settings_open);
        assert!(!a.editor.selection.is_empty(), "the selection should survive");
    }

    #[test]
    fn opening_the_settings_finishes_an_open_label() {
        let mut a = app_editing();
        a.on_key(&Key::Character("x".into()), ElementState::Pressed, Some("x"));
        a.perform(UiAction::OpenSettings);

        assert!(!a.editor.is_editing());
        assert_eq!(a.editor.page().shapes()[0].text_str(), "x");
    }

    #[test]
    fn clicking_away_from_the_sheet_closes_it() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.perform(UiAction::OpenSettings);

        // A press on the canvas lands on the scrim, which closes the screen
        // rather than drawing anything.
        a.editor.tool = Tool::Shape(dc_model::ShapeType::Rectangle);
        a.pointer = kurbo::Point::new(20.0, a.chrome().layout.canvas.y0 + 20.0);
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.settings_open);
        assert_eq!(a.editor.page().shapes().len(), 0, "nothing should be drawn");
        assert!(!a.dragging_canvas);
    }

    #[test]
    fn a_setting_reaches_the_renderer_and_the_editor() {
        let mut a = app();

        a.change_setting(SettingChange::GridSpacing(18.0));
        assert_eq!(a.document_renderer.options.grid_spacing, 18.0);
        assert_eq!(a.editor.grid_spacing, 18.0, "snapping uses the same pitch");

        a.change_setting(SettingChange::GridStyle(dc_render::GridStyle::Dots));
        assert_eq!(a.document_renderer.options.grid_style, dc_render::GridStyle::Dots);

        a.change_setting(SettingChange::SnapToObjects(false));
        assert!(!a.editor.snap_enabled);

        a.change_setting(SettingChange::SnapToGrid(false));
        assert!(!a.editor.snap_to_grid);

        a.change_setting(SettingChange::SnapDistance(16.0));
        assert_eq!(a.editor.snap_threshold_px, 16.0);

        a.change_setting(SettingChange::DarkTheme(true));
        assert!(a.document_renderer.theme.is_dark());

        a.change_setting(SettingChange::RouteAroundShapes(false));
        assert!(!a.document_renderer.options.route_around_shapes);

        a.change_setting(SettingChange::ShowPageBounds(false));
        assert!(!a.document_renderer.options.show_page_bounds);
    }

    #[test]
    fn the_zoom_step_setting_changes_how_far_a_zoom_goes() {
        let mut a = app();
        a.sync_canvas_viewport();

        a.change_setting(SettingChange::ZoomStep(1.1));
        let start = a.editor.camera.zoom;
        a.perform(UiAction::ZoomIn);
        let gentle = a.editor.camera.zoom / start;

        a.perform(UiAction::ActualSize);
        a.change_setting(SettingChange::ZoomStep(1.5));
        let start = a.editor.camera.zoom;
        a.perform(UiAction::ZoomIn);
        let steep = a.editor.camera.zoom / start;

        assert!(steep > gentle, "{steep} should be more than {gentle}");
    }

    #[test]
    fn new_documents_take_the_defaults_from_the_settings() {
        let mut a = app();
        a.change_setting(SettingChange::DefaultUnits(dc_model::Units::Cm));
        a.change_setting(SettingChange::DefaultPageSize("a4".into()));

        a.perform(UiAction::NewDocument);
        assert_eq!(a.editor.document().units(), dc_model::Units::Cm);
        assert_eq!(a.editor.page_size_name(), "a4");
    }

    // ------------------------------------------------------ template gallery

    #[test]
    fn opening_the_gallery_from_the_menu() {
        let mut a = app_with_window_menus();
        click_menu_title(&mut a, 0); // File
        click_menu_entry(&mut a, "New from Template…");
        assert!(a.view.template_gallery_open);
    }

    #[test]
    fn clicking_a_template_loads_it_and_closes_the_gallery() {
        let mut a = app();
        a.view.template_gallery_open = true;

        let row = a
            .chrome()
            .regions
            .iter()
            .find(|r| r.action == UiAction::NewFromTemplate(0))
            .unwrap()
            .rect;
        a.pointer = row.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.template_gallery_open);
        assert!(!a.editor.page().shapes().is_empty(), "the template's shapes should be loaded");
    }

    #[test]
    fn clicking_cancel_closes_the_gallery_without_changing_the_document() {
        let mut a = app();
        let before = a.editor.page().shapes().len();
        a.view.template_gallery_open = true;

        let cancel = a.chrome().regions.iter().find(|r| r.label == "Cancel").unwrap().rect;
        a.pointer = cancel.center();
        a.on_pointer_down(MouseButton::Left);

        assert!(!a.view.template_gallery_open);
        assert_eq!(a.editor.page().shapes().len(), before);
    }

    #[test]
    fn clicking_the_scrim_closes_the_gallery() {
        let mut a = app();
        a.view.template_gallery_open = true;
        a.pointer = kurbo::Point::new(2.0, 2.0);
        a.on_pointer_down(MouseButton::Left);
        assert!(!a.view.template_gallery_open);
    }

    #[test]
    fn new_connectors_take_the_default_routing() {
        let mut a = app();
        a.sync_canvas_viewport();
        a.change_setting(SettingChange::DefaultLineType(dc_model::LineType::Curved));

        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(0.0, 0.0, 40.0, 20.0),
        );
        a.editor.create_shape(
            dc_model::ShapeType::Rectangle,
            dc_model::BoundingBox::new(200.0, 0.0, 40.0, 20.0),
        );
        a.editor.tool = Tool::Connector;
        {
            let mut cx = App::text_ctx(&mut a.document_renderer);
            a.editor.pointer_down(Point::new(20.0, 10.0), Mods::default(), false, 1, &mut cx);
            a.editor.pointer_move(Point::new(220.0, 10.0), Mods::default(), &mut cx);
            a.editor.pointer_up(Point::new(220.0, 10.0), Mods::default(), &mut cx);
        }

        assert_eq!(a.editor.page().lines()[0].line_type, dc_model::LineType::Curved);
    }

    #[test]
    fn the_view_menu_and_the_settings_screen_agree() {
        // Both change the same preference, so toggling from one must show up
        // in the other.
        let mut a = app();
        assert!(a.view.settings.show_grid);

        a.perform(UiAction::ToggleGrid);
        assert!(!a.view.settings.show_grid);
        assert!(!a.document_renderer.options.show_grid);

        a.perform(UiAction::ChangeSetting(SettingChange::ShowGrid(true)));
        assert!(a.view.settings.show_grid);

        let menus = crate::menu::menus(&a.editor, &a.view);
        let grid = menus
            .iter()
            .find(|m| m.title == "View")
            .unwrap()
            .items
            .iter()
            .find(|i| i.label == "Show Grid")
            .unwrap();
        assert_eq!(grid.kind, crate::menu::ItemKind::Check(true));
    }

    #[test]
    fn restoring_the_defaults_puts_everything_back() {
        let mut a = app();
        a.change_setting(SettingChange::DarkTheme(true));
        a.change_setting(SettingChange::GridSpacing(36.0));
        a.change_setting(SettingChange::SnapToObjects(false));

        a.change_setting(SettingChange::RestoreDefaults);

        assert_eq!(a.view.settings, crate::settings::Settings::default());
        assert!(!a.document_renderer.theme.is_dark(), "and it is applied, not just stored");
        assert_eq!(a.editor.grid_spacing, 9.0);
        assert!(a.editor.snap_enabled);
    }

    #[test]
    fn a_test_application_never_writes_the_users_settings_file() {
        // Running the suite must not reach outside the build directory.
        let a = app();
        assert!(
            !a.persist_settings,
            "an application built for a test must keep its preferences to itself"
        );
    }

    #[test]
    fn an_application_starts_from_the_settings_it_is_given() {
        let settings = crate::settings::Settings {
            dark_theme: true,
            grid_spacing: 18.0,
            snap_to_objects: false,
            default_line_type: dc_model::LineType::Straight,
            ..Default::default()
        };
        let a = App::with_settings(settings);

        assert!(a.document_renderer.theme.is_dark(), "settings apply at startup");
        assert_eq!(a.document_renderer.options.grid_spacing, 18.0);
        assert!(!a.editor.snap_enabled);
        assert_eq!(a.editor.default_line_type, dc_model::LineType::Straight);
    }

    // ---------------------------------------------------- page tab dragging

    /// An app with three pages, sized so the tab bar is laid out.
    fn app_with_pages() -> App {
        let mut a = app();
        a.sync_canvas_viewport();
        a.editor.add_page();
        a.editor.add_page();
        a.editor.page_index = 0;
        a
    }

    fn page_titles(a: &App) -> Vec<String> {
        a.editor
            .document()
            .pages
            .iter()
            .map(|p| p.title.clone())
            .collect()
    }

    #[test]
    fn pressing_a_tab_without_moving_selects_its_page() {
        let mut a = app_with_pages();
        let chrome = a.chrome();
        a.pointer = chrome.tab_rects[2].center();

        a.on_pointer_down(MouseButton::Left);
        assert_eq!(a.tab_press, Some((2, a.pointer.x)));
        assert_eq!(a.editor.page_index, 0, "selection waits for the release");

        a.on_pointer_up();
        assert_eq!(a.editor.page_index, 2);
        assert!(a.tab_press.is_none());
    }

    #[test]
    fn dragging_a_tab_sideways_reorders_the_pages() {
        let mut a = app_with_pages();
        let before = page_titles(&a);
        let chrome = a.chrome();

        // Press the first tab and drag it past the last.
        a.pointer = chrome.tab_rects[0].center();
        a.on_pointer_down(MouseButton::Left);
        a.pointer = kurbo::Point::new(chrome.tab_rects[2].center().x, a.pointer.y);
        a.on_pointer_move();
        assert!(a.tab_drag.is_some(), "the press should have become a drag");

        a.on_pointer_up();
        let after = page_titles(&a);
        assert_eq!(after, vec![before[1].clone(), before[2].clone(), before[0].clone()]);
        assert!(a.tab_drag.is_none());
    }

    #[test]
    fn the_dragged_page_stays_the_one_showing() {
        let mut a = app_with_pages();
        a.editor.page_index = 0;
        let moved = a.editor.page().title.clone();

        let chrome = a.chrome();
        a.pointer = chrome.tab_rects[0].center();
        a.on_pointer_down(MouseButton::Left);
        a.pointer = kurbo::Point::new(chrome.tab_rects[2].center().x, a.pointer.y);
        a.on_pointer_move();
        a.on_pointer_up();

        assert_eq!(a.editor.page().title, moved);
        assert_eq!(a.editor.page_index, 2);
    }

    #[test]
    fn a_tiny_wobble_on_a_tab_is_not_a_drag() {
        let mut a = app_with_pages();
        let before = page_titles(&a);
        let chrome = a.chrome();

        a.pointer = chrome.tab_rects[0].center();
        a.on_pointer_down(MouseButton::Left);
        a.pointer.x += 2.0;
        a.on_pointer_move();
        assert!(a.tab_drag.is_none(), "two pixels is a click, not a drag");

        a.on_pointer_up();
        assert_eq!(page_titles(&a), before);
        assert_eq!(a.editor.page_index, 0);
    }

    #[test]
    fn dragging_a_tab_is_undoable() {
        let mut a = app_with_pages();
        let before = page_titles(&a);
        let chrome = a.chrome();

        a.pointer = chrome.tab_rects[0].center();
        a.on_pointer_down(MouseButton::Left);
        a.pointer = kurbo::Point::new(chrome.tab_rects[2].center().x, a.pointer.y);
        a.on_pointer_move();
        a.on_pointer_up();
        assert_ne!(page_titles(&a), before);

        a.editor.undo();
        assert_eq!(page_titles(&a), before);
    }

    #[test]
    fn double_clicking_a_tab_still_renames_it() {
        let mut a = app_with_pages();
        let chrome = a.chrome();
        a.pointer = chrome.tab_rects[1].center();

        a.on_pointer_down(MouseButton::Left);
        a.on_pointer_up();
        a.on_pointer_down(MouseButton::Left);

        assert_eq!(a.editor.editing_page_title(), Some(1));
        assert!(a.tab_drag.is_none(), "renaming is not a drag");
    }

    // ------------------------------------------------------ importing visio

    #[test]
    fn opening_a_visio_drawing_imports_it() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drawing.vsdx");
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            let parts = [
                (
                    "visio/pages/pages.xml",
                    r#"<Pages xmlns="x" xmlns:r="y"><Page ID="0" NameU="Imported">
                       <PageSheet><Cell N="PageHeight" V="11"/></PageSheet>
                       <Rel r:id="rId1"/></Page></Pages>"#,
                ),
                (
                    "visio/pages/_rels/pages.xml.rels",
                    r#"<Relationships xmlns="r"><Relationship Id="rId1" Target="page1.xml"/></Relationships>"#,
                ),
                (
                    "visio/pages/page1.xml",
                    r#"<PageContents xmlns="x"><Shapes>
                       <Shape ID="1" Type="Shape" NameU="Decision">
                       <Cell N="PinX" V="2"/><Cell N="PinY" V="9"/>
                       <Cell N="Width" V="2"/><Cell N="Height" V="1"/>
                       <Text>Imported?</Text></Shape></Shapes></PageContents>"#,
                ),
            ];
            for (name, content) in parts {
                zip.start_file(name, options).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        std::fs::write(&path, buf.into_inner()).unwrap();

        let mut a = app();
        a.sync_canvas_viewport();
        a.open_initial(&path);

        assert_eq!(a.editor.page().title, "Imported");
        assert_eq!(a.editor.page().shapes().len(), 1);
        assert_eq!(a.editor.page().shapes()[0].text_str(), "Imported?");
        assert!(
            a.editor.path.is_none(),
            "saving must not overwrite the Visio file"
        );
    }

    #[test]
    fn hovering_tracks_chrome_regions() {
        let mut a = app();
        a.sync_canvas_viewport();
        let chrome = a.chrome();
        a.pointer = chrome.regions[0].rect.center();
        a.on_pointer_move();
        assert!(a.hovered_region.is_some());

        a.pointer = chrome.layout.canvas.center();
        a.on_pointer_move();
        assert!(a.hovered_region.is_none());
    }
}
