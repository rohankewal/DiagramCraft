//! Outlines for every published shape type.
//!
//! Each outline is built axis-aligned in document coordinates and then rotated
//! about the box center, matching the format's clockwise rotation convention.
//!
//! Proportions follow the conventional public symbol vocabularies — ANSI/ISO
//! 5807 for flowcharts, OMG's BPMN 2.0 notation for the BPMN library, and plain
//! regular polygons elsewhere — rather than any measurement of another
//! program's output.

use dc_model::{RelPoint, Shape, ShapeType};
use crate::extended;
use kurbo::{Affine, BezPath, Ellipse, Insets, Point as KPoint, Rect, RoundedRect, Shape as _};

/// Bezier control-point ratio that approximates a quarter ellipse.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// Curve flattening tolerance when converting primitives to paths. Well below a
/// device pixel at normal zoom.
const PATH_TOLERANCE: f64 = 0.05;

/// The drawable geometry of a shape.
#[derive(Clone, Debug)]
pub struct ShapeGeometry {
    /// The main outline. Filled when `filled` is set, and always stroked.
    pub outline: BezPath,
    /// Interior detail lines that are stroked but never filled, such as the
    /// side bars on a predefined process or the cross inside an `or`.
    pub decoration: Option<BezPath>,
    /// Whether `outline` encloses an area. Open shapes like a brace do not.
    pub filled: bool,
    /// Whether `outline` is stroked.
    ///
    /// A few types carry their whole visible form in `decoration` and use the
    /// outline only for bounds, hit testing, and text layout: a text block has
    /// no border, and a BPMN text annotation is a bracket beside its text
    /// rather than a box around it.
    pub stroked: bool,
    /// Padding between the bounding box and the text area, which narrows for
    /// shapes whose outline cuts into their corners.
    pub text_insets: Insets,
}

impl ShapeGeometry {
    pub(crate) fn closed(outline: BezPath) -> Self {
        ShapeGeometry {
            outline,
            decoration: None,
            filled: true,
            stroked: true,
            text_insets: Insets::uniform(4.0),
        }
    }

    pub(crate) fn with_decoration(mut self, decoration: BezPath) -> Self {
        self.decoration = Some(decoration);
        self
    }

    pub(crate) fn with_insets(mut self, insets: Insets) -> Self {
        self.text_insets = insets;
        self
    }

    /// Marks the outline as geometry only: present for bounds and hit testing,
    /// never painted.
    pub(crate) fn unpainted(mut self) -> Self {
        self.filled = false;
        self.stroked = false;
        self
    }

    pub(crate) fn open(outline: BezPath) -> Self {
        ShapeGeometry {
            outline,
            decoration: None,
            filled: false,
            stroked: true,
            text_insets: Insets::uniform(4.0),
        }
    }
}

/// Builds the geometry for a shape, including its rotation.
pub fn geometry(shape: &Shape) -> ShapeGeometry {
    let bb = &shape.bounding_box;
    let (x, y, w, h) = (bb.x, bb.y, bb.w.max(0.0), bb.h.max(0.0));
    let mut geo = outline_for(shape, x, y, w, h);

    let rotation = bb.rotation_degrees();
    if rotation != 0.0 {
        let center = bb.center();
        let affine = Affine::rotate_about(rotation.to_radians(), KPoint::new(center.x, center.y));
        geo.outline.apply_affine(affine);
        if let Some(d) = geo.decoration.as_mut() {
            d.apply_affine(affine);
        }
    }
    geo
}

fn outline_for(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    use ShapeType::*;
    match &shape.shape_type {
        // Standard library.
        Rectangle | Process => rounded_rect(shape, x, y, w, h),
        TextBlock => {
            // A text block has no outline of its own; the rect is kept for hit
            // testing and layout.
            ShapeGeometry::closed(rect_path(x, y, w, h)).unpainted()
        }
        Hotspot | Image => ShapeGeometry::closed(rect_path(x, y, w, h)),
        StickyNote => folded_corner(x, y, w, h, Fold::BottomRight),

        // Flowchart library.
        Terminator => stadium(x, y, w, h),
        Decision => diamond(x, y, w, h),
        Data => parallelogram(x, y, w, h),
        Document => document(x, y, w, h),
        MultipleDocuments => multiple_documents(x, y, w, h),
        PredefinedProcess => predefined_process(shape, x, y, w, h),
        InternalStorage => internal_storage(x, y, w, h),
        Preparation => hexagon(x, y, w, h),
        ManualInput => manual_input(x, y, w, h),
        ManualOperation => manual_operation(x, y, w, h),
        Display => display(x, y, w, h),
        Delay => delay(x, y, w, h),
        StoredData => stored_data(x, y, w, h),
        Database => database(x, y, w, h),
        DirectAccessStorage => direct_access_storage(x, y, w, h),
        PaperTape => paper_tape(x, y, w, h),
        Merge => triangle_down(x, y, w, h),
        Or => circle_with_cross(x, y, w, h),
        SummingJunction => circle_with_saltire(x, y, w, h),
        Connector => ellipse_path_geo(x, y, w, h),
        OffPageLink => off_page_link(x, y, w, h),
        Note => folded_corner(x, y, w, h, Fold::TopRight),
        BraceNote => brace(shape, x, y, w, h),

        // Shape library.
        Circle => ellipse_path_geo(x, y, w, h),
        Cloud => cloud(x, y, w, h),
        Cross => cross(shape, x, y, w, h),
        Diamond => diamond(x, y, w, h),
        DoubleArrow => arrow(x, y, w, h, ArrowKind::Double, "right"),
        FlexiblePolygon => from_vertices(&shape.resolved_vertices(), x, y, w, h),
        Hexagon => regular_polygon(6, x, y, w, h),
        IsoscelesTriangle => triangle_up(x, y, w, h),
        Octagon => regular_polygon(8, x, y, w, h),
        Pentagon => regular_polygon(5, x, y, w, h),
        PolyStar => star(shape, x, y, w, h),
        RightTriangle => right_triangle(x, y, w, h),
        SingleArrow => arrow(
            x,
            y,
            w,
            h,
            ArrowKind::Single,
            shape.orientation.as_deref().unwrap_or("right"),
        ),

        // Container library. A container is a frame its members sit inside, so
        // its label lives in a title bar rather than in the middle of the area.
        BraceContainer => brace(shape, x, y, w, h),
        BracketContainer => bracket(x, y, w, h),
        CircleContainer => titled(ellipse_path_geo(x, y, w, h), x, y, w, h, TitleBar::None),
        DiamondContainer => diamond(x, y, w, h),
        PillContainer => titled(stadium(x, y, w, h), x, y, w, h, TitleBar::Top),
        RectangleContainer => titled(
            ShapeGeometry::closed(rect_path(x, y, w, h)),
            x,
            y,
            w,
            h,
            TitleBar::Top,
        ),
        RoundedRectangleContainer => titled(
            ShapeGeometry::closed(rounded_path(x, y, w, h, (w.min(h) * 0.08).min(16.0))),
            x,
            y,
            w,
            h,
            TitleBar::Top,
        ),
        SwimLanes => swim_lanes(shape, x, y, w, h),

        // Lucidspark library.
        FreehandDrawing => freehand(shape, x, y, w, h),
        SparkCalloutSquare => callout(x, y, w, h),
        SparkContainer => titled(
            ShapeGeometry::closed(rounded_path(x, y, w, h, (w.min(h) * 0.06).min(14.0))),
            x,
            y,
            w,
            h,
            TitleBar::None,
        ),
        SparkFrame => titled(
            ShapeGeometry::closed(rect_path(x, y, w, h)),
            x,
            y,
            w,
            h,
            TitleBar::Top,
        ),

        // BPMN 2.0 library.
        BpmnActivity => bpmn_activity(shape, x, y, w, h),
        BpmnBlackBoxPool => bpmn_pool(shape, x, y, w, h, 0),
        BpmnChoreography => bpmn_choreography(x, y, w, h),
        BpmnConversation => bpmn_conversation(shape, x, y, w, h),
        BpmnDataObject => bpmn_data_object(shape, x, y, w, h),
        BpmnDataStore => database(x, y, w, h),
        BpmnEvent => bpmn_event(shape, x, y, w, h),
        BpmnGateway => bpmn_gateway(shape, x, y, w, h),
        BpmnGroup => titled(
            ShapeGeometry::closed(rounded_path(x, y, w, h, (w.min(h) * 0.06).min(14.0))),
            x,
            y,
            w,
            h,
            TitleBar::Top,
        ),
        BpmnPool => bpmn_pool(shape, x, y, w, h, 3),
        BpmnTextAnnotation => bpmn_text_annotation(x, y, w, h),

        // Table library.
        Table => table(shape, x, y, w, h),

        // AWS, Azure, and GCP. The published libraries ship their icons as
        // artwork this build does not have, so a named shape draws as a tile
        // carrying its vendor mark and its text, and a named container as a
        // titled frame. Both stay selectable, movable, and lossless on save.
        NamedShape => named_shape(shape, x, y, w, h),
        NamedContainer => titled(
            ShapeGeometry::closed(rounded_path(x, y, w, h, (w.min(h) * 0.05).min(12.0))),
            x,
            y,
            w,
            h,
            TitleBar::Top,
        ),

        // The notations the published format does not cover, drawn from
        // their own definitions in `extended`.
        UmlClass => extended::uml_class(x, y, w, h),
        UmlInterface => extended::uml_interface(x, y, w, h),
        UmlPackage => extended::uml_package(x, y, w, h),
        UmlActor => extended::uml_actor(x, y, w, h),
        UmlUseCase => extended::uml_use_case(x, y, w, h),
        UmlLifeline => extended::uml_lifeline(x, y, w, h),
        UmlActivation => extended::uml_activation(x, y, w, h),
        UmlComponent => extended::uml_component(x, y, w, h),
        UmlNode => extended::uml_node(x, y, w, h),
        UmlState => extended::uml_state(x, y, w, h),
        UmlInitialState => extended::uml_initial_state(x, y, w, h),
        UmlFinalState => extended::uml_final_state(x, y, w, h),
        UmlFork => extended::uml_fork(x, y, w, h),

        ErdEntity => extended::erd_entity(x, y, w, h),
        ErdWeakEntity => extended::erd_weak_entity(x, y, w, h),
        ErdRelationship => extended::erd_relationship(x, y, w, h),
        ErdWeakRelationship => extended::erd_weak_relationship(x, y, w, h),
        ErdAttribute => extended::erd_attribute(x, y, w, h),
        ErdMultivaluedAttribute => extended::erd_multivalued_attribute(x, y, w, h),
        ErdDerivedAttribute => extended::erd_derived_attribute(x, y, w, h),

        NetServer => extended::net_server(x, y, w, h),
        NetRouter => extended::net_router(x, y, w, h),
        NetSwitch => extended::net_switch(x, y, w, h),
        NetFirewall => extended::net_firewall(x, y, w, h),
        NetWorkstation => extended::net_workstation(x, y, w, h),
        NetLaptop => extended::net_laptop(x, y, w, h),
        NetMobile => extended::net_mobile(x, y, w, h),
        NetPrinter => extended::net_printer(x, y, w, h),
        NetRack => extended::net_rack(x, y, w, h),
        NetWireless => extended::net_wireless(x, y, w, h),

        WireBrowser => extended::wire_browser(x, y, w, h),
        WirePhone => extended::wire_phone(x, y, w, h),
        WireButton => extended::wire_button(x, y, w, h),
        WireTextField => extended::wire_text_field(x, y, w, h),
        WireCheckbox => extended::wire_checkbox(x, y, w, h),
        WireRadio => extended::wire_radio(x, y, w, h),
        WireDropdown => extended::wire_dropdown(x, y, w, h),
        WireImage => extended::wire_image(x, y, w, h),
        WireNavBar => extended::wire_nav_bar(x, y, w, h),
        WireSlider => extended::wire_slider(x, y, w, h),
        WireToggle => extended::wire_toggle(x, y, w, h),

        ChartVennTwo => extended::chart_venn_two(x, y, w, h),
        ChartVennThree => extended::chart_venn_three(x, y, w, h),
        ChartMatrix => extended::chart_matrix(x, y, w, h),
        ChartSitemapPage => extended::chart_sitemap_page(x, y, w, h),
        ChartTimeline => extended::chart_timeline(x, y, w, h),
        ChartMilestone => extended::chart_milestone(x, y, w, h),

        // A type string no published library declares: draw a plain rectangle
        // so the shape is still visible, selectable, and saveable.
        Other(_) => ShapeGeometry::closed(rect_path(x, y, w, h)),
    }
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

pub(crate) fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    Rect::new(x, y, x + w, y + h).to_path(PATH_TOLERANCE)
}

pub(crate) fn rounded_path(x: f64, y: f64, w: f64, h: f64, radius: f64) -> BezPath {
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    if r <= 0.0 {
        return rect_path(x, y, w, h);
    }
    RoundedRect::new(x, y, x + w, y + h, r).to_path(PATH_TOLERANCE)
}

pub(crate) fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut p = BezPath::new();
    if let Some(&(x0, y0)) = points.first() {
        p.move_to((x0, y0));
        for &(px, py) in &points[1..] {
            p.line_to((px, py));
        }
        p.close_path();
    }
    p
}

/// Appends a half-ellipse from `from` to `to`, bulging perpendicular by
/// `(bulge_x, bulge_y)` at its midpoint. Built from two quarter-ellipse cubics.
fn half_ellipse(path: &mut BezPath, from: (f64, f64), to: (f64, f64), bulge_x: f64, bulge_y: f64) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let (mx, my) = ((x0 + x1) / 2.0 + bulge_x, (y0 + y1) / 2.0 + bulge_y);
    // Half-extent along the chord, used to place the tangent controls.
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);

    path.curve_to(
        (x0 + bulge_x * KAPPA, y0 + bulge_y * KAPPA),
        (mx - hx * KAPPA, my - hy * KAPPA),
        (mx, my),
    );
    path.curve_to(
        (mx + hx * KAPPA, my + hy * KAPPA),
        (x1 + bulge_x * KAPPA, y1 + bulge_y * KAPPA),
        (x1, y1),
    );
}

/// Appends a single-cycle S-wave from `from` to `to` along x, with vertical
/// amplitude `amp`. Used for the torn edges on documents and paper tape.
///
/// A symmetric cubic reaches only three quarters of its control offset, so the
/// controls are scaled by 4/3 to make the curve peak at exactly `amp`. Without
/// that correction the wave overshoots its shape's bounding box.
fn wave_x(path: &mut BezPath, from: (f64, f64), to: (f64, f64), amp: f64) {
    const PEAK_CORRECTION: f64 = 4.0 / 3.0;
    let reach = amp * PEAK_CORRECTION;

    let (x0, y0) = from;
    let (x1, y1) = to;
    let third = (x1 - x0) / 3.0;
    let mid = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);

    path.curve_to(
        (x0 + third * 0.6, y0 + reach),
        (mid.0 - third * 0.5, mid.1 + reach),
        mid,
    );
    path.curve_to(
        (mid.0 + third * 0.5, mid.1 - reach),
        (x1 - third * 0.6, y1 - reach),
        (x1, y1),
    );
}

pub(crate) fn line(path: &mut BezPath, from: (f64, f64), to: (f64, f64)) {
    path.move_to(from);
    path.line_to(to);
}

// ---------------------------------------------------------------------------
// Shape builders
// ---------------------------------------------------------------------------

fn rounded_rect(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let radius = shape
        .style
        .as_ref()
        .map(|s| s.corner_radius_px())
        .unwrap_or(0.0)
        // A radius beyond half the shorter side would self-intersect.
        .min(w.min(h) / 2.0)
        .max(0.0);

    let path = if radius > 0.0 {
        RoundedRect::new(x, y, x + w, y + h, radius).to_path(PATH_TOLERANCE)
    } else {
        rect_path(x, y, w, h)
    };
    ShapeGeometry::closed(path)
}

pub(crate) fn ellipse_path_geo(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let e = Ellipse::new(
        KPoint::new(x + w / 2.0, y + h / 2.0),
        (w / 2.0, h / 2.0),
        0.0,
    );
    // Text inside an ellipse must clear the curve, so inset by the gap between
    // the ellipse and its box at 45 degrees.
    let gap = 1.0 - std::f64::consts::FRAC_1_SQRT_2;
    ShapeGeometry::closed(e.to_path(PATH_TOLERANCE))
        .with_insets(Insets::new(w * gap, h * gap, w * gap, h * gap))
}

fn stadium(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (h / 2.0).min(w / 2.0);
    let path = RoundedRect::new(x, y, x + w, y + h, r).to_path(PATH_TOLERANCE);
    ShapeGeometry::closed(path).with_insets(Insets::new(r * 0.7, 4.0, r * 0.7, 4.0))
}

fn diamond(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let path = polygon(&[
        (x + w / 2.0, y),
        (x + w, y + h / 2.0),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
    ]);
    // Only the middle quarter of a diamond reliably holds text.
    ShapeGeometry::closed(path).with_insets(Insets::new(w * 0.25, h * 0.25, w * 0.25, h * 0.25))
}

fn parallelogram(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let skew = w * 0.2;
    let path = polygon(&[
        (x + skew, y),
        (x + w, y),
        (x + w - skew, y + h),
        (x, y + h),
    ]);
    ShapeGeometry::closed(path).with_insets(Insets::new(skew, 4.0, skew, 4.0))
}

fn hexagon(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let notch = w * 0.15;
    let path = polygon(&[
        (x + notch, y),
        (x + w - notch, y),
        (x + w, y + h / 2.0),
        (x + w - notch, y + h),
        (x + notch, y + h),
        (x, y + h / 2.0),
    ]);
    ShapeGeometry::closed(path).with_insets(Insets::new(notch, 4.0, notch, 4.0))
}

fn triangle_down(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let path = polygon(&[(x, y), (x + w, y), (x + w / 2.0, y + h)]);
    ShapeGeometry::closed(path).with_insets(Insets::new(w * 0.2, 4.0, w * 0.2, h * 0.45))
}

fn manual_input(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    // The top edge rises left to right.
    let path = polygon(&[(x, y + h * 0.25), (x + w, y), (x + w, y + h), (x, y + h)]);
    ShapeGeometry::closed(path).with_insets(Insets::new(4.0, h * 0.25, 4.0, 4.0))
}

fn manual_operation(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let taper = w * 0.15;
    let path = polygon(&[
        (x, y),
        (x + w, y),
        (x + w - taper, y + h),
        (x + taper, y + h),
    ]);
    ShapeGeometry::closed(path).with_insets(Insets::new(taper, 4.0, taper, 4.0))
}

fn off_page_link(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let shoulder = y + h * 0.6;
    let path = polygon(&[
        (x, y),
        (x + w, y),
        (x + w, shoulder),
        (x + w / 2.0, y + h),
        (x, shoulder),
    ]);
    ShapeGeometry::closed(path).with_insets(Insets::new(4.0, 4.0, 4.0, h * 0.4))
}

fn delay(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (h / 2.0).min(w);
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w - r, y));
    half_ellipse(&mut p, (x + w - r, y), (x + w - r, y + h), r, 0.0);
    p.line_to((x, y + h));
    p.close_path();
    ShapeGeometry::closed(p).with_insets(Insets::new(4.0, 4.0, r * 0.6, 4.0))
}

fn display(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let point = w * 0.15;
    let cap = w * 0.15;
    let mut p = BezPath::new();
    p.move_to((x + point, y));
    p.line_to((x + w - cap, y));
    half_ellipse(&mut p, (x + w - cap, y), (x + w - cap, y + h), cap, 0.0);
    p.line_to((x + point, y + h));
    p.line_to((x, y + h / 2.0));
    p.close_path();
    ShapeGeometry::closed(p).with_insets(Insets::new(point, 4.0, cap, 4.0))
}

fn stored_data(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w * 0.12).min(w / 3.0);
    let mut p = BezPath::new();
    // The arcs are placed so the outline touches the box edges exactly: the
    // right arc crests at x + w, and the left arc curves inward from x.
    p.move_to((x, y));
    p.line_to((x + w - r, y));
    half_ellipse(&mut p, (x + w - r, y), (x + w - r, y + h), r, 0.0);
    p.line_to((x, y + h));
    // The left edge curves right, which is what gives the symbol its profile.
    half_ellipse(&mut p, (x, y + h), (x, y), r, 0.0);
    p.close_path();
    ShapeGeometry::closed(p).with_insets(Insets::new(r * 1.6, 4.0, r * 1.2, 4.0))
}

fn database(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let ry = (h * 0.12).min(h / 4.0);
    let mut p = BezPath::new();
    p.move_to((x, y + ry));
    // Top cap, bulging up.
    half_ellipse(&mut p, (x, y + ry), (x + w, y + ry), 0.0, -ry);
    p.line_to((x + w, y + h - ry));
    // Bottom cap, bulging down.
    half_ellipse(&mut p, (x + w, y + h - ry), (x, y + h - ry), 0.0, ry);
    p.close_path();

    // The near side of the top cap, which makes the cylinder read as 3D.
    let mut d = BezPath::new();
    d.move_to((x, y + ry));
    half_ellipse(&mut d, (x, y + ry), (x + w, y + ry), 0.0, ry);

    ShapeGeometry::closed(p)
        .with_decoration(d)
        .with_insets(Insets::new(4.0, ry * 2.2, 4.0, ry * 1.2))
}

fn direct_access_storage(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let rx = (w * 0.12).min(w / 4.0);
    let mut p = BezPath::new();
    // Both caps crest on the box edges, giving a capsule that fills its box.
    p.move_to((x + rx, y));
    p.line_to((x + w - rx, y));
    half_ellipse(&mut p, (x + w - rx, y), (x + w - rx, y + h), rx, 0.0);
    p.line_to((x + rx, y + h));
    half_ellipse(&mut p, (x + rx, y + h), (x + rx, y), -rx, 0.0);
    p.close_path();

    // The far rim of the left cap, which makes the tube read as 3D.
    let mut d = BezPath::new();
    d.move_to((x + rx, y));
    half_ellipse(&mut d, (x + rx, y), (x + rx, y + h), rx, 0.0);

    ShapeGeometry::closed(p)
        .with_decoration(d)
        .with_insets(Insets::new(rx * 2.2, 4.0, rx * 1.2, 4.0))
}

fn document(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let amp = (h * 0.08).min(12.0);
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h - amp));
    wave_x(&mut p, (x + w, y + h - amp), (x, y + h - amp), -amp);
    p.close_path();
    ShapeGeometry::closed(p).with_insets(Insets::new(4.0, 4.0, 4.0, amp * 2.0))
}

fn multiple_documents(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let off = (w.min(h) * 0.1).min(10.0);
    // The front document sits lower-left; two more peek out behind it.
    let front = document(x, y + off * 2.0, w - off * 2.0, h - off * 2.0);

    let mut d = BezPath::new();
    for i in [1.0_f64, 2.0] {
        let back = document(x + off * i, y + off * (2.0 - i), w - off * 2.0, h - off * 2.0);
        d.extend(back.outline.iter());
    }

    ShapeGeometry::closed(front.outline)
        .with_decoration(d)
        .with_insets(Insets::new(4.0, off * 2.0 + 4.0, off * 2.0 + 4.0, off * 2.0))
}

fn paper_tape(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let amp = (h * 0.1).min(12.0);
    let mut p = BezPath::new();
    p.move_to((x, y + amp));
    wave_x(&mut p, (x, y + amp), (x + w, y + amp), -amp);
    p.line_to((x + w, y + h - amp));
    wave_x(&mut p, (x + w, y + h - amp), (x, y + h - amp), amp);
    p.close_path();
    ShapeGeometry::closed(p).with_insets(Insets::new(4.0, amp * 2.0, 4.0, amp * 2.0))
}

fn predefined_process(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let inset = w * shape.resolved_side_width();
    let mut d = BezPath::new();
    line(&mut d, (x + inset, y), (x + inset, y + h));
    line(&mut d, (x + w - inset, y), (x + w - inset, y + h));

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(d)
        .with_insets(Insets::new(inset + 4.0, 4.0, inset + 4.0, 4.0))
}

fn internal_storage(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let bar_x = x + (w * 0.18).min(24.0);
    let bar_y = y + (h * 0.22).min(24.0);
    let mut d = BezPath::new();
    line(&mut d, (x, bar_y), (x + w, bar_y));
    line(&mut d, (bar_x, y), (bar_x, y + h));

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(d)
        .with_insets(Insets::new(bar_x - x + 4.0, bar_y - y + 4.0, 4.0, 4.0))
}

fn circle_with_cross(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut d = BezPath::new();
    line(&mut d, (x + w / 2.0, y), (x + w / 2.0, y + h));
    line(&mut d, (x, y + h / 2.0), (x + w, y + h / 2.0));
    ellipse_path_geo(x, y, w, h).with_decoration(d)
}

fn circle_with_saltire(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    // The diagonals meet the circle at 45 degrees, so they stop short of the box.
    let k = (1.0 - std::f64::consts::FRAC_1_SQRT_2) / 2.0;
    let (ix, iy) = (w * k, h * k);
    let mut d = BezPath::new();
    line(&mut d, (x + ix, y + iy), (x + w - ix, y + h - iy));
    line(&mut d, (x + w - ix, y + iy), (x + ix, y + h - iy));
    ellipse_path_geo(x, y, w, h).with_decoration(d)
}

enum Fold {
    TopRight,
    BottomRight,
}

fn folded_corner(x: f64, y: f64, w: f64, h: f64, corner: Fold) -> ShapeGeometry {
    let fold = (w.min(h) * 0.25).min(28.0);
    let (outline, decoration, insets) = match corner {
        Fold::TopRight => {
            let o = polygon(&[
                (x, y),
                (x + w - fold, y),
                (x + w, y + fold),
                (x + w, y + h),
                (x, y + h),
            ]);
            let mut d = BezPath::new();
            d.move_to((x + w - fold, y));
            d.line_to((x + w - fold, y + fold));
            d.line_to((x + w, y + fold));
            (o, d, Insets::new(4.0, fold, 4.0, 4.0))
        }
        Fold::BottomRight => {
            let o = polygon(&[
                (x, y),
                (x + w, y),
                (x + w, y + h - fold),
                (x + w - fold, y + h),
                (x, y + h),
            ]);
            let mut d = BezPath::new();
            d.move_to((x + w - fold, y + h));
            d.line_to((x + w - fold, y + h - fold));
            d.line_to((x + w, y + h - fold));
            (o, d, Insets::new(4.0, 4.0, 4.0, fold))
        }
    };
    ShapeGeometry::closed(outline).with_decoration(decoration).with_insets(insets)
}

fn brace(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let bw = shape.brace_width.unwrap_or(w * 0.6).clamp(1.0, w.max(1.0));
    let right_facing = shape.right_facing.unwrap_or(false);
    let (mid_y, q1, q3) = (y + h / 2.0, y + h * 0.25, y + h * 0.75);

    // Built facing left (a `{`), then mirrored for the right-facing variant.
    let tip = x + bw;
    let cusp = x;
    let mut p = BezPath::new();
    p.move_to((tip, y));
    p.curve_to((tip - bw * 0.5, y), (cusp + bw * 0.5, q1 - h * 0.1), (cusp + bw * 0.5, q1));
    p.line_to((cusp + bw * 0.5, mid_y - h * 0.04));
    p.curve_to(
        (cusp + bw * 0.5, mid_y),
        (cusp, mid_y - h * 0.02),
        (cusp, mid_y),
    );
    p.curve_to(
        (cusp, mid_y + h * 0.02),
        (cusp + bw * 0.5, mid_y),
        (cusp + bw * 0.5, mid_y + h * 0.04),
    );
    p.line_to((cusp + bw * 0.5, q3));
    p.curve_to((cusp + bw * 0.5, q3 + h * 0.1), (tip - bw * 0.5, y + h), (tip, y + h));

    if right_facing {
        // Mirror about the box's vertical center line.
        let cx = x + w / 2.0;
        p.apply_affine(Affine::translate((cx, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-cx, 0.0)));
    }

    let insets = if right_facing {
        Insets::new(4.0, 4.0, bw + 4.0, 4.0)
    } else {
        Insets::new(bw + 4.0, 4.0, 4.0, 4.0)
    };
    ShapeGeometry::open(p).with_insets(insets)
}

// ---------------------------------------------------------------------------
// Shape library
// ---------------------------------------------------------------------------

/// A regular polygon inscribed in the box, starting at the top and running
/// clockwise. Stretching the inscribed circle to the box means a hexagon in a
/// wide box is a wide hexagon, which is what a diagramming tool should do.
fn regular_polygon(sides: u32, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let vertices = dc_model::shape::regular_polygon_vertices(sides);
    let geo = from_vertices(&vertices, x, y, w, h);
    // The inscribed radius of a regular polygon is `cos(pi/n)` of the
    // circumscribed one, so text must clear that much of the box.
    let inset = 0.5 * (1.0 - (std::f64::consts::PI / sides.max(3) as f64).cos());
    geo.with_insets(Insets::new(w * inset, h * inset, w * inset, h * inset))
}

/// A closed polygon from relative vertices.
fn from_vertices(vertices: &[RelPoint], x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let points: Vec<(f64, f64)> = vertices
        .iter()
        .map(|v| (x + w * v.x, y + h * v.y))
        .collect();
    // A degenerate vertex list would produce an empty path, which nothing
    // downstream can draw or hit test.
    if points.len() < 3 {
        return ShapeGeometry::closed(rect_path(x, y, w, h));
    }
    // Only the middle of an arbitrary polygon reliably holds text.
    ShapeGeometry::closed(polygon(&points))
        .with_insets(Insets::new(w * 0.18, h * 0.18, w * 0.18, h * 0.18))
}

fn triangle_up(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let path = polygon(&[(x + w / 2.0, y), (x + w, y + h), (x, y + h)]);
    ShapeGeometry::closed(path).with_insets(Insets::new(w * 0.2, h * 0.45, w * 0.2, 4.0))
}

fn right_triangle(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    // The right angle sits at the bottom left, the conventional orientation.
    let path = polygon(&[(x, y), (x + w, y + h), (x, y + h)]);
    ShapeGeometry::closed(path).with_insets(Insets::new(4.0, h * 0.45, w * 0.45, 4.0))
}

fn cross(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let i = shape.resolved_indent();
    let (ix, iy) = (w * i.x, h * i.y);
    let path = polygon(&[
        (x + ix, y),
        (x + w - ix, y),
        (x + w - ix, y + iy),
        (x + w, y + iy),
        (x + w, y + h - iy),
        (x + w - ix, y + h - iy),
        (x + w - ix, y + h),
        (x + ix, y + h),
        (x + ix, y + h - iy),
        (x, y + h - iy),
        (x, y + iy),
        (x + ix, y + iy),
    ]);
    // The centre square is the only region both arms cover.
    ShapeGeometry::closed(path).with_insets(Insets::new(ix, iy, ix, iy))
}

fn star(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let spec = shape.resolved_star();
    let points = spec.resolved_points();
    let inner = spec.resolved_inner_radius();
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (rx, ry) = (w / 2.0, h / 2.0);

    let mut verts = Vec::with_capacity(points as usize * 2);
    for i in 0..points * 2 {
        let radius = if i % 2 == 0 { 1.0 } else { inner };
        let angle = -std::f64::consts::FRAC_PI_2
            + i as f64 * std::f64::consts::PI / points as f64;
        verts.push((cx + rx * radius * angle.cos(), cy + ry * radius * angle.sin()));
    }

    // Text has to fit inside the waist of the star, not its points.
    let pad = (1.0 - inner.max(0.25)) / 2.0;
    ShapeGeometry::closed(polygon(&verts))
        .with_insets(Insets::new(w * pad, h * pad, w * pad, h * pad))
}

fn cloud(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    // Five lobes around the box, each a half ellipse bulging outward. The
    // chords meet on an inset rectangle so the lobes crest exactly on the box.
    let (bx, by) = (w * 0.18, h * 0.22);
    let (l, t, r, b) = (x + bx, y + by, x + w - bx, y + h - by);

    let mut p = BezPath::new();
    p.move_to((l, t));
    // Top: two lobes.
    half_ellipse(&mut p, (l, t), ((l + r) / 2.0, t), 0.0, -by);
    half_ellipse(&mut p, ((l + r) / 2.0, t), (r, t), 0.0, -by * 0.8);
    // Right.
    half_ellipse(&mut p, (r, t), (r, b), bx, 0.0);
    // Bottom: two lobes, running right to left.
    half_ellipse(&mut p, (r, b), ((l + r) / 2.0, b), 0.0, by * 0.8);
    half_ellipse(&mut p, ((l + r) / 2.0, b), (l, b), 0.0, by);
    // Left.
    half_ellipse(&mut p, (l, b), (l, t), -bx, 0.0);
    p.close_path();

    ShapeGeometry::closed(p).with_insets(Insets::new(bx, by, bx, by))
}

enum ArrowKind {
    Single,
    Double,
}

/// A block arrow. `orientation` is one of the published `right`, `left`, `up`,
/// `down`; anything else points right.
fn arrow(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    kind: ArrowKind,
    orientation: &str,
) -> ShapeGeometry {
    // Built pointing right in a unit box, then mapped onto the real one.
    let head = match kind {
        ArrowKind::Single => 0.35,
        ArrowKind::Double => 0.3,
    };
    let shaft = 0.3; // half-height of the shaft

    let unit: Vec<(f64, f64)> = match kind {
        ArrowKind::Single => vec![
            (0.0, 0.5 - shaft),
            (1.0 - head, 0.5 - shaft),
            (1.0 - head, 0.0),
            (1.0, 0.5),
            (1.0 - head, 1.0),
            (1.0 - head, 0.5 + shaft),
            (0.0, 0.5 + shaft),
        ],
        ArrowKind::Double => vec![
            (0.0, 0.5),
            (head, 0.0),
            (head, 0.5 - shaft),
            (1.0 - head, 0.5 - shaft),
            (1.0 - head, 0.0),
            (1.0, 0.5),
            (1.0 - head, 1.0),
            (1.0 - head, 0.5 + shaft),
            (head, 0.5 + shaft),
            (head, 1.0),
        ],
    };

    // Reorienting rotates the unit square, so the arrow still fills its box
    // rather than being rotated about the centre and poking out of it.
    let placed: Vec<(f64, f64)> = unit
        .iter()
        .map(|&(u, v)| {
            let (a, b) = match orientation {
                "left" => (1.0 - u, 1.0 - v),
                "up" => (v, 1.0 - u),
                "down" => (1.0 - v, u),
                _ => (u, v),
            };
            (x + w * a, y + h * b)
        })
        .collect();

    let along = 1.0 - head;
    let across = shaft * 2.0;
    let (ix0, iy0, ix1, iy1) = match orientation {
        "left" => (w * head, h * (0.5 - shaft), 0.0, h * (0.5 - shaft)),
        "up" => (w * (0.5 - shaft), h * head, w * (0.5 - shaft), 0.0),
        "down" => (w * (0.5 - shaft), 0.0, w * (0.5 - shaft), h * head),
        _ => (0.0, h * (0.5 - shaft), w * (1.0 - along), h * (0.5 - shaft)),
    };
    let _ = across;

    ShapeGeometry::closed(polygon(&placed)).with_insets(Insets::new(ix0, iy0, ix1, iy1))
}

// ---------------------------------------------------------------------------
// Container library
// ---------------------------------------------------------------------------

/// Where a container puts its label.
enum TitleBar {
    /// A bar across the top, separated from the body by a rule.
    Top,
    /// No bar; the label sits in the body.
    None,
}

/// Height of a container's title bar, in document units.
fn title_bar_height(h: f64) -> f64 {
    (h * 0.12).clamp(14.0, 32.0).min(h)
}

/// Adds a title bar to a container outline and moves its text into the bar.
fn titled(
    mut geo: ShapeGeometry,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    bar: TitleBar,
) -> ShapeGeometry {
    match bar {
        TitleBar::None => {
            // The label sits in a band at the top of the body, leaving the
            // middle clear for whatever the container holds.
            let band = (h * 0.16).clamp(16.0, 36.0).min(h);
            geo.with_insets(Insets::new(w * 0.12, 6.0, w * 0.12, (h - band).max(0.0)))
        }
        TitleBar::Top => {
            let bar_h = title_bar_height(h);
            let mut d = geo.decoration.take().unwrap_or_default();
            line(&mut d, (x, y + bar_h), (x + w, y + bar_h));
            geo.with_decoration(d)
                .with_insets(Insets::new(6.0, 2.0, 6.0, (h - bar_h + 2.0).max(0.0)))
        }
    }
}

/// A square bracket pair, `[ ]`, enclosing its members on two sides.
fn bracket(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let arm = w * 0.3;
    // Both arms belong to one stroke: they are a single mark, and splitting
    // them would leave the outline covering only one side of the box.
    let mut p = BezPath::new();
    p.move_to((x + arm, y));
    p.line_to((x, y));
    p.line_to((x, y + h));
    p.line_to((x + arm, y + h));
    p.move_to((x + w - arm, y));
    p.line_to((x + w, y));
    p.line_to((x + w, y + h));
    p.line_to((x + w - arm, y + h));

    ShapeGeometry::open(p).with_insets(Insets::new(arm + 2.0, 4.0, arm + 2.0, 4.0))
}

fn swim_lanes(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let lane_count = shape
        .extra
        .get("lanes")
        .and_then(|v| v.as_array())
        .map(|a| a.len().clamp(1, 64))
        .unwrap_or(3);

    let bar = title_bar_height(h);
    let mut d = BezPath::new();

    if shape.is_vertical() {
        // Vertical lanes: a title bar across the top, columns beneath it.
        line(&mut d, (x, y + bar), (x + w, y + bar));
        for i in 1..lane_count {
            let lx = x + w * i as f64 / lane_count as f64;
            line(&mut d, (lx, y + bar), (lx, y + h));
        }
        ShapeGeometry::closed(rect_path(x, y, w, h))
            .with_decoration(d)
            .with_insets(Insets::new(6.0, 2.0, 6.0, (h - bar + 2.0).max(0.0)))
    } else {
        // Horizontal lanes: a title band down the left, rows beside it.
        let band = (w * 0.08).clamp(14.0, 40.0).min(w);
        line(&mut d, (x + band, y), (x + band, y + h));
        for i in 1..lane_count {
            let ly = y + h * i as f64 / lane_count as f64;
            line(&mut d, (x + band, ly), (x + w, ly));
        }
        ShapeGeometry::closed(rect_path(x, y, w, h))
            .with_decoration(d)
            .with_insets(Insets::new(2.0, 4.0, (w - band + 2.0).max(0.0), 4.0))
    }
}

// ---------------------------------------------------------------------------
// Lucidspark library
// ---------------------------------------------------------------------------

/// A freehand stroke, drawn from its recorded points.
///
/// The points are in the same space as the shape's offset, so the path is
/// normalized into the bounding box: that way moving or resizing the shape
/// carries the stroke with it, as every other type does.
fn freehand(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let points = shape.points.as_deref().unwrap_or(&[]);
    if points.len() < 2 {
        // Nothing recorded: a diagonal stands in so the shape is visible and
        // can be selected rather than vanishing.
        let mut p = BezPath::new();
        p.move_to((x, y + h));
        p.line_to((x + w, y));
        return ShapeGeometry::open(p);
    }

    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for p in points {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let span_x = (max_x - min_x).max(1e-6);
    let span_y = (max_y - min_y).max(1e-6);

    let mut path = BezPath::new();
    for (i, pt) in points.iter().enumerate() {
        let px = x + w * (pt.x - min_x) / span_x;
        let py = y + h * (pt.y - min_y) / span_y;
        if i == 0 {
            path.move_to((px, py));
        } else {
            path.line_to((px, py));
        }
    }
    ShapeGeometry::open(path)
}

/// A rounded speech box with a tail at the bottom left.
fn callout(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let tail = (h * 0.18).min(h / 3.0);
    let body_h = h - tail;
    let r = (w.min(body_h) * 0.1).min(14.0);

    let mut p = rounded_path(x, y, w, body_h, r);
    // The tail hangs from the body, inside the box, so the shape still fits.
    let foot_x = x + w * 0.22;
    p.move_to((foot_x, y + body_h));
    p.line_to((foot_x + w * 0.06, y + h));
    p.line_to((foot_x + w * 0.2, y + body_h));
    p.close_path();

    ShapeGeometry::closed(p).with_insets(Insets::new(8.0, 6.0, 8.0, tail + 6.0))
}

// ---------------------------------------------------------------------------
// BPMN 2.0 library
// ---------------------------------------------------------------------------

/// An activity: a rounded rectangle, with the border the activity type calls
/// for and the task icon and markers as interior detail.
fn bpmn_activity(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w.min(h) * 0.12).min(14.0);
    let outline = rounded_path(x, y, w, h, r);
    let mut d = BezPath::new();

    match shape.activity_type.as_deref() {
        // A transaction is drawn as a double border.
        Some("transaction") => {
            let gap = (w.min(h) * 0.05).min(5.0);
            d.extend(
                rounded_path(x + gap, y + gap, w - gap * 2.0, h - gap * 2.0, (r - gap).max(0.0))
                    .iter(),
            );
        }
        // A call activity carries a thick border, approximated by a second
        // outline just inside the first.
        Some("callActivity") => {
            let gap = (w.min(h) * 0.025).min(3.0);
            d.extend(
                rounded_path(x + gap, y + gap, w - gap * 2.0, h - gap * 2.0, (r - gap).max(0.0))
                    .iter(),
            );
        }
        _ => {}
    }

    // Task-type icon, top left.
    if shape.task_type.is_some() {
        let s = (w.min(h) * 0.16).min(14.0);
        d.extend(rect_path(x + 5.0, y + 5.0, s, s).iter());
    }

    // Loop and sub-process markers, bottom centre.
    let markers: Vec<&str> = [
        shape.extra.get("activityMarker1").and_then(|v| v.as_str()),
        shape.extra.get("activityMarker2").and_then(|v| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter(|m| *m != "none")
    .collect();

    if !markers.is_empty() {
        let s = (w.min(h) * 0.14).min(12.0);
        let total = markers.len() as f64 * (s + 4.0) - 4.0;
        let mut mx = x + (w - total) / 2.0;
        let my = y + h - s - 4.0;
        for _ in &markers {
            d.extend(rect_path(mx, my, s, s).iter());
            mx += s + 4.0;
        }
    }

    let bottom = if markers.is_empty() { 6.0 } else { (w.min(h) * 0.14).min(12.0) + 8.0 };
    let geo = ShapeGeometry::closed(outline).with_insets(Insets::new(8.0, 6.0, 8.0, bottom));
    if d.is_empty() { geo } else { geo.with_decoration(d) }
}

/// An event: a circle whose border weight says whether it starts, continues, or
/// ends a flow, with its trigger drawn inside.
fn bpmn_event(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut d = BezPath::new();
    let group = shape.event_group.as_deref().unwrap_or("start");

    // An intermediate event is a double ring; an end event a heavy one, which
    // a second ring just inside approximates without a second stroke width.
    let gap = match group {
        "intermediate" => w.min(h) * 0.09,
        "end" => w.min(h) * 0.04,
        _ => 0.0,
    };
    if gap > 0.0 {
        let e = Ellipse::new(
            KPoint::new(x + w / 2.0, y + h / 2.0),
            (w / 2.0 - gap, h / 2.0 - gap),
            0.0,
        );
        d.extend(e.to_path(PATH_TOLERANCE).iter());
    }

    // The trigger glyph, centred. Every published trigger is a small mark in
    // the middle of the ring, so one generic glyph per family is enough to
    // tell a plain event from a triggered one.
    if shape
        .event_type
        .as_deref()
        .is_some_and(|t| !t.is_empty() && t != "none")
    {
        let s = w.min(h) * 0.22;
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        match shape.event_type.as_deref().unwrap_or("") {
            "timer" => {
                let e = Ellipse::new(KPoint::new(cx, cy), (s, s), 0.0);
                d.extend(e.to_path(PATH_TOLERANCE).iter());
                line(&mut d, (cx, cy), (cx, cy - s * 0.7));
                line(&mut d, (cx, cy), (cx + s * 0.5, cy));
            }
            "message" => {
                d.extend(rect_path(cx - s, cy - s * 0.7, s * 2.0, s * 1.4).iter());
                line(&mut d, (cx - s, cy - s * 0.7), (cx, cy + s * 0.1));
                line(&mut d, (cx, cy + s * 0.1), (cx + s, cy - s * 0.7));
            }
            "error" => {
                d.move_to((cx - s, cy + s));
                d.line_to((cx - s * 0.2, cy - s * 0.3));
                d.line_to((cx + s * 0.2, cy + s * 0.3));
                d.line_to((cx + s, cy - s));
            }
            _ => {
                d.move_to((cx, cy - s));
                d.line_to((cx + s, cy + s * 0.6));
                d.line_to((cx - s, cy + s * 0.6));
                d.close_path();
            }
        }
    }

    // An event's label sits beneath the circle, not inside it.
    let geo = ellipse_path_geo(x, y, w, h).with_insets(Insets::new(2.0, h * 0.8, 2.0, 0.0));
    if d.is_empty() { geo } else { geo.with_decoration(d) }
}

/// A gateway: a diamond with the marker its type calls for.
fn bpmn_gateway(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let s = w.min(h) * 0.18;
    let mut d = BezPath::new();

    match shape.gateway_type.as_deref().unwrap_or("none") {
        "exclusive" => {
            line(&mut d, (cx - s, cy - s), (cx + s, cy + s));
            line(&mut d, (cx + s, cy - s), (cx - s, cy + s));
        }
        "parallel" | "parallelEventBased" => {
            line(&mut d, (cx, cy - s * 1.3), (cx, cy + s * 1.3));
            line(&mut d, (cx - s * 1.3, cy), (cx + s * 1.3, cy));
        }
        "inclusive" => {
            let e = Ellipse::new(KPoint::new(cx, cy), (s * 1.2, s * 1.2), 0.0);
            d.extend(e.to_path(PATH_TOLERANCE).iter());
        }
        "eventBased" | "exclusiveEventBased" => {
            let e = Ellipse::new(KPoint::new(cx, cy), (s * 1.4, s * 1.4), 0.0);
            d.extend(e.to_path(PATH_TOLERANCE).iter());
            let inner = Ellipse::new(KPoint::new(cx, cy), (s * 1.1, s * 1.1), 0.0);
            d.extend(inner.to_path(PATH_TOLERANCE).iter());
        }
        "complex" => {
            line(&mut d, (cx, cy - s * 1.3), (cx, cy + s * 1.3));
            line(&mut d, (cx - s * 1.3, cy), (cx + s * 1.3, cy));
            line(&mut d, (cx - s, cy - s), (cx + s, cy + s));
            line(&mut d, (cx + s, cy - s), (cx - s, cy + s));
        }
        _ => {}
    }

    // Like an event, a gateway is labelled below rather than inside.
    let geo = diamond(x, y, w, h).with_insets(Insets::new(2.0, h * 0.8, 2.0, 0.0));
    if d.is_empty() { geo } else { geo.with_decoration(d) }
}

/// A data object: a page with a folded corner, marked for collection, input, or
/// output as the published `dataType` says.
fn bpmn_data_object(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let geo = folded_corner(x, y, w, h, Fold::TopRight);
    let fold = (w.min(h) * 0.25).min(28.0);
    let mut d = geo.decoration.clone().unwrap_or_default();

    match shape.data_type.as_deref().unwrap_or("none") {
        // Three bars at the bottom centre.
        "collection" => {
            let bar_h = h * 0.18;
            let bar_w = w * 0.06;
            let gap = bar_w * 0.8;
            let total = bar_w * 3.0 + gap * 2.0;
            let mut bx = x + (w - total) / 2.0;
            for _ in 0..3 {
                d.extend(rect_path(bx, y + h - bar_h - 6.0, bar_w, bar_h).iter());
                bx += bar_w + gap;
            }
        }
        // An arrow in the top left, hollow for input and solid for output.
        "input" | "output" => {
            let s = (w.min(h) * 0.16).min(14.0);
            let (ax, ay) = (x + 6.0, y + 6.0 + s / 2.0);
            d.move_to((ax, ay - s / 3.0));
            d.line_to((ax + s, ay - s / 3.0));
            d.line_to((ax + s, ay - s / 2.0));
            d.line_to((ax + s * 1.6, ay));
            d.line_to((ax + s, ay + s / 2.0));
            d.line_to((ax + s, ay + s / 3.0));
            d.line_to((ax, ay + s / 3.0));
            d.close_path();
        }
        _ => {}
    }

    ShapeGeometry::closed(geo.outline)
        .with_decoration(d)
        .with_insets(Insets::new(6.0, fold + 4.0, 6.0, h * 0.22))
}

/// A conversation: a hexagon, doubled for a call conversation and marked with a
/// sub-conversation cross when it has one.
fn bpmn_conversation(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let notch = w * 0.25;
    let outline = polygon(&[
        (x + notch, y),
        (x + w - notch, y),
        (x + w, y + h / 2.0),
        (x + w - notch, y + h),
        (x + notch, y + h),
        (x, y + h / 2.0),
    ]);
    let mut d = BezPath::new();

    if shape.is_call.unwrap_or(false) {
        let gap = w.min(h) * 0.06;
        d.extend(
            polygon(&[
                (x + notch + gap, y + gap),
                (x + w - notch - gap, y + gap),
                (x + w - gap, y + h / 2.0),
                (x + w - notch - gap, y + h - gap),
                (x + notch + gap, y + h - gap),
                (x + gap, y + h / 2.0),
            ])
            .iter(),
        );
    }
    if shape.is_sub_conversation.unwrap_or(false) {
        let s = (w.min(h) * 0.14).min(12.0);
        let (cx, by) = (x + w / 2.0, y + h - s - 4.0);
        d.extend(rect_path(cx - s / 2.0, by, s, s).iter());
        line(&mut d, (cx, by + s * 0.25), (cx, by + s * 0.75));
        line(&mut d, (cx - s * 0.25, by + s / 2.0), (cx + s * 0.25, by + s / 2.0));
    }

    let geo = ShapeGeometry::closed(outline).with_insets(Insets::new(notch, 4.0, notch, 4.0));
    if d.is_empty() { geo } else { geo.with_decoration(d) }
}

/// A choreography: a rounded rectangle with a participant band at the top and
/// another at the bottom.
fn bpmn_choreography(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w.min(h) * 0.1).min(12.0);
    let band = (h * 0.22).clamp(12.0, 34.0).min(h / 3.0);
    let mut d = BezPath::new();
    line(&mut d, (x, y + band), (x + w, y + band));
    line(&mut d, (x, y + h - band), (x + w, y + h - band));

    ShapeGeometry::closed(rounded_path(x, y, w, h, r))
        .with_decoration(d)
        .with_insets(Insets::new(6.0, band + 4.0, 6.0, band + 4.0))
}

/// A pool: a rectangle with a title band down one side and `lanes` dividers
/// across the rest. A black box pool is the same frame with no lanes.
fn bpmn_pool(shape: &Shape, x: f64, y: f64, w: f64, h: f64, default_lanes: usize) -> ShapeGeometry {
    let lanes = shape
        .extra
        .get("lanes")
        .and_then(|v| v.as_array())
        .map(|a| a.len().clamp(1, 64))
        .unwrap_or(default_lanes);

    let mut d = BezPath::new();
    if shape.is_vertical() {
        let band = (h * 0.14).clamp(14.0, 40.0).min(h);
        line(&mut d, (x, y + band), (x + w, y + band));
        for i in 1..lanes {
            let lx = x + w * i as f64 / lanes as f64;
            line(&mut d, (lx, y + band), (lx, y + h));
        }
        ShapeGeometry::closed(rect_path(x, y, w, h))
            .with_decoration(d)
            .with_insets(Insets::new(6.0, 2.0, 6.0, (h - band + 2.0).max(0.0)))
    } else {
        let band = (w * 0.08).clamp(14.0, 40.0).min(w);
        line(&mut d, (x + band, y), (x + band, y + h));
        for i in 1..lanes {
            let ly = y + h * i as f64 / lanes as f64;
            line(&mut d, (x + band, ly), (x + w, ly));
        }
        ShapeGeometry::closed(rect_path(x, y, w, h))
            .with_decoration(d)
            .with_insets(Insets::new(2.0, 4.0, (w - band + 2.0).max(0.0), 4.0))
    }
}

/// A text annotation: a bracket beside the text, with no box around it. The
/// full rectangle is kept unpainted so the annotation is still selectable over
/// its whole area.
fn bpmn_text_annotation(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let arm = (w * 0.12).min(18.0);
    let mut d = BezPath::new();
    d.move_to((x + arm, y));
    d.line_to((x, y));
    d.line_to((x, y + h));
    d.line_to((x + arm, y + h));

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(d)
        .with_insets(Insets::new(arm + 6.0, 4.0, 4.0, 4.0))
}

// ---------------------------------------------------------------------------
// Table library
// ---------------------------------------------------------------------------

/// A table: the frame plus the rules its row and column counts imply. Cell text
/// is not drawn, because the format stores it per cell rather than on the
/// shape; the cells survive a round trip untouched.
fn table(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let (rows, cols) = shape.resolved_table_size();
    let mut d = BezPath::new();

    if shape.horizontal_border.unwrap_or(true) {
        for r in 1..rows {
            let ly = y + h * r as f64 / rows as f64;
            line(&mut d, (x, ly), (x + w, ly));
        }
    }
    if shape.vertical_border.unwrap_or(true) {
        for c in 1..cols {
            let lx = x + w * c as f64 / cols as f64;
            line(&mut d, (lx, y), (lx, y + h));
        }
    }

    // The shape's own text is the header, so it belongs in the first row.
    let header = h / rows as f64;
    let geo = ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_insets(Insets::new(6.0, 2.0, 6.0, (h - header + 2.0).max(0.0)));
    if d.is_empty() { geo } else { geo.with_decoration(d) }
}

// ---------------------------------------------------------------------------
// AWS, Azure, and GCP
// ---------------------------------------------------------------------------

/// A cloud service icon. The published libraries ship their artwork separately
/// from the format, so the tile carries a vendor mark rather than a fake icon.
fn named_shape(shape: &Shape, x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w.min(h) * 0.12).min(12.0);
    let s = w.min(h) * 0.26;
    let (cx, cy) = (x + w / 2.0, y + h * 0.42);
    let mut d = BezPath::new();

    match shape.cloud_vendor() {
        // A smile under the wordmark.
        Some(dc_model::CloudVendor::Aws) => {
            d.move_to((cx - s, cy + s * 0.3));
            d.curve_to(
                (cx - s * 0.3, cy + s * 1.1),
                (cx + s * 0.3, cy + s * 1.1),
                (cx + s, cy + s * 0.3),
            );
            line(&mut d, (cx + s * 0.55, cy + s * 0.5), (cx + s, cy + s * 0.3));
        }
        // Azure's cut chevron.
        Some(dc_model::CloudVendor::Azure) => {
            d.move_to((cx - s, cy + s));
            d.line_to((cx - s * 0.1, cy - s));
            d.line_to((cx + s * 0.45, cy - s * 0.1));
            d.line_to((cx - s * 0.2, cy + s * 0.35));
            d.line_to((cx + s, cy + s));
            d.close_path();
        }
        // GCP's four-segment hexagon.
        Some(dc_model::CloudVendor::Gcp) => {
            d.extend(
                polygon(&[
                    (cx - s * 0.5, cy - s),
                    (cx + s * 0.5, cy - s),
                    (cx + s, cy),
                    (cx + s * 0.5, cy + s),
                    (cx - s * 0.5, cy + s),
                    (cx - s, cy),
                ])
                .iter(),
            );
            line(&mut d, (cx - s * 0.5, cy), (cx + s * 0.5, cy));
        }
        // Not one of the three published cloud libraries: a neutral badge.
        None => {
            let e = Ellipse::new(KPoint::new(cx, cy), (s * 0.8, s * 0.8), 0.0);
            d.extend(e.to_path(PATH_TOLERANCE).iter());
        }
    }

    ShapeGeometry::closed(rounded_path(x, y, w, h, r))
        .with_decoration(d)
        .with_insets(Insets::new(3.0, h * 0.68, 3.0, 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Style};

    fn shape_of(t: ShapeType) -> Shape {
        Shape::new("s", t, BoundingBox::new(10.0, 20.0, 120.0, 60.0))
    }

    fn all_known_types() -> Vec<ShapeType> {
        ShapeType::KNOWN.iter().map(|s| ShapeType::from(*s)).collect()
    }

    #[test]
    fn every_published_type_produces_geometry() {
        for t in all_known_types() {
            let geo = geometry(&shape_of(t.clone()));
            assert!(
                !geo.outline.is_empty(),
                "{} produced an empty outline",
                t.as_str()
            );
        }
    }

    #[test]
    fn outlines_stay_within_their_bounding_box() {
        // Every symbol must paint inside its box, or selection handles and
        // culling would be wrong.
        for t in all_known_types() {
            let s = shape_of(t.clone());
            let bb = s.bounding_box;
            let geo = geometry(&s);
            let bounds = geo.outline.bounding_box();

            let slack = 0.75; // allows for curve flattening
            assert!(
                bounds.x0 >= bb.x - slack
                    && bounds.y0 >= bb.y - slack
                    && bounds.x1 <= bb.x + bb.w + slack
                    && bounds.y1 <= bb.y + bb.h + slack,
                "{} escaped its box: {:?} vs {:?}",
                t.as_str(),
                bounds,
                bb
            );
        }
    }

    #[test]
    fn outlines_fill_most_of_their_box() {
        // Guards against a builder that silently collapses to a dot.
        for t in all_known_types() {
            let s = shape_of(t.clone());
            let geo = geometry(&s);
            let b = geo.outline.bounding_box();
            assert!(
                b.width() >= s.bounding_box.w * 0.5 && b.height() >= s.bounding_box.h * 0.5,
                "{} only covers {}x{} of {}x{}",
                t.as_str(),
                b.width(),
                b.height(),
                s.bounding_box.w,
                s.bounding_box.h
            );
        }
    }

    #[test]
    fn zero_sized_shapes_do_not_panic() {
        for t in all_known_types() {
            let s = Shape::new("s", t.clone(), BoundingBox::new(0.0, 0.0, 0.0, 0.0));
            let _ = geometry(&s);
        }
    }

    #[test]
    fn negative_sized_shapes_are_clamped() {
        for t in all_known_types() {
            let s = Shape::new("s", t.clone(), BoundingBox::new(5.0, 5.0, -50.0, -20.0));
            let geo = geometry(&s);
            let b = geo.outline.bounding_box();
            assert!(b.width() >= 0.0 && b.height() >= 0.0, "{}", t.as_str());
        }
    }

    #[test]
    fn rotation_widens_a_rotated_rectangle() {
        // A 120x60 rectangle turned 45 degrees needs (120+60)/sqrt(2) of width.
        let mut s = shape_of(ShapeType::Rectangle);
        let before = geometry(&s).outline.bounding_box();
        s.bounding_box = s.bounding_box.with_rotation(45.0);
        let after = geometry(&s).outline.bounding_box();

        assert!(after.width() > before.width(), "{} vs {}", after.width(), before.width());
        let expected = (120.0 + 60.0) / 2f64.sqrt();
        assert!((after.width() - expected).abs() < 0.5, "width was {}", after.width());
    }

    #[test]
    fn rotation_is_about_the_center() {
        // A diamond turned 45 degrees becomes an axis-aligned square and so
        // gets *narrower*; what must hold in every case is that the center
        // stays put.
        let mut s = shape_of(ShapeType::Decision);
        s.bounding_box = s.bounding_box.with_rotation(45.0);
        let after = geometry(&s).outline.bounding_box();

        let c = s.bounding_box.center();
        assert!((after.center().x - c.x).abs() < 0.5, "x drifted to {}", after.center().x);
        assert!((after.center().y - c.y).abs() < 0.5, "y drifted to {}", after.center().y);
        assert!(after.width() < 120.0, "a rotated diamond is narrower, not wider");
    }

    #[test]
    fn rounding_is_applied_from_style() {
        let mut square = shape_of(ShapeType::Rectangle);
        let sharp = geometry(&square).outline.segments().count();

        square.style = Some(Style { rounding: Some(20.0), ..Default::default() });
        let rounded = geometry(&square).outline.segments().count();
        assert!(rounded > sharp, "a rounded rect has more segments than a sharp one");
    }

    #[test]
    fn excessive_rounding_does_not_self_intersect() {
        let mut s = Shape::new("s", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 40.0, 20.0));
        s.style = Some(Style { rounding: Some(10_000.0), ..Default::default() });
        let b = geometry(&s).outline.bounding_box();
        assert!(b.width() <= 40.5 && b.height() <= 20.5);
    }

    #[test]
    fn decorated_types_carry_decoration() {
        for t in [
            ShapeType::PredefinedProcess,
            ShapeType::InternalStorage,
            ShapeType::Or,
            ShapeType::SummingJunction,
            ShapeType::Database,
            ShapeType::DirectAccessStorage,
            ShapeType::Note,
            ShapeType::StickyNote,
            ShapeType::MultipleDocuments,
        ] {
            assert!(
                geometry(&shape_of(t.clone())).decoration.is_some(),
                "{} should have interior detail",
                t.as_str()
            );
        }
    }

    #[test]
    fn plain_types_carry_no_decoration() {
        for t in [ShapeType::Rectangle, ShapeType::Process, ShapeType::Decision] {
            assert!(geometry(&shape_of(t.clone())).decoration.is_none(), "{}", t.as_str());
        }
    }

    #[test]
    fn brace_is_open_and_respects_facing() {
        let mut s = shape_of(ShapeType::BraceNote);
        let left = geometry(&s);
        assert!(!left.filled, "a brace is a stroke, not a region");

        s.right_facing = Some(true);
        let right = geometry(&s);
        // Mirroring keeps the same box but moves the cusp to the other side.
        let lb = left.outline.bounding_box();
        let rb = right.outline.bounding_box();
        assert!((lb.width() - rb.width()).abs() < 1.0);
        assert!(rb.x1 > lb.x1 - 1.0);
    }

    #[test]
    fn text_block_and_brace_are_unfilled() {
        assert!(!geometry(&shape_of(ShapeType::TextBlock)).filled);
        assert!(!geometry(&shape_of(ShapeType::BraceNote)).filled);
        assert!(geometry(&shape_of(ShapeType::Rectangle)).filled);
    }

    #[test]
    fn tight_shapes_reserve_less_room_for_text() {
        let diamond = geometry(&shape_of(ShapeType::Decision)).text_insets;
        let rect = geometry(&shape_of(ShapeType::Rectangle)).text_insets;
        assert!(
            diamond.x0 > rect.x0,
            "a diamond must inset text further than a rectangle"
        );
    }

    #[test]
    fn every_library_draws_something_other_than_a_rectangle() {
        // The point of implementing a library is that its shapes stop being
        // rectangles, so each one must contribute at least one outline that a
        // plain rectangle's four segments cannot account for.
        for library in dc_model::ShapeLibrary::ALL {
            // A library whose contents come from the person using it rather
            // than from a fixed list has nothing here to assert about.
            if library.types().is_empty() {
                continue;
            }
            let distinctive = library.types().iter().any(|t| {
                let s = shape_of(t.clone());
                let geo = geometry(&s);
                geo.outline.segments().count() != 4 || geo.decoration.is_some()
            });
            assert!(
                distinctive,
                "{} draws nothing but rectangles",
                library.display_name()
            );
        }
    }

    #[test]
    fn bpmn_event_border_weight_follows_its_group() {
        let mut s = shape_of(ShapeType::BpmnEvent);
        s.event_group = Some("start".into());
        assert!(geometry(&s).decoration.is_none(), "a start event is a plain ring");

        s.event_group = Some("intermediate".into());
        assert!(
            geometry(&s).decoration.is_some(),
            "an intermediate event is a double ring"
        );

        s.event_group = Some("end".into());
        assert!(geometry(&s).decoration.is_some(), "an end event has a heavy ring");
    }

    #[test]
    fn bpmn_event_triggers_draw_a_glyph() {
        let mut plain = shape_of(ShapeType::BpmnEvent);
        plain.event_group = Some("start".into());
        let bare = geometry(&plain).decoration.is_some();
        assert!(!bare);

        for trigger in ["timer", "message", "error", "signal"] {
            let mut s = shape_of(ShapeType::BpmnEvent);
            s.event_group = Some("start".into());
            s.event_type = Some(trigger.into());
            assert!(
                geometry(&s).decoration.is_some(),
                "{trigger} should draw a glyph"
            );
        }
    }

    #[test]
    fn bpmn_gateway_markers_vary_by_type() {
        let mut none = shape_of(ShapeType::BpmnGateway);
        none.gateway_type = Some("none".into());
        assert!(geometry(&none).decoration.is_none());

        let mut exclusive = shape_of(ShapeType::BpmnGateway);
        exclusive.gateway_type = Some("exclusive".into());
        let x_marks = geometry(&exclusive).decoration.unwrap().segments().count();

        let mut complex = shape_of(ShapeType::BpmnGateway);
        complex.gateway_type = Some("complex".into());
        let star_marks = geometry(&complex).decoration.unwrap().segments().count();
        assert!(star_marks > x_marks, "complex has more strokes than exclusive");
    }

    #[test]
    fn bpmn_activity_types_change_the_border() {
        let mut task = shape_of(ShapeType::BpmnActivity);
        task.activity_type = Some("task".into());
        assert!(geometry(&task).decoration.is_none());

        let mut transaction = shape_of(ShapeType::BpmnActivity);
        transaction.activity_type = Some("transaction".into());
        assert!(
            geometry(&transaction).decoration.is_some(),
            "a transaction is a double border"
        );
    }

    #[test]
    fn a_text_annotation_has_no_box_around_it() {
        let geo = geometry(&shape_of(ShapeType::BpmnTextAnnotation));
        assert!(!geo.stroked, "the rectangle is geometry, not a border");
        assert!(!geo.filled);
        assert!(geo.decoration.is_some(), "the bracket is what gets drawn");
    }

    #[test]
    fn a_text_block_has_no_border() {
        let geo = geometry(&shape_of(ShapeType::TextBlock));
        assert!(!geo.stroked);
        assert!(!geo.filled);
    }

    #[test]
    fn a_table_rules_follow_its_row_and_column_counts() {
        let mut s = shape_of(ShapeType::Table);
        s.row_count = Some(2);
        s.col_count = Some(2);
        let few = geometry(&s).decoration.unwrap().segments().count();

        s.row_count = Some(6);
        s.col_count = Some(5);
        let many = geometry(&s).decoration.unwrap().segments().count();
        assert!(many > few, "more rows and columns means more rules");
    }

    #[test]
    fn table_borders_can_be_switched_off() {
        let mut s = shape_of(ShapeType::Table);
        s.row_count = Some(4);
        s.col_count = Some(4);
        s.horizontal_border = Some(false);
        s.vertical_border = Some(false);
        assert!(geometry(&s).decoration.is_none());
    }

    #[test]
    fn lane_counts_come_from_the_lanes_property() {
        let mut s = shape_of(ShapeType::SwimLanes);
        let default_rules = geometry(&s).decoration.unwrap().segments().count();

        s.extra.insert(
            "lanes".into(),
            serde_json::json!([{ "title": "a" }, { "title": "b" }, { "title": "c" }, { "title": "d" }]),
        );
        let four = geometry(&s).decoration.unwrap().segments().count();
        assert!(four > default_rules, "four lanes need more dividers than three");
    }

    #[test]
    fn a_pool_puts_its_title_band_on_the_side_it_is_told_to() {
        let mut horizontal = shape_of(ShapeType::BpmnPool);
        horizontal.vertical = Some(false);
        let h_insets = geometry(&horizontal).text_insets;
        assert!(h_insets.x1 > h_insets.y1, "a horizontal pool is titled down its left");

        let mut vertical = shape_of(ShapeType::BpmnPool);
        vertical.vertical = Some(true);
        let v_insets = geometry(&vertical).text_insets;
        assert!(v_insets.y1 > v_insets.x1, "a vertical pool is titled across its top");
    }

    #[test]
    fn an_arrow_points_where_its_orientation_says() {
        let tip_of = |orientation: &str| {
            let mut s = shape_of(ShapeType::SingleArrow);
            s.orientation = Some(orientation.into());
            let b = geometry(&s).outline.bounding_box();
            (b.width(), b.height())
        };
        // Whichever way it points, a block arrow fills its box.
        for o in ["right", "left", "up", "down"] {
            let (w, h) = tip_of(o);
            assert!((w - 120.0).abs() < 1.0 && (h - 60.0).abs() < 1.0, "{o}: {w}x{h}");
        }

        // The point itself moves: the rightward arrow's extreme x is its tip.
        let mut right = shape_of(ShapeType::SingleArrow);
        right.orientation = Some("right".into());
        let mut left = shape_of(ShapeType::SingleArrow);
        left.orientation = Some("left".into());
        let insets = |s: &Shape| geometry(s).text_insets;
        assert!(insets(&right).x1 > insets(&right).x0, "a right arrow insets on the right");
        assert!(insets(&left).x0 > insets(&left).x1, "a left arrow insets on the left");
    }

    #[test]
    fn a_star_follows_its_published_shape_property() {
        let mut s = shape_of(ShapeType::PolyStar);
        s.shape = Some(dc_model::StarShape { num_points: 5, inner_radius: 0.4 });
        let five = geometry(&s).outline.segments().count();

        s.shape = Some(dc_model::StarShape { num_points: 12, inner_radius: 0.4 });
        let twelve = geometry(&s).outline.segments().count();
        assert!(twelve > five, "a twelve-pointed star has more edges");
    }

    #[test]
    fn a_flexible_polygon_uses_its_vertices() {
        let mut s = shape_of(ShapeType::FlexiblePolygon);
        s.vertices = Some(vec![
            dc_model::RelPoint::new(0.5, 0.0),
            dc_model::RelPoint::new(1.0, 1.0),
            dc_model::RelPoint::new(0.0, 1.0),
        ]);
        let geo = geometry(&s);
        assert_eq!(geo.outline.segments().count(), 3, "three vertices, three edges");
    }

    #[test]
    fn a_cross_respects_its_indent() {
        let mut thin = shape_of(ShapeType::Cross);
        thin.indent = Some(dc_model::RelPoint::new(0.45, 0.45));
        let mut thick = shape_of(ShapeType::Cross);
        thick.indent = Some(dc_model::RelPoint::new(0.1, 0.1));
        assert!(
            geometry(&thin).text_insets.x0 > geometry(&thick).text_insets.x0,
            "thinner arms leave less room for text"
        );
    }

    #[test]
    fn freehand_drawing_follows_its_recorded_points() {
        let mut s = shape_of(ShapeType::FreehandDrawing);
        s.points = Some(vec![
            dc_model::Point::new(0.0, 0.0),
            dc_model::Point::new(5.0, 10.0),
            dc_model::Point::new(10.0, 0.0),
            dc_model::Point::new(15.0, 8.0),
        ]);
        let geo = geometry(&s);
        assert!(!geo.filled, "a freehand stroke is a line, not a region");
        assert_eq!(geo.outline.segments().count(), 3, "four points, three segments");

        // And the stroke is scaled into the shape's box, so moving the shape
        // moves the drawing.
        let b = geo.outline.bounding_box();
        let bb = s.bounding_box;
        assert!((b.x0 - bb.x).abs() < 0.5 && (b.x1 - (bb.x + bb.w)).abs() < 0.5);
    }

    #[test]
    fn freehand_drawing_without_points_is_still_visible() {
        let geo = geometry(&shape_of(ShapeType::FreehandDrawing));
        assert!(!geo.outline.is_empty());
    }

    #[test]
    fn cloud_shapes_carry_their_vendor_mark() {
        for class in [
            "ArchAmazonEC2AWS2024",
            "VirtualMachineAzure2024",
            "GCP2021CloudStorageIcon",
        ] {
            let mut s = shape_of(ShapeType::NamedShape);
            s.class_name = Some(class.into());
            assert!(
                geometry(&s).decoration.is_some(),
                "{class} should draw a vendor mark"
            );
        }
    }

    #[test]
    fn containers_label_themselves_in_a_title_bar() {
        for t in [
            ShapeType::RectangleContainer,
            ShapeType::RoundedRectangleContainer,
            ShapeType::PillContainer,
            ShapeType::SparkFrame,
            ShapeType::NamedContainer,
        ] {
            let s = shape_of(t.clone());
            let insets = geometry(&s).text_insets;
            assert!(
                insets.y1 > s.bounding_box.h / 2.0,
                "{} should keep its label out of the body",
                t.as_str()
            );
        }
    }

    #[test]
    fn unknown_type_falls_back_to_a_visible_rectangle() {
        let s = shape_of(ShapeType::Other("awsLambda".into()));
        let geo = geometry(&s);
        let b = geo.outline.bounding_box();
        assert!(geo.filled);
        assert!((b.width() - 120.0).abs() < 0.5);
    }

    #[test]
    fn predefined_process_side_bars_follow_side_width() {
        let mut s = shape_of(ShapeType::PredefinedProcess);
        s.side_width = Some(0.3);
        let wide = geometry(&s).text_insets.x0;
        s.side_width = Some(0.02);
        let narrow = geometry(&s).text_insets.x0;
        assert!(wide > narrow);
    }
}
