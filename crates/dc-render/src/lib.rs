//! Scene construction for diagram documents.
//!
//! This crate turns a [`dc_model::Document`] into a [`vello::Scene`]. It never
//! touches the GPU, so the whole render path is testable on a machine with no
//! graphics device: the application owns the device, surface, and swapchain.
//!
//! Document content is encoded in document coordinates with the camera applied
//! as a single transform, which keeps stroke widths scaling with zoom the way a
//! vector drawing should. Interface overlays, where handles must stay the same
//! physical size at any zoom, are encoded in screen coordinates instead.

pub mod camera;
pub mod handles;
pub mod images;
pub mod text;
pub mod theme;

pub use camera::Camera;
// `GridStyle` is declared below, beside the options it belongs to.
pub use images::ImageCache;
pub use handles::{HandleKind, all_handles, handle_position};
pub use text::TextEngine;
pub use theme::{Metrics, Theme};

use dc_geom::{MarkerFill, Obstacles, RoutedLine, Router};
use dc_model::{BoundingBox, Document, Fill as PaintFill, LabelSide, Line, Page, Point, Shape, ShapeType, StrokeStyle};
use kurbo::{Affine, BezPath, Circle, Rect, Stroke as KStroke};
use peniko::{Color as PenikoColor, Fill as FillRule};
use vello::Scene;

/// How the grid is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GridStyle {
    /// Every line, with every tenth one emphasized: graph paper.
    #[default]
    Graph,
    /// Only the emphasized lines, for a quieter sheet.
    Lines,
    /// A dot at each intersection.
    Dots,
}

impl GridStyle {
    pub fn label(self) -> &'static str {
        match self {
            GridStyle::Graph => "Graph",
            GridStyle::Lines => "Lines",
            GridStyle::Dots => "Dots",
        }
    }
}

/// Above this many dots the grid stops being a texture and starts being a
/// wash of colour that costs more to encode than it conveys, so the dot grid
/// falls back to lines.
const MAX_GRID_DOTS: f64 = 12_000.0;

/// What to include when drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    pub show_grid: bool,
    /// Minor grid pitch in document units.
    pub grid_spacing: f64,
    pub grid_style: GridStyle,
    pub show_page_bounds: bool,
    pub show_selection: bool,
    /// Whether connectors steer around unrelated shapes.
    pub route_around_shapes: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            show_grid: true,
            // Nine points is an eighth of an inch, a conventional diagram pitch.
            grid_spacing: 9.0,
            grid_style: GridStyle::Graph,
            show_page_bounds: true,
            show_selection: true,
            route_around_shapes: true,
        }
    }
}

/// An alignment guide shown during a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapGuide {
    /// Document-space coordinate of the guide.
    pub at: f64,
    pub vertical: bool,
}

/// Transient interface state drawn on top of the document.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay<'a> {
    /// IDs of selected shapes and lines.
    pub selected: &'a [String],
    pub hovered: Option<&'a str>,
    /// A shape a connector being drawn would attach to.
    pub attach_target: Option<&'a str>,
    /// Rubber-band selection rectangle, in document units.
    pub marquee: Option<BoundingBox>,
    pub snap_guides: &'a [SnapGuide],
    /// A connector mid-draw, from its origin to the pointer.
    pub pending_connector: Option<(Point, Point)>,
    /// Connection points to offer on the shape being aimed at, in document
    /// units. Empty whenever no connector is being drawn or aimed.
    pub ports: &'a [Point],
    /// Quick-connect arrows around the hovered shape, in document units.
    /// Dragging one draws a connector and makes the shape it lands on.
    pub quick_connect: &'a [Point],
    /// The item whose label is being edited.
    ///
    /// Its static text is suppressed so the live editing view is the only copy
    /// on screen; drawing both would show a stale ghost behind the caret.
    pub editing: Option<&'a str>,
}

/// Builds scenes from documents.
pub struct Renderer {
    pub theme: Theme,
    pub metrics: Metrics,
    pub options: RenderOptions,
    /// Decoded assets for the open document.
    pub images: ImageCache,
    text: TextEngine,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            theme: Theme::light(),
            metrics: Metrics::default(),
            options: RenderOptions::default(),
            images: ImageCache::new(),
            text: TextEngine::new(),
        }
    }

    pub fn with_theme(theme: Theme) -> Self {
        Renderer { theme, ..Self::new() }
    }

    pub fn text_engine(&mut self) -> &mut TextEngine {
        &mut self.text
    }

    /// Encodes one page of a document into `scene`.
    ///
    /// The scene is reset first, so a caller can rebuild every frame.
    pub fn draw(
        &mut self,
        scene: &mut Scene,
        doc: &Document,
        page_index: usize,
        camera: &Camera,
        overlay: &Overlay<'_>,
    ) {
        scene.reset();
        let Some(page) = doc.pages.get(page_index) else {
            self.fill_viewport(scene, camera);
            return;
        };

        let units = doc.units();
        let transform = camera.transform();

        self.fill_viewport(scene, camera);
        self.draw_page_sheet(scene, page, units, camera, transform);
        if self.options.show_grid {
            self.draw_grid(scene, page, units, camera, transform);
        }

        // Routing once per frame keeps connectors consistent between the
        // stroke pass and the overlay pass.
        let mut router = Router::new(page);
        router.avoid_obstacles = self.options.route_around_shapes;
        let routes = router.route_all();

        self.draw_content(scene, page, &routes, transform, overlay);

        if self.options.show_selection {
            self.draw_overlay(scene, page, &routes, camera, overlay);
        }
    }

    /// The page's own background color.
    ///
    /// This is a property of the document, not of the interface theme: dark
    /// mode themes the chrome, it does not repaint someone's artwork. Anything
    /// drawn *onto* the sheet — the grid, label plates, hollow arrowheads —
    /// must contrast with this rather than with the panels.
    fn page_background(&self, page: &Page) -> dc_model::Color {
        page.resolved_settings().resolved_fill()
    }

    /// A tint of the page background, used for marks that sit on the sheet.
    fn on_page(&self, page: &Page, amount: f32) -> dc_model::Color {
        let background = self.page_background(page);
        background.lerp(background.contrasting_text(), amount)
    }

    fn fill_viewport(&self, scene: &mut Scene, camera: &Camera) {
        let r = Rect::new(0.0, 0.0, camera.viewport.x, camera.viewport.y);
        scene.fill(
            FillRule::NonZero,
            Affine::IDENTITY,
            px(self.theme.canvas),
            None,
            &r,
        );
    }

    fn draw_page_sheet(
        &self,
        scene: &mut Scene,
        page: &Page,
        units: dc_model::Units,
        camera: &Camera,
        transform: Affine,
    ) {
        let settings = page.resolved_settings();
        let Some((w, h)) = settings.bounds_points(units) else {
            // An infinite canvas has no sheet; the page color covers the view.
            let visible = camera.visible_rect();
            scene.fill(
                FillRule::NonZero,
                transform,
                px(settings.resolved_fill()),
                None,
                &visible,
            );
            return;
        };

        if !self.options.show_page_bounds {
            return;
        }

        let sheet = Rect::new(0.0, 0.0, w, h);
        // A soft offset rectangle reads as a shadow without needing a blur.
        let shadow = Rect::new(2.0, 3.0, w + 3.0, h + 4.0);
        scene.fill(FillRule::NonZero, transform, px(self.theme.page_shadow), None, &shadow);
        scene.fill(
            FillRule::NonZero,
            transform,
            px(settings.resolved_fill()),
            None,
            &sheet,
        );
        scene.stroke(
            &KStroke::new(hairline(camera)),
            transform,
            px(settings.resolved_fill().lerp(
                settings.resolved_fill().contrasting_text(),
                0.25,
            )),
            None,
            &sheet,
        );
    }

    fn draw_grid(
        &self,
        scene: &mut Scene,
        page: &Page,
        units: dc_model::Units,
        camera: &Camera,
        transform: Affine,
    ) {
        let minor_color = self.on_page(page, 0.09);
        let major_color = self.on_page(page, 0.18);
        let pitch = self.options.grid_spacing;
        if pitch <= 0.0 {
            return;
        }

        // The grid belongs to the page, so it stops at the sheet edge. Letting
        // it run across the surround would paint page-colored lines onto the
        // canvas, which reads as a glaring rectangle in a dark theme.
        let settings = page.resolved_settings();
        let visible = match settings.bounds_points(units) {
            Some((w, h)) if self.options.show_page_bounds => {
                let sheet = Rect::new(0.0, 0.0, w, h);
                let clipped = sheet.intersect(camera.visible_rect());
                if clipped.width() <= 0.0 || clipped.height() <= 0.0 {
                    return;
                }
                clipped
            }
            // An infinite canvas has no edge to stop at.
            _ => camera.visible_rect(),
        };
        let width = hairline(camera);

        // Below a few pixels of pitch the minor grid turns into a wash of
        // color, so it drops out and only every tenth line remains.
        let mut draw_minor = pitch * camera.zoom >= 5.0;
        let major_pitch = pitch * 10.0;
        let draw_major = major_pitch * camera.zoom >= 5.0;
        if !draw_minor && !draw_major {
            return;
        }

        if self.options.grid_style == GridStyle::Dots
            && self.draw_grid_dots(scene, visible, pitch, major_color, camera, transform)
        {
            return;
        }

        // A plain line grid is the emphasized lines only; the minor ones are
        // what make it read as graph paper.
        if self.options.grid_style == GridStyle::Lines {
            draw_minor = false;
        }

        let mut minor = BezPath::new();
        let mut major = BezPath::new();

        let first_x = (visible.x0 / pitch).floor() * pitch;
        let mut x = first_x;
        while x <= visible.x1 {
            let is_major = ((x / major_pitch).round() * major_pitch - x).abs() < pitch * 0.01;
            let target = if is_major { &mut major } else { &mut minor };
            if is_major || draw_minor {
                target.move_to((x, visible.y0));
                target.line_to((x, visible.y1));
            }
            x += pitch;
        }

        let first_y = (visible.y0 / pitch).floor() * pitch;
        let mut y = first_y;
        while y <= visible.y1 {
            let is_major = ((y / major_pitch).round() * major_pitch - y).abs() < pitch * 0.01;
            let target = if is_major { &mut major } else { &mut minor };
            if is_major || draw_minor {
                target.move_to((visible.x0, y));
                target.line_to((visible.x1, y));
            }
            y += pitch;
        }

        if draw_minor && !minor.is_empty() {
            scene.stroke(&KStroke::new(width), transform, px(minor_color), None, &minor);
        }
        if draw_major && !major.is_empty() {
            scene.stroke(&KStroke::new(width), transform, px(major_color), None, &major);
        }
    }

    /// Paints a dot at each grid intersection.
    ///
    /// Returns whether it drew. Zoomed far enough out the dots outnumber what
    /// is worth encoding and stop reading as a grid at all, so the caller is
    /// told to fall back to lines instead.
    fn draw_grid_dots(
        &self,
        scene: &mut Scene,
        visible: Rect,
        pitch: f64,
        color: dc_model::Color,
        camera: &Camera,
        transform: Affine,
    ) -> bool {
        let across = (visible.width() / pitch).floor() + 1.0;
        let down = (visible.height() / pitch).floor() + 1.0;
        if across * down > MAX_GRID_DOTS {
            return false;
        }

        // A dot keeps the same size on screen whatever the zoom, the way the
        // hairlines do, so the grid reads the same at every magnification.
        let radius = (1.1 / camera.zoom).min(pitch / 6.0);
        let mut dots = BezPath::new();

        let first_x = (visible.x0 / pitch).ceil() * pitch;
        let first_y = (visible.y0 / pitch).ceil() * pitch;
        let mut x = first_x;
        while x <= visible.x1 {
            let mut y = first_y;
            while y <= visible.y1 {
                // A square rather than a circle: at a dot's size on screen the
                // two are indistinguishable, and this is four lines where a
                // circle is four curves, across tens of thousands of dots.
                dots.move_to((x - radius, y - radius));
                dots.line_to((x + radius, y - radius));
                dots.line_to((x + radius, y + radius));
                dots.line_to((x - radius, y + radius));
                dots.close_path();
                y += pitch;
            }
            x += pitch;
        }

        if !dots.is_empty() {
            scene.fill(FillRule::NonZero, transform, px(color), None, &dots);
        }
        true
    }

    /// Paints shapes and connectors interleaved by z-index.
    fn draw_content(
        &mut self,
        scene: &mut Scene,
        page: &Page,
        routes: &std::collections::HashMap<String, RoutedLine>,
        transform: Affine,
        overlay: &Overlay<'_>,
    ) {
        enum Item<'a> {
            Shape(&'a Shape),
            Line(&'a Line),
        }

        let hidden = hidden_item_ids(page);
        let mut items: Vec<(i64, usize, Item<'_>)> = Vec::new();
        for (i, s) in page.shapes().iter().enumerate() {
            if hidden.contains(s.id.as_str()) {
                continue;
            }
            items.push((s.resolved_z_index(), i, Item::Shape(s)));
        }
        let shape_count = page.shapes().len();
        for (i, l) in page.lines().iter().enumerate() {
            if hidden.contains(l.id.as_str()) {
                continue;
            }
            items.push((l.resolved_z_index(), shape_count + i, Item::Line(l)));
        }
        // Equal z-index falls back to document order, which keeps painting
        // stable across frames.
        items.sort_by_key(|(z, order, _)| (*z, *order));

        for (_, _, item) in items {
            match item {
                Item::Shape(s) => self.draw_shape(scene, s, transform, overlay),
                Item::Line(l) => {
                    if let Some(route) = routes.get(&l.id) {
                        let editing = overlay.editing == Some(l.id.as_str());
                        self.draw_line(scene, l, route, page, transform, editing);
                    }
                }
            }
        }
    }

    fn draw_shape(
        &mut self,
        scene: &mut Scene,
        shape: &Shape,
        transform: Affine,
        overlay: &Overlay<'_>,
    ) {
        let geo = dc_geom::geometry(shape);
        let style = shape.resolved_style();
        let alpha = shape.opacity_factor();
        if alpha <= 0.0 {
            return;
        }

        // A hotspot is an invisible click target in the authoring tool; the
        // editor shows it as a dashed region so it can be found and moved.
        if shape.shape_type == ShapeType::Hotspot {
            let dashed = KStroke::new(1.0).with_dashes(0.0, [4.0, 3.0]);
            scene.stroke(
                &dashed,
                transform,
                px(self.theme.hotspot_outline.with_alpha(scale_alpha(255, alpha))),
                None,
                &geo.outline,
            );
            return;
        }

        if geo.filled {
            match style.resolved_fill() {
                PaintFill::Color { color } => {
                    scene.fill(
                        FillRule::NonZero,
                        transform,
                        px(color.with_alpha(scale_alpha(color.a, alpha))),
                        None,
                        &geo.outline,
                    );
                }
                PaintFill::Image { r#ref, image_scale, .. } => {
                    self.draw_image_fill(
                        scene,
                        &geo.outline,
                        shape.bounding_box,
                        r#ref.as_deref(),
                        image_scale.unwrap_or_default(),
                        transform,
                        alpha,
                    );
                }
            }
        }

        // An image block carries its asset outside `style`.
        if shape.shape_type == ShapeType::Image {
            let name = shape.image.as_ref().and_then(|i| i.r#ref.as_deref());
            self.draw_image_fill(
                scene,
                &geo.outline,
                shape.bounding_box,
                name,
                dc_model::ImageScale::Fit,
                transform,
                alpha,
            );
        }

        let stroke = shape.effective_stroke();
        if !stroke.is_invisible() {
            let color = stroke.resolved_color();
            // A few types keep their outline for bounds and hit testing only,
            // and carry everything visible in their decoration.
            if geo.stroked {
                scene.stroke(
                    &to_kurbo_stroke(stroke.resolved_width(), stroke.resolved_style()),
                    transform,
                    px(color.with_alpha(scale_alpha(color.a, alpha))),
                    None,
                    &geo.outline,
                );
            }
            if let Some(decoration) = &geo.decoration {
                scene.stroke(
                    &to_kurbo_stroke(stroke.resolved_width(), stroke.resolved_style()),
                    transform,
                    px(color.with_alpha(scale_alpha(color.a, alpha))),
                    None,
                    decoration,
                );
            }
        }

        if overlay.attach_target == Some(shape.id.as_str()) {
            scene.stroke(
                &KStroke::new(2.0),
                transform,
                px(self.theme.attach_highlight),
                None,
                &geo.outline,
            );
        }

        if overlay.editing != Some(shape.id.as_str()) {
            self.draw_shape_text(scene, shape, &geo, transform, alpha);
        }
    }

    /// Paints a referenced image inside a shape, or a placeholder when the
    /// asset is missing or undecodable.
    #[allow(clippy::too_many_arguments)]
    fn draw_image_fill(
        &self,
        scene: &mut Scene,
        outline: &BezPath,
        bounds: BoundingBox,
        name: Option<&str>,
        mode: dc_model::ImageScale,
        transform: Affine,
        alpha: f64,
    ) {
        let area = Rect::new(bounds.x, bounds.y, bounds.right(), bounds.bottom());
        let decoded = name.and_then(|n| self.images.get(n));

        let Some(image) = decoded else {
            // No asset: a flat placeholder keeps the shape visible and
            // selectable rather than leaving a hole.
            let color = self.theme.image_placeholder;
            scene.fill(
                FillRule::NonZero,
                transform,
                px(color.with_alpha(scale_alpha(color.a, alpha))),
                None,
                outline,
            );
            return;
        };

        // Clip to the shape so a non-rectangular outline crops the image.
        scene.push_clip_layer(FillRule::NonZero, transform, outline);
        let brush = peniko::ImageBrush::new(image.clone())
            .with_alpha(alpha as f32)
            .with_extend(peniko::Extend::Pad);
        let placement = images::placement(mode, area, image.width, image.height);
        scene.draw_image(brush.as_ref(), transform * placement);
        scene.pop_layer();
    }

    fn draw_shape_text(
        &mut self,
        scene: &mut Scene,
        shape: &Shape,
        geo: &dc_geom::ShapeGeometry,
        transform: Affine,
        alpha: f64,
    ) {
        let raw = shape.text_str();
        if raw.is_empty() || !shape.shape_type.accepts_text() {
            return;
        }

        let mut base = dc_model::TextStyle::default();
        base.color = shape.resolved_style().resolved_text_color();
        if alpha < 1.0 {
            base.color = base.color.with_alpha(scale_alpha(base.color.a, alpha));
        }

        let parsed = dc_model::markup::parse(raw, base);
        let bb = shape.bounding_box;
        let insets = geo.text_insets;
        let area = Rect::new(
            bb.x + insets.x0,
            bb.y + insets.y0,
            bb.right() - insets.x1,
            bb.bottom() - insets.y1,
        );
        if area.width() <= 1.0 || area.height() <= 0.0 {
            return;
        }

        let block = self.text.layout(&parsed, Some(area.width()));

        // Rotated shapes carry their text with them.
        let rotation = bb.rotation_degrees();
        let full = if rotation != 0.0 {
            let c = bb.center();
            transform * Affine::rotate_about(rotation.to_radians(), kurbo::Point::new(c.x, c.y))
        } else {
            transform
        };
        text::draw(scene, &block, area, full);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &mut self,
        scene: &mut Scene,
        line: &Line,
        route: &RoutedLine,
        page: &Page,
        transform: Affine,
        suppress_labels: bool,
    ) {
        let background = self.page_background(page);
        let stroke = line.resolved_stroke();
        let color = stroke.resolved_color();
        let width = stroke.resolved_width();

        if !stroke.is_invisible() {
            scene.stroke(
                &to_kurbo_stroke(width, stroke.resolved_style()),
                transform,
                px(color),
                None,
                &route.path,
            );
        }

        for (endpoint, tip, dir) in [
            (&line.endpoint1, route.start, -route.start_dir),
            (&line.endpoint2, route.end, -route.end_dir),
        ] {
            // The glyph points back along the connector, so the outward normal
            // is negated relative to the route's exit direction.
            let Some(marker) =
                dc_geom::endpoint_marker(endpoint.style(), tip, -dir, width)
            else {
                continue;
            };
            match marker.fill {
                MarkerFill::Solid => {
                    scene.fill(FillRule::NonZero, transform, px(color), None, &marker.path);
                }
                MarkerFill::Hollow => {
                    // Hollow means "the page shows through", so it is filled
                    // with the sheet color rather than the panel color.
                    scene.fill(
                        FillRule::NonZero,
                        transform,
                        px(background),
                        None,
                        &marker.path,
                    );
                    scene.stroke(&KStroke::new(width), transform, px(color), None, &marker.path);
                }
                MarkerFill::Open => {
                    scene.stroke(&KStroke::new(width), transform, px(color), None, &marker.path);
                }
            }
        }

        if !suppress_labels {
            self.draw_line_labels(scene, line, route, background, transform);
        }
    }

    fn draw_line_labels(
        &mut self,
        scene: &mut Scene,
        line: &Line,
        route: &RoutedLine,
        background: dc_model::Color,
        transform: Affine,
    ) {
        for label in line.labels() {
            if label.text.is_empty() {
                continue;
            }
            let t = label.resolved_position();
            let anchor = route.point_at_fraction(t);

            let mut base = dc_model::TextStyle::default();
            // A label normally takes the connector's color, but a dark stroke
            // on a dark page would be invisible, so poor contrast falls back
            // to whatever reads against the sheet.
            let stroke_color = line.resolved_stroke().resolved_color();
            base.color = if contrast_ratio(stroke_color, background) >= 3.0 {
                stroke_color
            } else {
                background.contrasting_text()
            };
            let parsed = dc_model::markup::parse(&label.text, base);
            let block = self.text.layout(&parsed, None);

            let (w, h) = (block.width().max(1.0), block.height().max(1.0));
            // Offset off the line so the label does not sit on the stroke.
            let gap = 3.0 + line.resolved_stroke().resolved_width();
            let dy = match label.resolved_side() {
                LabelSide::Top => -h - gap,
                LabelSide::Middle => -h / 2.0,
                LabelSide::Bottom => gap,
            };
            let area = Rect::new(
                anchor.x - w / 2.0,
                anchor.y + dy,
                anchor.x + w / 2.0,
                anchor.y + dy + h,
            );

            // A plate behind the text keeps it readable where it crosses the
            // connector or another shape.
            if label.resolved_side() == LabelSide::Middle {
                let plate = area.inflate(3.0, 1.0);
                scene.fill(FillRule::NonZero, transform, px(background), None, &plate);
            }
            text::draw(scene, &block, area, transform);
        }
    }

    /// Selection, hover, marquee, and guides, encoded in screen space so they
    /// keep a constant on-screen size at any zoom.
    fn draw_overlay(
        &mut self,
        scene: &mut Scene,
        page: &Page,
        routes: &std::collections::HashMap<String, RoutedLine>,
        camera: &Camera,
        overlay: &Overlay<'_>,
    ) {
        let ident = Affine::IDENTITY;

        for guide in overlay.snap_guides {
            let mut p = BezPath::new();
            if guide.vertical {
                let x = camera.doc_to_screen(Point::new(guide.at, 0.0)).x;
                p.move_to((x, 0.0));
                p.line_to((x, camera.viewport.y));
            } else {
                let y = camera.doc_to_screen(Point::new(0.0, guide.at)).y;
                p.move_to((0.0, y));
                p.line_to((camera.viewport.x, y));
            }
            scene.stroke(
                &KStroke::new(1.0).with_dashes(0.0, [4.0, 4.0]),
                ident,
                px(self.theme.snap_guide),
                None,
                &p,
            );
        }

        if let Some((from, to)) = overlay.pending_connector {
            let a = camera.doc_to_screen(from);
            let b = camera.doc_to_screen(to);
            let mut p = BezPath::new();
            p.move_to((a.x, a.y));
            p.line_to((b.x, b.y));
            scene.stroke(
                &KStroke::new(1.5).with_dashes(0.0, [5.0, 4.0]),
                ident,
                px(self.theme.accent),
                None,
                &p,
            );
        }

        // Quick-connect arrows: a filled chevron pointing away from the
        // shape, so it reads as "drag out this way" rather than as another
        // handle to grab.
        for arrow in overlay.quick_connect {
            let p = camera.doc_to_screen(*arrow);
            let r = self.metrics.quick_connect_radius;
            let dot = Circle::new((p.x, p.y), r);
            scene.fill(FillRule::NonZero, ident, px(self.theme.accent), None, &dot);
            scene.stroke(
                &KStroke::new(1.0),
                ident,
                px(self.theme.handle_fill),
                None,
                &dot,
            );
        }

        // Connection points, drawn after the pending connector so the port
        // being aimed at reads as on top of the line rather than under it.
        for port in overlay.ports {
            let p = camera.doc_to_screen(*port);
            let dot = Circle::new((p.x, p.y), self.metrics.port_radius);
            scene.fill(FillRule::NonZero, ident, px(self.theme.handle_fill), None, &dot);
            scene.stroke(
                &KStroke::new(1.5),
                ident,
                px(self.theme.attach_highlight),
                None,
                &dot,
            );
        }

        if let Some(hovered) = overlay.hovered {
            if !overlay.selected.iter().any(|s| s == hovered) {
                if let Some(shape) = page.shape(hovered) {
                    let b = to_screen_box(camera, shape.bounding_box.aabb());
                    scene.stroke(
                        &KStroke::new(1.0),
                        ident,
                        px(self.theme.accent.with_alpha(0x80)),
                        None,
                        &to_rect(b),
                    );
                }
            }
        }

        // Shapes get a box with handles; connectors get vertex dots.
        let mut selection_box: Option<BoundingBox> = None;
        for id in overlay.selected {
            if let Some(shape) = page.shape(id) {
                let aabb = shape.bounding_box.aabb();
                selection_box = Some(match selection_box {
                    Some(b) => b.union(&aabb),
                    None => aabb,
                });
                let b = to_screen_box(camera, aabb);
                scene.stroke(&KStroke::new(1.0), ident, px(self.theme.accent), None, &to_rect(b));
            } else if let Some(route) = routes.get(id.as_str()) {
                scene.stroke(
                    &KStroke::new(3.0),
                    camera.transform(),
                    px(self.theme.accent.with_alpha(0x55)),
                    None,
                    &route.path,
                );
                for p in &route.points {
                    let s = camera.doc_to_screen(*p);
                    let dot = Circle::new((s.x, s.y), 3.5);
                    scene.fill(FillRule::NonZero, ident, px(self.theme.handle_fill), None, &dot);
                    scene.stroke(&KStroke::new(1.5), ident, px(self.theme.accent), None, &dot);
                }
            }
        }

        if let Some(doc_box) = selection_box {
            let screen = to_screen_box(camera, doc_box);
            // A single dashed box around a multiple selection.
            if overlay.selected.len() > 1 {
                scene.stroke(
                    &KStroke::new(1.0).with_dashes(0.0, [3.0, 3.0]),
                    ident,
                    px(self.theme.accent),
                    None,
                    &to_rect(screen),
                );
            }
            self.draw_handles(scene, screen);
        }

        if let Some(marquee) = overlay.marquee {
            let screen = to_rect(to_screen_box(camera, marquee));
            scene.fill(FillRule::NonZero, ident, px(self.theme.marquee_fill), None, &screen);
            scene.stroke(
                &KStroke::new(1.0),
                ident,
                px(self.theme.marquee_stroke),
                None,
                &screen,
            );
        }
    }

    fn draw_handles(&self, scene: &mut Scene, screen_box: BoundingBox) {
        let size = self.metrics.handle_size;
        let half = size / 2.0;

        // The arm connecting the box to the rotation handle.
        let top_center = handle_position(screen_box, HandleKind::TopCenter);
        let rotate = handle_position(screen_box, HandleKind::Rotate);
        let mut arm = BezPath::new();
        arm.move_to((top_center.x, top_center.y));
        arm.line_to((rotate.x, rotate.y));
        scene.stroke(
            &KStroke::new(1.0),
            Affine::IDENTITY,
            px(self.theme.accent),
            None,
            &arm,
        );

        for (kind, p) in all_handles(screen_box) {
            if kind == HandleKind::Rotate {
                let dot = Circle::new((p.x, p.y), half);
                scene.fill(
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    px(self.theme.handle_fill),
                    None,
                    &dot,
                );
                scene.stroke(
                    &KStroke::new(1.5),
                    Affine::IDENTITY,
                    px(self.theme.handle_stroke),
                    None,
                    &dot,
                );
                continue;
            }
            let r = Rect::new(p.x - half, p.y - half, p.x + half, p.y + half);
            scene.fill(
                FillRule::NonZero,
                Affine::IDENTITY,
                px(self.theme.handle_fill),
                None,
                &r,
            );
            scene.stroke(
                &KStroke::new(1.5),
                Affine::IDENTITY,
                px(self.theme.handle_stroke),
                None,
                &r,
            );
        }
    }
}

/// Finds the topmost shape or connector under a document-space point.
///
/// Shapes are tested front to back. Connectors are tested within `tolerance`
/// document units of their route, since a hairline is hard to hit exactly.
pub fn pick(page: &Page, at: Point, tolerance: f64, route_around_shapes: bool) -> Option<String> {
    let unpickable = unpickable_item_ids(page);
    for shape in page.shapes_in_paint_order().into_iter().rev() {
        if unpickable.contains(shape.id.as_str()) {
            continue;
        }
        if shape.bounding_box.contains_rotated(at) {
            return Some(shape.id.clone());
        }
    }

    let mut router = Router::new(page);
    router.avoid_obstacles = route_around_shapes;
    let mut best: Option<(f64, String)> = None;
    for line in page.lines() {
        if unpickable.contains(line.id.as_str()) {
            continue;
        }
        let Some(route) = router.route(line) else {
            continue;
        };
        let d = route
            .points
            .windows(2)
            .map(|w| distance_to_segment(at, w[0], w[1]))
            .fold(f64::MAX, f64::min);
        if d <= tolerance && best.as_ref().is_none_or(|(b, _)| d < *b) {
            best = Some((d, line.id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

/// Every shape whose box intersects a region, for marquee selection.
pub fn pick_in_region(page: &Page, region: BoundingBox) -> Vec<String> {
    let unpickable = unpickable_item_ids(page);
    page.shapes()
        .iter()
        .filter(|s| !unpickable.contains(s.id.as_str()) && s.bounding_box.aabb().intersects(&region))
        .map(|s| s.id.clone())
        .collect()
}

/// The ids of every item sitting in a hidden layer — nothing draws them.
fn hidden_item_ids(page: &Page) -> std::collections::HashSet<&str> {
    page.layers()
        .iter()
        .filter(|l| !l.is_visible())
        .flat_map(|l| l.items())
        .map(|s| s.as_str())
        .collect()
}

/// The ids of every item that cannot be picked: hidden (nothing to click)
/// or locked (visible, but not selectable).
fn unpickable_item_ids(page: &Page) -> std::collections::HashSet<&str> {
    page.layers()
        .iter()
        .filter(|l| !l.is_visible() || l.is_locked())
        .flat_map(|l| l.items())
        .map(|s| s.as_str())
        .collect()
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-12 {
        return p.distance_to(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    p.distance_to(Point::new(a.x + dx * t, a.y + dy * t))
}

/// A stroke one screen pixel wide, expressed in document units.
fn hairline(camera: &Camera) -> f64 {
    1.0 / camera.zoom.max(1e-6)
}

fn to_kurbo_stroke(width: f64, style: StrokeStyle) -> KStroke {
    let s = KStroke::new(width.max(0.01));
    match style.dash_pattern(width) {
        Some(dashes) => s.with_dashes(0.0, dashes),
        None => s,
    }
}

fn to_screen_box(camera: &Camera, doc: BoundingBox) -> BoundingBox {
    let tl = camera.doc_to_screen(Point::new(doc.x, doc.y));
    let br = camera.doc_to_screen(Point::new(doc.right(), doc.bottom()));
    BoundingBox::new(tl.x, tl.y, br.x - tl.x, br.y - tl.y)
}

fn to_rect(b: BoundingBox) -> Rect {
    Rect::new(b.x, b.y, b.right(), b.bottom())
}

fn scale_alpha(a: u8, factor: f64) -> u8 {
    (a as f64 * factor.clamp(0.0, 1.0)).round() as u8
}

/// The WCAG contrast ratio between two colors, from 1.0 to 21.0.
fn contrast_ratio(a: dc_model::Color, b: dc_model::Color) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn px(c: dc_model::Color) -> PenikoColor {
    PenikoColor::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Re-exported so applications can collect obstacles without depending on
/// `dc-geom` directly.
pub type RouteObstacles = Obstacles;

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{Endpoint, EndpointStyle, Layer, LineType, ShapeType, Style, Units};
    use kurbo::Vec2;

    fn doc_with_flowchart() -> Document {
        let mut doc = Document::new();
        doc.set_units(Units::Pt);
        let page = &mut doc.pages[0];
        page.add_shape(
            Shape::new("start", ShapeType::Terminator, BoundingBox::new(60.0, 40.0, 140.0, 50.0))
                .with_text("Start"),
        );
        page.add_shape(
            Shape::new("check", ShapeType::Decision, BoundingBox::new(60.0, 160.0, 140.0, 100.0))
                .with_text("Approved?"),
        );
        page.add_shape(
            Shape::new("done", ShapeType::Process, BoundingBox::new(300.0, 180.0, 140.0, 60.0))
                .with_text("Ship it")
                .with_style(Style::default()),
        );
        page.add_line(
            Line::new(
                "e1",
                Endpoint::at_shape("start", EndpointStyle::None),
                Endpoint::at_shape("check", EndpointStyle::Arrow),
            )
            .with_line_type(LineType::Elbow),
        );
        page.add_line(
            Line::new(
                "e2",
                Endpoint::at_shape("check", EndpointStyle::None),
                Endpoint::at_shape("done", EndpointStyle::Arrow),
            )
            .with_line_type(LineType::Elbow)
            .with_label("yes"),
        );
        doc
    }

    fn camera() -> Camera {
        Camera::new(Vec2::new(1000.0, 700.0))
    }

    #[test]
    fn drawing_a_document_encodes_paths() {
        let mut r = Renderer::new();
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc_with_flowchart(), 0, &camera(), &Overlay::default());

        assert!(scene.encoding().n_paths > 0, "nothing was encoded");
        assert!(scene.encoding().draw_tags.len() > 0);
    }

    #[test]
    fn drawing_is_deterministic() {
        let mut r = Renderer::new();
        let doc = doc_with_flowchart();
        let cam = camera();

        let mut a = Scene::new();
        r.draw(&mut a, &doc, 0, &cam, &Overlay::default());
        let counts_a = (a.encoding().n_paths, a.encoding().draw_tags.len());

        let mut b = Scene::new();
        r.draw(&mut b, &doc, 0, &cam, &Overlay::default());
        let counts_b = (b.encoding().n_paths, b.encoding().draw_tags.len());

        assert_eq!(counts_a, counts_b, "two identical draws differed");
    }

    #[test]
    fn redrawing_resets_rather_than_accumulates() {
        let mut r = Renderer::new();
        let doc = doc_with_flowchart();
        let cam = camera();
        let mut scene = Scene::new();

        r.draw(&mut scene, &doc, 0, &cam, &Overlay::default());
        let first = scene.encoding().n_paths;
        r.draw(&mut scene, &doc, 0, &cam, &Overlay::default());
        assert_eq!(scene.encoding().n_paths, first, "scene grew on redraw");
    }

    #[test]
    fn an_out_of_range_page_still_paints_the_canvas() {
        let mut r = Renderer::new();
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc_with_flowchart(), 99, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 0, "the canvas fill is missing");
    }

    #[test]
    fn an_empty_document_does_not_panic() {
        let mut r = Renderer::new();
        let mut scene = Scene::new();
        let doc = Document::new();
        r.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 0);
    }

    #[test]
    fn selection_overlay_adds_handles() {
        let mut r = Renderer::new();
        let doc = doc_with_flowchart();
        let cam = camera();

        let mut plain = Scene::new();
        r.draw(&mut plain, &doc, 0, &cam, &Overlay::default());

        let selected = vec!["start".to_string()];
        let mut with_selection = Scene::new();
        r.draw(
            &mut with_selection,
            &doc,
            0,
            &cam,
            &Overlay { selected: &selected, ..Default::default() },
        );

        assert!(
            with_selection.encoding().n_paths > plain.encoding().n_paths,
            "selection should add handle geometry"
        );
    }

    #[test]
    fn grid_can_be_turned_off() {
        let doc = doc_with_flowchart();
        let cam = camera();

        let mut with_grid = Renderer::new();
        let mut a = Scene::new();
        with_grid.draw(&mut a, &doc, 0, &cam, &Overlay::default());

        let mut without = Renderer::new();
        without.options.show_grid = false;
        let mut b = Scene::new();
        without.draw(&mut b, &doc, 0, &cam, &Overlay::default());

        assert!(a.encoding().n_paths > b.encoding().n_paths);
    }

    #[test]
    fn each_grid_style_draws_something_different() {
        let doc = doc_with_flowchart();
        let cam = camera();

        let encode = |style: GridStyle| {
            let mut r = Renderer::new();
            r.options.grid_style = style;
            let mut scene = Scene::new();
            r.draw(&mut scene, &doc, 0, &cam, &Overlay::default());
            scene.encoding().n_paths
        };

        let graph = encode(GridStyle::Graph);
        let lines = encode(GridStyle::Lines);
        let dots = encode(GridStyle::Dots);

        assert!(graph > 0 && lines > 0 && dots > 0);
        assert_ne!(graph, dots, "a dot grid is not a line grid");
        assert_ne!(graph, lines, "plain lines drop the minor ones");
    }

    #[test]
    fn a_plain_line_grid_is_quieter_than_graph_paper() {
        let doc = doc_with_flowchart();
        let cam = camera();

        let mut graph = Renderer::new();
        graph.options.grid_style = GridStyle::Graph;
        let mut a = Scene::new();
        graph.draw(&mut a, &doc, 0, &cam, &Overlay::default());

        let mut lines = Renderer::new();
        lines.options.grid_style = GridStyle::Lines;
        let mut b = Scene::new();
        lines.draw(&mut b, &doc, 0, &cam, &Overlay::default());

        assert!(
            b.encoding().n_paths <= a.encoding().n_paths,
            "dropping the minor lines cannot add geometry"
        );
    }

    #[test]
    fn a_dot_grid_falls_back_to_lines_when_it_would_be_a_wash() {
        // Zoomed out far enough the dots stop reading as a grid, and encoding
        // tens of thousands of them would cost more than it conveys.
        let doc = doc_with_flowchart();
        let mut r = Renderer::new();
        r.options.grid_style = GridStyle::Dots;

        let mut far = camera();
        far.zoom = 0.08;
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc, 0, &far, &Overlay::default());

        // The point is that it still draws, and does not hang or explode.
        assert!(scene.encoding().n_paths > 0);
        assert!(
            scene.encoding().n_paths < 5_000,
            "the fallback should be cheap, got {}",
            scene.encoding().n_paths
        );
    }

    #[test]
    fn grid_styles_have_labels() {
        for style in [GridStyle::Graph, GridStyle::Lines, GridStyle::Dots] {
            assert!(!style.label().is_empty());
        }
        assert_eq!(GridStyle::default(), GridStyle::Graph);
    }

    #[test]
    fn grid_drops_out_when_zoomed_far_out() {
        let doc = doc_with_flowchart();
        let mut r = Renderer::new();

        let mut near = camera();
        near.zoom = 1.0;
        let mut a = Scene::new();
        r.draw(&mut a, &doc, 0, &near, &Overlay::default());

        let mut far = camera();
        far.zoom = 0.03;
        let mut b = Scene::new();
        r.draw(&mut b, &doc, 0, &far, &Overlay::default());

        assert!(
            b.encoding().n_paths < a.encoding().n_paths,
            "a dense grid should not be encoded when it would be a wash"
        );
    }

    #[test]
    fn marquee_and_guides_encode() {
        let mut r = Renderer::new();
        let doc = doc_with_flowchart();
        let cam = camera();

        let mut plain = Scene::new();
        r.draw(&mut plain, &doc, 0, &cam, &Overlay::default());

        let guides = [SnapGuide { at: 60.0, vertical: true }];
        let mut extra = Scene::new();
        r.draw(
            &mut extra,
            &doc,
            0,
            &cam,
            &Overlay {
                marquee: Some(BoundingBox::new(0.0, 0.0, 200.0, 200.0)),
                snap_guides: &guides,
                pending_connector: Some((Point::new(0.0, 0.0), Point::new(50.0, 50.0))),
                ..Default::default()
            },
        );

        assert!(extra.encoding().n_paths > plain.encoding().n_paths);
    }

    #[test]
    fn transparent_shapes_are_skipped() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        let mut s = Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 50.0, 50.0));
        s.opacity = Some(0.0);
        page.add_shape(s);

        let mut r = Renderer::new();
        r.options.show_grid = false;
        r.options.show_page_bounds = false;
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());

        // Only the viewport fill should be present.
        assert_eq!(scene.encoding().n_paths, 1, "a fully transparent shape was drawn");
    }

    #[test]
    fn picking_prefers_the_front_shape() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        let mut back = Shape::new("back", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0));
        back.z_index = Some(0);
        let mut front = Shape::new("front", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0));
        front.z_index = Some(10);
        page.add_shape(back);
        page.add_shape(front);

        assert_eq!(
            pick(&doc.pages[0], Point::new(50.0, 50.0), 4.0, true).as_deref(),
            Some("front")
        );
    }

    #[test]
    fn a_locked_layers_shape_cannot_be_picked() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        page.add_shape(Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0)));
        page.layers = Some(vec![Layer {
            id: "l1".into(),
            title: None,
            items: Some(vec!["s1".into()]),
            visible: None,
            locked: Some(true),
            extra: Default::default(),
        }]);

        assert_eq!(pick(&doc.pages[0], Point::new(50.0, 50.0), 4.0, true), None);
        assert!(pick_in_region(&doc.pages[0], BoundingBox::new(0.0, 0.0, 100.0, 100.0)).is_empty());
    }

    #[test]
    fn a_hidden_layers_shape_cannot_be_picked_either() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        page.add_shape(Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0)));
        page.layers = Some(vec![Layer {
            id: "l1".into(),
            title: None,
            items: Some(vec!["s1".into()]),
            visible: Some(false),
            locked: None,
            extra: Default::default(),
        }]);

        assert_eq!(pick(&doc.pages[0], Point::new(50.0, 50.0), 4.0, true), None);
    }

    #[test]
    fn a_shape_outside_any_layer_picks_normally() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        page.add_shape(Shape::new("s1", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0)));
        page.layers = Some(vec![Layer {
            id: "l1".into(),
            title: None,
            items: Some(vec!["someone-else".into()]),
            visible: Some(false),
            locked: Some(true),
            extra: Default::default(),
        }]);

        assert_eq!(pick(&doc.pages[0], Point::new(50.0, 50.0), 4.0, true).as_deref(), Some("s1"));
    }

    #[test]
    fn a_hidden_layers_shape_is_not_drawn() {
        let mut doc = Document::new();
        let page = &mut doc.pages[0];
        page.add_shape(Shape::new("hidden", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 100.0, 100.0)));
        page.layers = Some(vec![Layer {
            id: "l1".into(),
            title: None,
            items: Some(vec!["hidden".into()]),
            visible: Some(false),
            locked: None,
            extra: Default::default(),
        }]);

        let mut r = Renderer::new();
        r.options.show_grid = false;
        r.options.show_page_bounds = false;
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());

        // Only the viewport fill should be present, the same as a fully
        // transparent shape.
        assert_eq!(scene.encoding().n_paths, 1, "a hidden-layer shape was drawn");
    }

    #[test]
    fn picking_finds_a_connector_near_its_route() {
        let doc = doc_with_flowchart();
        let page = &doc.pages[0];
        let router = Router::new(page);
        let route = router.route(page.line("e1").unwrap()).unwrap();
        let mid = route.point_at_fraction(0.5);

        assert_eq!(pick(page, mid, 5.0, true).as_deref(), Some("e1"));
    }

    #[test]
    fn picking_empty_space_returns_nothing() {
        let doc = doc_with_flowchart();
        assert_eq!(pick(&doc.pages[0], Point::new(900.0, 900.0), 4.0, true), None);
    }

    #[test]
    fn marquee_selects_intersecting_shapes_only() {
        let doc = doc_with_flowchart();
        let hits = pick_in_region(&doc.pages[0], BoundingBox::new(0.0, 0.0, 250.0, 120.0));
        assert_eq!(hits, vec!["start".to_string()]);
    }

    #[test]
    fn dashed_strokes_carry_a_dash_pattern() {
        assert!(to_kurbo_stroke(2.0, StrokeStyle::Dashed).dash_pattern.len() > 0);
        assert!(to_kurbo_stroke(2.0, StrokeStyle::Solid).dash_pattern.is_empty());
    }

    #[test]
    fn hairline_tracks_zoom() {
        let mut c = camera();
        c.zoom = 4.0;
        assert_eq!(hairline(&c), 0.25, "one screen pixel at 4x is a quarter unit");
    }

    #[test]
    fn on_page_marks_contrast_with_the_sheet_not_the_panel() {
        // The bug this guards: in a dark theme the grid was tinted from the
        // panel color and drawn onto a white sheet, producing near-black
        // gridlines over the artwork.
        let renderer = Renderer::with_theme(Theme::dark());
        let page = Page::new("p", "P");

        let background = renderer.page_background(&page);
        let grid = renderer.on_page(&page, 0.09);
        assert_eq!(background, dc_model::Color::WHITE, "an unstyled page is white");
        assert!(
            contrast_ratio(grid, background) < 1.5,
            "a gridline should barely differ from its page, got {:?}",
            grid
        );
    }

    #[test]
    fn an_explicitly_dark_page_gets_light_marks() {
        let renderer = Renderer::new();
        let mut page = Page::new("p", "P");
        page.settings = Some(dc_model::PageSettings {
            fill_color: Some(dc_model::Color::rgb(0x10, 0x10, 0x10)),
            ..Default::default()
        });

        let grid = renderer.on_page(&page, 0.09);
        let background = renderer.page_background(&page);
        assert!(
            grid.luminance() > background.luminance(),
            "marks on a dark page must be lighter than it"
        );
    }

    #[test]
    fn the_grid_stops_at_the_page_edge() {
        // Panning far off the sheet should leave nothing to draw.
        let doc = doc_with_flowchart();
        let mut renderer = Renderer::new();
        renderer.options.show_page_bounds = true;

        let mut away = camera();
        away.origin = Point::new(5000.0, 5000.0);
        let mut off_sheet = Scene::new();
        renderer.draw(&mut off_sheet, &doc, 0, &away, &Overlay::default());

        let mut on_sheet = Scene::new();
        renderer.draw(&mut on_sheet, &doc, 0, &camera(), &Overlay::default());

        assert!(
            off_sheet.encoding().n_paths < on_sheet.encoding().n_paths,
            "the grid should not be painted across the surround"
        );
    }

    #[test]
    fn a_label_reads_against_a_dark_page() {
        // A black connector stroke on a dark page would make its label
        // invisible, so the label color falls back to the page contrast.
        let mut doc = Document::new();
        {
            let page = &mut doc.pages[0];
            page.settings = Some(dc_model::PageSettings {
                fill_color: Some(dc_model::Color::rgb(0x11, 0x11, 0x11)),
                ..Default::default()
            });
            page.add_shape(Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 60.0, 40.0)));
            page.add_shape(Shape::new("b", ShapeType::Rectangle, BoundingBox::new(300.0, 0.0, 60.0, 40.0)));
            page.add_line(
                Line::new(
                    "l",
                    Endpoint::at_shape("a", EndpointStyle::None),
                    Endpoint::at_shape("b", EndpointStyle::Arrow),
                )
                .with_label("yes"),
            );
        }

        // The assertion is on the rule rather than on pixels: a near-black
        // stroke against a near-black page must not be used for the label.
        let background = dc_model::Color::rgb(0x11, 0x11, 0x11);
        let stroke = dc_model::Color::BLACK;
        assert!(contrast_ratio(stroke, background) < 3.0, "precondition");
        assert!(
            contrast_ratio(background.contrasting_text(), background) >= 3.0,
            "the fallback must actually be readable"
        );

        let mut renderer = Renderer::new();
        let mut scene = Scene::new();
        renderer.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 0);
    }

    #[test]
    fn dark_theme_renders_too() {
        let mut r = Renderer::with_theme(Theme::dark());
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc_with_flowchart(), 0, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 0);
    }

    #[test]
    fn every_shape_type_renders_without_panicking() {
        let mut doc = Document::new();
        {
            let page = &mut doc.pages[0];
            for (i, name) in ShapeType::KNOWN.iter().enumerate() {
                let x = (i % 6) as f64 * 160.0;
                let y = (i / 6) as f64 * 110.0;
                page.add_shape(
                    Shape::new(
                        format!("s{i}"),
                        ShapeType::from(*name),
                        BoundingBox::new(x, y, 140.0, 80.0),
                    )
                    .with_text(*name),
                );
            }
        }

        let mut r = Renderer::new();
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 20);
    }

    #[test]
    fn every_endpoint_style_renders_without_panicking() {
        let mut doc = Document::new();
        {
            let page = &mut doc.pages[0];
            page.add_shape(Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 80.0, 40.0)));
            page.add_shape(Shape::new("b", ShapeType::Rectangle, BoundingBox::new(400.0, 0.0, 80.0, 40.0)));
            for (i, style) in EndpointStyle::KNOWN.iter().enumerate() {
                page.add_line(Line::new(
                    format!("l{i}"),
                    Endpoint::at_shape("a", EndpointStyle::from(*style)),
                    Endpoint::at_shape("b", EndpointStyle::from(*style)),
                ));
            }
        }

        let mut r = Renderer::new();
        let mut scene = Scene::new();
        r.draw(&mut scene, &doc, 0, &camera(), &Overlay::default());
        assert!(scene.encoding().n_paths > 20);
    }
}
