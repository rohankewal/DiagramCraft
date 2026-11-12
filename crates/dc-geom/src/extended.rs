//! Geometry for the notations the published format does not cover.
//!
//! UML, entity-relationship, network and wireframe shapes are standards or
//! long-settled conventions in their own right, but the Standard Import
//! format publishes no types for them, so they are drawn here from their own
//! definitions rather than from Lucid's.
//!
//! Each function takes the shape's box and returns the same
//! [`ShapeGeometry`] the published shapes return, so everything downstream —
//! rendering, hit testing, export, text layout — treats them identically.
//!
//! The convention throughout: `outline` is the shape's silhouette, and
//! `decoration` carries interior rules that are stroked but never filled.

use crate::shapes::{ShapeGeometry, ellipse_path_geo, line, polygon, rect_path, rounded_path};
use kurbo::{BezPath, Insets};

// ------------------------------------------------------------------- UML

/// A class box: a name compartment over attributes over operations.
pub fn uml_class(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let header = (h * 0.3).clamp(14.0, 34.0);
    let middle = header + (h - header) / 2.0;
    let mut rules = BezPath::new();
    line(&mut rules, (x, y + header), (x + w, y + header));
    if h > header + 24.0 {
        line(&mut rules, (x, y + middle), (x + w, y + middle));
    }
    // Text sits in the name compartment, which is the one a label means.
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(rules)
        .with_insets(Insets::new(6.0, 4.0, 6.0, h - header + 4.0))
}

/// An interface, drawn as a class box; the «interface» marker is text.
pub fn uml_interface(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    uml_class(x, y, w, h)
}

/// A package: a tab along the top left, then the body.
pub fn uml_package(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let tab_h = (h * 0.18).clamp(10.0, 26.0).min(h);
    // The tab wants to be about 30 points wide, but never more than the
    // shape itself: on a narrow package the upper bound is the binding one,
    // and clamping with the bounds the wrong way round would panic.
    let tab_w = (w * 0.4).max(30.0).min(w * 0.6);
    let outline = polygon(&[
        (x, y),
        (x + tab_w, y),
        (x + tab_w, y + tab_h),
        (x + w, y + tab_h),
        (x + w, y + h),
        (x, y + h),
    ]);
    ShapeGeometry::closed(outline)
        .with_insets(Insets::new(6.0, tab_h + 4.0, 6.0, 4.0))
}

/// An actor: the stick figure, drawn entirely as decoration because a stick
/// figure encloses nothing to fill.
pub fn uml_actor(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cx = x + w / 2.0;
    let head_r = (w.min(h) * 0.22).max(3.0);
    let head_cy = y + head_r;
    let shoulder = head_cy + head_r * 1.4;
    let hip = y + h * 0.62;

    let mut figure = BezPath::new();
    // Head.
    let head = kurbo::Ellipse::new(kurbo::Point::new(cx, head_cy), (head_r, head_r), 0.0);
    figure.extend(kurbo::Shape::path_elements(&head, 0.1));
    line(&mut figure, (cx, shoulder), (cx, hip));
    line(&mut figure, (x, shoulder + (hip - shoulder) * 0.25), (x + w, shoulder + (hip - shoulder) * 0.25));
    line(&mut figure, (cx, hip), (x, y + h));
    line(&mut figure, (cx, hip), (x + w, y + h));

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(figure)
        // The label goes under the figure, which is where UML puts it.
        .with_insets(Insets::new(0.0, h, 0.0, -16.0))
}

/// A use case: an ellipse.
pub fn uml_use_case(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ellipse_path_geo(x, y, w, h)
}

/// A lifeline: the head box with its tail dropping from it.
///
/// The outline is the whole box rather than just the head, for two reasons:
/// the entire lifeline is then clickable, which is what someone reaching for
/// one expects, and the shape keeps the convention the rest of this crate
/// follows, where the outline is the extent and not merely the painted part.
/// Head and tail are both decoration, which is why neither takes a fill.
pub fn uml_lifeline(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let head = (h * 0.2).clamp(24.0, 60.0).min(h);
    let mut parts = rect_path(x, y, w, head);
    line(&mut parts, (x + w / 2.0, y + head), (x + w / 2.0, y + h));

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(parts)
        .with_insets(Insets::new(6.0, 4.0, 6.0, (h - head + 4.0).max(0.0)))
}

/// An activation bar on a lifeline.
pub fn uml_activation(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ShapeGeometry::closed(rect_path(x, y, w, h)).with_insets(Insets::uniform(1.0))
}

/// A component: a box with the two tabs out of its left edge.
pub fn uml_component(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let tab_w = (w * 0.16).clamp(10.0, 26.0);
    let tab_h = (h * 0.16).clamp(6.0, 16.0);
    let left = x + tab_w / 2.0;

    let mut tabs = BezPath::new();
    for i in 0..2 {
        let ty = y + h * (0.3 + 0.3 * i as f64) - tab_h / 2.0;
        tabs.extend(kurbo::Shape::path_elements(
            &kurbo::Rect::new(x, ty, x + tab_w, ty + tab_h),
            0.1,
        ));
    }

    ShapeGeometry::closed(rect_path(left, y, w - tab_w / 2.0, h))
        .with_decoration(tabs)
        .with_insets(Insets::new(tab_w + 4.0, 4.0, 6.0, 4.0))
}

/// A node: a box drawn in three dimensions.
pub fn uml_node(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let d = (w.min(h) * 0.2).clamp(6.0, 24.0);
    let front = kurbo::Rect::new(x, y + d, x + w - d, y + h);

    let mut top = BezPath::new();
    top.move_to((x, y + d));
    top.line_to((x + d, y));
    top.line_to((x + w, y));
    top.line_to((x + w, y + h - d));
    top.line_to((x + w - d, y + h));
    top.line_to((x + w - d, y + d));
    top.close_path();

    ShapeGeometry::closed(kurbo::Shape::to_path(&front, 0.1))
        .with_decoration(top)
        .with_insets(Insets::new(6.0, d + 4.0, d + 6.0, 4.0))
}

/// A state: a rounded box.
pub fn uml_state(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ShapeGeometry::closed(rounded_path(x, y, w, h, (w.min(h) * 0.25).min(14.0)))
}

/// An initial state: a filled disc.
pub fn uml_initial_state(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let d = w.min(h);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let disc = kurbo::Ellipse::new(kurbo::Point::new(cx, cy), (d / 2.0, d / 2.0), 0.0);
    ShapeGeometry::closed(kurbo::Shape::to_path(&disc, 0.1)).with_insets(Insets::uniform(0.0))
}

/// A final state: a ring with a filled centre.
pub fn uml_final_state(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let d = w.min(h);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let outer = kurbo::Ellipse::new(kurbo::Point::new(cx, cy), (d / 2.0, d / 2.0), 0.0);
    let inner = kurbo::Ellipse::new(
        kurbo::Point::new(cx, cy),
        (d * 0.28, d * 0.28),
        0.0,
    );
    ShapeGeometry::open(kurbo::Shape::to_path(&outer, 0.1))
        .with_decoration(kurbo::Shape::to_path(&inner, 0.1))
        .with_insets(Insets::uniform(0.0))
}

/// A fork or join: a solid bar.
pub fn uml_fork(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ShapeGeometry::closed(rect_path(x, y, w, h)).with_insets(Insets::uniform(0.0))
}

// ------------------------------------------------------------------- ERD

/// An entity: a named box over its attribute rows.
pub fn erd_entity(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let header = (h * 0.3).clamp(16.0, 32.0);
    let mut rule = BezPath::new();
    line(&mut rule, (x, y + header), (x + w, y + header));
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(rule)
        .with_insets(Insets::new(6.0, 4.0, 6.0, h - header + 4.0))
}

/// A weak entity: an entity inside a second border.
pub fn erd_weak_entity(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let gap = (w.min(h) * 0.06).clamp(3.0, 6.0);
    let header = (h * 0.3).clamp(16.0, 32.0);
    let mut detail = rect_path(x + gap, y + gap, w - gap * 2.0, h - gap * 2.0);
    line(&mut detail, (x + gap, y + header), (x + w - gap, y + header));
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(detail)
        .with_insets(Insets::new(gap + 6.0, gap + 4.0, gap + 6.0, h - header + 4.0))
}

/// A relationship: a diamond.
pub fn erd_relationship(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let outline = polygon(&[
        (x + w / 2.0, y),
        (x + w, y + h / 2.0),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
    ]);
    ShapeGeometry::closed(outline).with_insets(Insets::new(w / 4.0, h / 4.0, w / 4.0, h / 4.0))
}

/// A weak relationship: a diamond inside a diamond.
pub fn erd_weak_relationship(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let gap = (w.min(h) * 0.1).clamp(4.0, 10.0);
    let inner = polygon(&[
        (x + w / 2.0, y + gap * 2.0),
        (x + w - gap * 2.0, y + h / 2.0),
        (x + w / 2.0, y + h - gap * 2.0),
        (x + gap * 2.0, y + h / 2.0),
    ]);
    erd_relationship(x, y, w, h).with_decoration(inner)
}

/// An attribute: an ellipse.
pub fn erd_attribute(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ellipse_path_geo(x, y, w, h)
}

/// A multivalued attribute: an ellipse inside an ellipse.
pub fn erd_multivalued_attribute(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let gap = (w.min(h) * 0.08).clamp(3.0, 7.0);
    let inner = kurbo::Ellipse::new(
        kurbo::Point::new(x + w / 2.0, y + h / 2.0),
        (w / 2.0 - gap, h / 2.0 - gap),
        0.0,
    );
    erd_attribute(x, y, w, h).with_decoration(kurbo::Shape::to_path(&inner, 0.1))
}

/// A derived attribute: an ellipse the author dashes, which is a line style
/// rather than a different outline.
pub fn erd_derived_attribute(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    erd_attribute(x, y, w, h)
}

// --------------------------------------------------------------- Network

/// A server: a tall case with drive slots.
pub fn net_server(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut slots = BezPath::new();
    let inset = w * 0.15;
    for i in 0..3 {
        let sy = y + h * (0.12 + 0.1 * i as f64);
        line(&mut slots, (x + inset, sy), (x + w - inset, sy));
    }
    ShapeGeometry::closed(rounded_path(x, y, w, h, 3.0))
        .with_decoration(slots)
        .with_insets(Insets::new(5.0, h * 0.45, 5.0, 5.0))
}

/// A router: the disc with traffic arrows across it.
pub fn net_router(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let r = w.min(h) / 2.0;
    let mut arrows = BezPath::new();
    line(&mut arrows, (cx - r * 0.6, cy - r * 0.2), (cx + r * 0.6, cy - r * 0.2));
    line(&mut arrows, (cx + r * 0.3, cy - r * 0.45), (cx + r * 0.6, cy - r * 0.2));
    line(&mut arrows, (cx + r * 0.6, cy + r * 0.2), (cx - r * 0.6, cy + r * 0.2));
    line(&mut arrows, (cx - r * 0.3, cy + r * 0.45), (cx - r * 0.6, cy + r * 0.2));

    let disc = kurbo::Ellipse::new(kurbo::Point::new(cx, cy), (w / 2.0, h / 2.0), 0.0);
    ShapeGeometry::closed(kurbo::Shape::to_path(&disc, 0.1))
        .with_decoration(arrows)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

/// A switch: a flat box with crossing arrows on its face.
pub fn net_switch(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut arrows = BezPath::new();
    let (ax, ay) = (x + w * 0.2, y + h * 0.35);
    let (bx, by) = (x + w * 0.8, y + h * 0.65);
    line(&mut arrows, (ax, ay), (bx, ay));
    line(&mut arrows, (bx, by), (ax, by));
    ShapeGeometry::closed(rounded_path(x, y, w, h, 3.0))
        .with_decoration(arrows)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

/// A firewall: a brick wall.
pub fn net_firewall(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let rows = 4;
    let row_h = h / rows as f64;
    let mut bricks = BezPath::new();
    for i in 1..rows {
        let ry = y + row_h * i as f64;
        line(&mut bricks, (x, ry), (x + w, ry));
    }
    // Alternating vertical joints, the way bricks are laid.
    for i in 0..rows {
        let ry = y + row_h * i as f64;
        let offset = if i % 2 == 0 { w / 2.0 } else { w / 4.0 };
        let mut jx = x + offset;
        while jx < x + w - 0.5 {
            line(&mut bricks, (jx, ry), (jx, ry + row_h));
            jx += w / 2.0;
        }
    }
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(bricks)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

/// A workstation: a monitor on a stand.
pub fn net_workstation(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let screen_h = h * 0.72;
    let mut stand = BezPath::new();
    let cx = x + w / 2.0;
    line(&mut stand, (cx, y + screen_h), (cx, y + h * 0.88));
    line(&mut stand, (x + w * 0.25, y + h), (x + w * 0.75, y + h));
    line(&mut stand, (cx - w * 0.12, y + h * 0.88), (cx + w * 0.12, y + h * 0.88));

    ShapeGeometry::closed(rounded_path(x, y, w, screen_h, 3.0))
        .with_decoration(stand)
        .with_insets(Insets::new(5.0, 5.0, 5.0, h - screen_h + 5.0))
}

/// A laptop: a screen above an angled base.
pub fn net_laptop(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let screen_h = h * 0.72;
    let lip = w * 0.08;
    let base = polygon(&[
        (x, y + h),
        (x + lip, y + screen_h),
        (x + w - lip, y + screen_h),
        (x + w, y + h),
    ]);
    ShapeGeometry::closed(rounded_path(x + lip, y, w - lip * 2.0, screen_h, 3.0))
        .with_decoration(base)
        .with_insets(Insets::new(lip + 4.0, 5.0, lip + 4.0, h - screen_h + 5.0))
}

/// A mobile device: a rounded slab with a screen inside it.
pub fn net_mobile(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let pad = (w * 0.12).clamp(4.0, 10.0);
    let screen = rect_path(x + pad, y + pad * 1.6, w - pad * 2.0, h - pad * 3.4);
    ShapeGeometry::closed(rounded_path(x, y, w, h, (w * 0.18).min(12.0)))
        .with_decoration(screen)
        .with_insets(Insets::new(pad + 2.0, pad * 1.6 + 2.0, pad + 2.0, pad * 1.8 + 2.0))
}

/// A printer: the body with a sheet out of the top.
pub fn net_printer(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let body_y = y + h * 0.35;
    let mut detail = BezPath::new();
    detail.extend(kurbo::Shape::path_elements(
        &kurbo::Rect::new(x + w * 0.2, y, x + w * 0.8, body_y),
        0.1,
    ));
    line(&mut detail, (x + w * 0.15, y + h * 0.62), (x + w * 0.85, y + h * 0.62));

    ShapeGeometry::closed(rounded_path(x, body_y, w, h - (body_y - y), 3.0))
        .with_decoration(detail)
        .with_insets(Insets::new(5.0, h * 0.45, 5.0, 5.0))
}

/// A rack: a tall frame holding units.
pub fn net_rack(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let units = 5;
    let inset = w * 0.1;
    let unit_h = h / units as f64;
    let mut shelves = BezPath::new();
    for i in 1..units {
        let sy = y + unit_h * i as f64;
        line(&mut shelves, (x + inset, sy), (x + w - inset, sy));
    }
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(shelves)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

/// A wireless access point: a base with signal arcs above it.
pub fn net_wireless(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let base_h = h * 0.3;
    let base_y = y + h - base_h;
    let cx = x + w / 2.0;
    let mut waves = BezPath::new();
    for i in 1..=3 {
        let r = w.min(h) * 0.16 * i as f64;
        let arc = kurbo::Arc::new(
            kurbo::Point::new(cx, base_y),
            (r, r),
            std::f64::consts::PI,
            std::f64::consts::PI,
            0.0,
        );
        waves.extend(kurbo::Shape::path_elements(&arc, 0.1));
    }
    // As with the lifeline, the outline is the whole box so the arcs above
    // the base are part of what can be clicked; the base and the signal are
    // both decoration.
    let mut parts = rounded_path(x + w * 0.2, base_y, w * 0.6, base_h, 3.0);
    parts.extend(waves.iter());

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(parts)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

// ------------------------------------------------------------- Wireframe

/// A browser window: a title bar with three dots, then the page area.
pub fn wire_browser(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let bar = (h * 0.12).clamp(16.0, 28.0);
    let mut chrome = BezPath::new();
    line(&mut chrome, (x, y + bar), (x + w, y + bar));
    let r = (bar * 0.18).clamp(2.0, 4.0);
    for i in 0..3 {
        let cx = x + bar * 0.5 + bar * 0.55 * i as f64;
        let dot = kurbo::Ellipse::new(kurbo::Point::new(cx, y + bar / 2.0), (r, r), 0.0);
        chrome.extend(kurbo::Shape::path_elements(&dot, 0.1));
    }
    ShapeGeometry::closed(rounded_path(x, y, w, h, 4.0))
        .with_decoration(chrome)
        .with_insets(Insets::new(6.0, bar + 6.0, 6.0, 6.0))
}

/// A phone frame: the slab, with an earpiece above the screen.
pub fn wire_phone(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let pad = (w * 0.07).clamp(5.0, 12.0);
    let top = pad * 2.2;
    let bottom = pad * 2.4;
    let mut detail = rect_path(x + pad, y + top, w - pad * 2.0, h - top - bottom);
    line(
        &mut detail,
        (x + w * 0.4, y + top * 0.5),
        (x + w * 0.6, y + top * 0.5),
    );
    ShapeGeometry::closed(rounded_path(x, y, w, h, (w * 0.12).min(18.0)))
        .with_decoration(detail)
        .with_insets(Insets::new(pad + 3.0, top + 3.0, pad + 3.0, bottom + 3.0))
}

/// A button.
pub fn wire_button(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    ShapeGeometry::closed(rounded_path(x, y, w, h, (h * 0.25).min(8.0)))
}

/// A text field: the box with a caret rule inside it.
pub fn wire_text_field(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut caret = BezPath::new();
    line(&mut caret, (x + 8.0, y + h * 0.25), (x + 8.0, y + h * 0.75));
    ShapeGeometry::closed(rounded_path(x, y, w, h, 3.0))
        .with_decoration(caret)
        .with_insets(Insets::new(14.0, 3.0, 8.0, 3.0))
}

/// A checkbox, with its tick.
pub fn wire_checkbox(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut tick = BezPath::new();
    tick.move_to((x + w * 0.22, y + h * 0.52));
    tick.line_to((x + w * 0.42, y + h * 0.74));
    tick.line_to((x + w * 0.8, y + h * 0.28));
    ShapeGeometry::closed(rounded_path(x, y, w, h, 2.0))
        .with_decoration(tick)
        .with_insets(Insets::uniform(0.0))
}

/// A radio button: a ring with its dot.
pub fn wire_radio(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let dot = kurbo::Ellipse::new(
        kurbo::Point::new(cx, cy),
        (w * 0.22, h * 0.22),
        0.0,
    );
    ellipse_path_geo(x, y, w, h)
        .with_decoration(kurbo::Shape::to_path(&dot, 0.1))
        .with_insets(Insets::uniform(0.0))
}

/// A dropdown: a field with the chevron at its right.
pub fn wire_dropdown(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cx = x + w - h * 0.5;
    let cy = y + h * 0.5;
    let s = h * 0.16;
    let mut chevron = BezPath::new();
    chevron.move_to((cx - s, cy - s * 0.5));
    chevron.line_to((cx, cy + s * 0.6));
    chevron.line_to((cx + s, cy - s * 0.5));
    ShapeGeometry::closed(rounded_path(x, y, w, h, 3.0))
        .with_decoration(chevron)
        .with_insets(Insets::new(8.0, 3.0, h + 2.0, 3.0))
}

/// An image placeholder: the box crossed corner to corner.
pub fn wire_image(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut cross = BezPath::new();
    line(&mut cross, (x, y), (x + w, y + h));
    line(&mut cross, (x + w, y), (x, y + h));
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(cross)
        .with_insets(Insets::uniform(6.0))
}

/// A navigation bar: a strip divided into items.
pub fn wire_nav_bar(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut dividers = BezPath::new();
    for i in 1..4 {
        let dx = x + w * (i as f64 / 4.0);
        line(&mut dividers, (dx, y), (dx, y + h));
    }
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(dividers)
        .with_insets(Insets::new(6.0, 3.0, 6.0, 3.0))
}

/// A slider: the track with its knob.
pub fn wire_slider(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cy = y + h / 2.0;
    let r = h / 2.0;
    let mut knob = BezPath::new();
    let circle = kurbo::Ellipse::new(kurbo::Point::new(x + w * 0.6, cy), (r, r), 0.0);
    knob.extend(kurbo::Shape::path_elements(&circle, 0.1));

    // The track is a thin bar across a taller box, so the outline is the box
    // and the bar is drawn with the knob — otherwise the shape's clickable
    // area would be a few points tall whatever its height was set to.
    let mut parts = rounded_path(x, cy - h * 0.15, w, h * 0.3, h * 0.15);
    parts.extend(knob.iter());

    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(parts)
        .with_insets(Insets::uniform(0.0))
}

/// A toggle: a pill with the knob at one end.
pub fn wire_toggle(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = h / 2.0;
    let knob = kurbo::Ellipse::new(
        kurbo::Point::new(x + w - r, y + r),
        (r * 0.72, r * 0.72),
        0.0,
    );
    ShapeGeometry::closed(rounded_path(x, y, w, h, r))
        .with_decoration(kurbo::Shape::to_path(&knob, 0.1))
        .with_insets(Insets::uniform(0.0))
}

// ----------------------------------------------------------------- Chart

/// Two overlapping circles.
pub fn chart_venn_two(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w / 3.0).min(h / 2.0);
    let cy = y + h / 2.0;
    let mut path = BezPath::new();
    for cx in [x + r, x + w - r] {
        let c = kurbo::Ellipse::new(kurbo::Point::new(cx, cy), (r, r), 0.0);
        path.extend(kurbo::Shape::path_elements(&c, 0.1));
    }
    // Open, because two overlapping discs filled as one path would punch a
    // hole through the overlap under the non-zero rule.
    ShapeGeometry::open(path).with_insets(Insets::new(w / 3.0, h / 3.0, w / 3.0, h / 3.0))
}

/// Three overlapping circles.
pub fn chart_venn_three(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let r = (w / 3.0).min(h / 3.0);
    let mut path = BezPath::new();
    let centres = [
        (x + w / 2.0, y + r),
        (x + r, y + h - r),
        (x + w - r, y + h - r),
    ];
    for (cx, cy) in centres {
        let c = kurbo::Ellipse::new(kurbo::Point::new(cx, cy), (r, r), 0.0);
        path.extend(kurbo::Shape::path_elements(&c, 0.1));
    }
    ShapeGeometry::open(path).with_insets(Insets::new(w / 3.0, h / 3.0, w / 3.0, h / 3.0))
}

/// A two-by-two matrix.
pub fn chart_matrix(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let mut rules = BezPath::new();
    line(&mut rules, (x + w / 2.0, y), (x + w / 2.0, y + h));
    line(&mut rules, (x, y + h / 2.0), (x + w, y + h / 2.0));
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(rules)
        .with_insets(Insets::new(6.0, 6.0, 6.0, h / 2.0 + 6.0))
}

/// A sitemap page: a sheet with a header band.
pub fn chart_sitemap_page(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let band = (h * 0.22).clamp(8.0, 18.0);
    let mut rule = BezPath::new();
    line(&mut rule, (x, y + band), (x + w, y + band));
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .with_decoration(rule)
        .with_insets(Insets::new(5.0, band + 4.0, 5.0, 4.0))
}

/// A timeline: an axis with tick marks along it.
pub fn chart_timeline(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let cy = y + h / 2.0;
    let mut axis = BezPath::new();
    line(&mut axis, (x, cy), (x + w, cy));
    for i in 0..=4 {
        let tx = x + w * (i as f64 / 4.0);
        line(&mut axis, (tx, cy - h * 0.2), (tx, cy + h * 0.2));
    }
    ShapeGeometry::closed(rect_path(x, y, w, h))
        .unpainted()
        .with_decoration(axis)
        .with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

/// A milestone: a diamond on a timeline.
pub fn chart_milestone(x: f64, y: f64, w: f64, h: f64) -> ShapeGeometry {
    let outline = polygon(&[
        (x + w / 2.0, y),
        (x + w, y + h / 2.0),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
    ]);
    ShapeGeometry::closed(outline).with_insets(Insets::new(0.0, h, 0.0, -14.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, Shape, ShapeType};
    use kurbo::Shape as _;

    /// Every shape in the new libraries, built at a workable size.
    fn every_extended_shape() -> Vec<(ShapeType, ShapeGeometry)> {
        use dc_model::ShapeLibrary;
        let libraries = [
            ShapeLibrary::Uml,
            ShapeLibrary::Erd,
            ShapeLibrary::Network,
            ShapeLibrary::Wireframe,
            ShapeLibrary::Chart,
        ];
        libraries
            .iter()
            .flat_map(|l| l.types())
            .map(|t| {
                let (w, h) = t.default_size();
                let shape = Shape::new("s", t.clone(), BoundingBox::new(10.0, 20.0, w, h));
                (t, crate::geometry(&shape))
            })
            .collect()
    }

    #[test]
    fn every_new_shape_produces_geometry() {
        for (shape_type, geo) in every_extended_shape() {
            let has_outline = !geo.outline.elements().is_empty();
            let has_decoration = geo.decoration.as_ref().is_some_and(|d| !d.elements().is_empty());
            assert!(
                has_outline || has_decoration,
                "{shape_type} drew nothing at all"
            );
        }
    }

    #[test]
    fn every_new_shape_stays_inside_its_box() {
        for (shape_type, geo) in every_extended_shape() {
            let (w, h) = shape_type.default_size();
            let box_ = kurbo::Rect::new(10.0, 20.0, 10.0 + w, 20.0 + h);
            // A hair of slack for curve flattening.
            let slack = 0.5;
            for path in [Some(&geo.outline), geo.decoration.as_ref()].into_iter().flatten() {
                if path.elements().is_empty() {
                    continue;
                }
                let b = path.bounding_box();
                assert!(
                    b.x0 >= box_.x0 - slack
                        && b.y0 >= box_.y0 - slack
                        && b.x1 <= box_.x1 + slack
                        && b.y1 <= box_.y1 + slack,
                    "{shape_type} drew outside its box: {b:?} not within {box_:?}"
                );
            }
        }
    }

    #[test]
    fn every_new_shape_leaves_somewhere_to_put_text() {
        for (shape_type, geo) in every_extended_shape() {
            let (w, h) = shape_type.default_size();
            let i = geo.text_insets;
            let area_w = w - i.x0 - i.x1;
            let area_h = h - i.y0 - i.y1;
            assert!(
                area_w > 0.0 && area_h > 0.0,
                "{shape_type} left no room for a label: {area_w}×{area_h}"
            );
        }
    }

    #[test]
    fn a_tiny_shape_does_not_produce_a_broken_path() {
        use dc_model::ShapeLibrary;
        for library in [
            ShapeLibrary::Uml,
            ShapeLibrary::Erd,
            ShapeLibrary::Network,
            ShapeLibrary::Wireframe,
            ShapeLibrary::Chart,
        ] {
            for t in library.types() {
                let shape = Shape::new("s", t.clone(), BoundingBox::new(0.0, 0.0, 1.0, 1.0));
                let geo = crate::geometry(&shape);
                for path in [Some(&geo.outline), geo.decoration.as_ref()].into_iter().flatten() {
                    for el in path.elements() {
                        let point = match el {
                            kurbo::PathEl::MoveTo(p)
                            | kurbo::PathEl::LineTo(p)
                            | kurbo::PathEl::QuadTo(_, p)
                            | kurbo::PathEl::CurveTo(_, _, p) => Some(*p),
                            kurbo::PathEl::ClosePath => None,
                        };
                        if let Some(p) = point {
                            assert!(
                                p.x.is_finite() && p.y.is_finite(),
                                "{t} produced a non-finite point at 1×1"
                            );
                        }
                    }
                }
            }
        }
    }
}
