//! The application icon.
//!
//! The icon is drawn rather than stored: the same geometry that draws a
//! terminator and a decision on the canvas draws them here, so the mark can
//! never drift from what the program actually renders. One function produces
//! it at any size, which is what an icon set needs — macOS alone wants six.
//!
//! The composition is deliberately two shapes and one connector. An icon is
//! read at sixteen pixels as often as at a thousand, and anything more
//! becomes a smudge at the small end.

use dc_model::{BoundingBox, Color, EndpointStyle, Shape, ShapeType};
use kurbo::{Affine, Point as KPoint, RoundedRect, Vec2};
use peniko::{Fill as FillRule, Gradient};
use vello::Scene;

/// The side of the square the icon is composed in. Every measurement below is
/// in these units and scales with the requested size.
const CANVAS: f64 = 1024.0;
/// The rounded square sits inside the canvas with a margin, which is how a
/// macOS icon is laid out: the art does not reach the edge.
const MARGIN: f64 = 100.0;
/// Corner radius of the rounded square, as macOS proportions it.
const CORNER: f64 = 185.0;

/// The icon's two background colours, top to bottom.
const TOP: Color = Color::rgb(0x6b, 0x63, 0xf5);
const BOTTOM: Color = Color::rgb(0x43, 0x38, 0xca);

/// The terminator at the top of the flowchart.
const START: BoundingBox = BoundingBox::new(307.0, 232.0, 410.0, 160.0);
/// The decision below it.
const DECISION: BoundingBox = BoundingBox::new(352.0, 540.0, 320.0, 250.0);
/// Weight of the connector between them.
///
/// This also sets the arrowhead: `endpoint_marker` grows the glyph to four
/// times the stroke so a heavy line keeps its proportions, which means a
/// stroke chosen for the shaft alone produces a head that swallows the
/// drawing. Twenty gives an eighty unit head, which is the size this
/// composition wants.
const CONNECTOR_WEIGHT: f64 = 20.0;
/// How far the arrow stops short of the shape it points at, so the two read
/// as separate marks at small sizes instead of merging into one.
const ARROW_GAP: f64 = 12.0;

/// The side of the rounded square itself.
const PLATE: f64 = CANVAS - MARGIN * 2.0;

// Proportions the composition depends on, checked where getting them wrong
// should stop the build rather than wait for a test run.
const _: () = assert!(MARGIN > 0.0, "the art must not reach the canvas edge");
const _: () = assert!(
    PLATE > CANVAS * 0.6,
    "a plate this small reads as a stamp floating in space"
);
const _: () = assert!(
    CORNER < PLATE / 2.0,
    "a radius past half the side makes a circle, not a rounded square"
);

/// Builds the icon into a scene, scaled to `size` pixels square.
pub fn draw(scene: &mut Scene, size: f64) {
    scene.reset();
    let scale = Affine::scale(size / CANVAS);

    // The rounded square. A gradient rather than a flat fill, because a flat
    // one reads as unfinished against the dock's other icons.
    let plate = RoundedRect::new(MARGIN, MARGIN, MARGIN + PLATE, MARGIN + PLATE, CORNER);
    let gradient = Gradient::new_linear(
        (CANVAS / 2.0, MARGIN),
        (CANVAS / 2.0, CANVAS - MARGIN),
    )
    .with_stops([px(TOP), px(BOTTOM)]);
    scene.fill(FillRule::NonZero, scale, &gradient, None, &plate);

    // A flowchart in miniature: a terminator, an arrow, a decision. Both
    // shapes come from the same geometry the canvas uses.
    let white = Color::WHITE;
    for (id, shape_type, bounds) in [
        ("icon.start", ShapeType::Terminator, START),
        ("icon.decision", ShapeType::Decision, DECISION),
    ] {
        let geo = dc_geom::geometry(&Shape::new(id, shape_type, bounds));
        scene.fill(FillRule::NonZero, scale, px(white), None, &geo.outline);
    }

    // The connector between them, with the arrowhead the renderer would draw.
    let tip = KPoint::new(CANVAS / 2.0, DECISION.y - ARROW_GAP);
    let head = (CONNECTOR_WEIGHT * 4.0).max(28.0);

    let mut shaft = kurbo::BezPath::new();
    shaft.move_to((tip.x, START.bottom()));
    // The shaft stops where the head begins, or the head's sloping sides
    // leave slivers of background showing through the join.
    shaft.line_to((tip.x, tip.y - head));
    scene.stroke(
        &kurbo::Stroke::new(CONNECTOR_WEIGHT),
        scale,
        px(white),
        None,
        &shaft,
    );

    if let Some(marker) = dc_geom::endpoint_marker(
        &EndpointStyle::Arrow,
        dc_model::Point::new(tip.x, tip.y),
        // Pointing away from the shape it arrives at, which is upward.
        Vec2::new(0.0, -1.0),
        CONNECTOR_WEIGHT,
    ) {
        scene.fill(FillRule::NonZero, scale, px(white), None, &marker.path);
    }
}

/// The icon as a PNG, built into the binary.
///
/// Rendering it at startup instead would mean asking for a GPU device before
/// the window exists, which is slower and can fail on a machine that has no
/// graphics hardware to spare. The file is regenerated with
/// `diagramcraft icon`, and a test checks it still matches what this module
/// draws.
pub const EMBEDDED_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// The embedded icon decoded to RGBA, with its side in pixels.
///
/// `None` rather than an error: an application that cannot decode its own
/// icon should still open, just without one.
pub fn decoded() -> Option<(Vec<u8>, u32)> {
    // The decoder needs to seek, which a byte slice cannot do on its own.
    let decoder = png::Decoder::new(std::io::Cursor::new(EMBEDDED_PNG));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;

    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        log::warn!("the embedded icon is not 8-bit RGBA; skipping it");
        return None;
    }
    buffer.truncate(info.buffer_size());
    Some((buffer, info.width))
}

fn px(c: Color) -> peniko::Color {
    peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_encodes_geometry_at_every_size_an_icon_set_needs() {
        // macOS alone asks for six sizes, and the smallest is where a drawn
        // icon is most likely to collapse to nothing.
        for size in [16.0, 32.0, 64.0, 128.0, 256.0, 512.0, 1024.0] {
            let mut scene = Scene::new();
            draw(&mut scene, size);
            assert!(
                scene.encoding().n_paths >= 4,
                "{size}px encoded only {} paths",
                scene.encoding().n_paths
            );
        }
    }

    #[test]
    fn the_composition_sits_within_the_plate() {
        // The shapes and the connector are positioned by hand; this keeps a
        // later edit from pushing one past the rounded square's edge.
        let plate = kurbo::Rect::new(MARGIN, MARGIN, CANVAS - MARGIN, CANVAS - MARGIN);
        for bounds in [START, DECISION] {
            let r = kurbo::Rect::new(bounds.x, bounds.y, bounds.right(), bounds.bottom());
            assert_eq!(plate.union(r), plate, "{r:?} escapes the plate {plate:?}");
        }
    }

    #[test]
    fn the_embedded_icon_decodes_to_square_rgba() {
        let (pixels, size) = decoded().expect("the built-in icon must decode");
        assert_eq!(size, 256, "the embedded icon is the 256px one");
        assert_eq!(pixels.len(), (size * size * 4) as usize);
    }

    #[test]
    fn the_embedded_icon_has_transparent_corners_and_a_coloured_middle() {
        // This is what tells an icon from a plain square: the desktop shows
        // through the corners. It also catches the file being regenerated at
        // the wrong size or without an alpha channel.
        let (pixels, size) = decoded().expect("the built-in icon must decode");
        let at = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3])
        };

        assert_eq!(at(0, 0).3, 0, "the top-left corner must be transparent");
        assert_eq!(at(size - 1, size - 1).3, 0, "and the bottom-right");

        let (_, _, _, alpha) = at(size / 2, size / 2);
        assert_eq!(alpha, 255, "the middle must be opaque");
    }

    #[test]
    fn the_embedded_icon_matches_what_this_module_draws() {
        // The file is generated from `draw`; if someone edits the composition
        // without regenerating it, the icon in the dock stops matching the
        // icon in the repository. Comparing the plate's colour catches that
        // without re-rasterising at test time.
        let (pixels, size) = decoded().expect("the built-in icon must decode");
        let middle = {
            let i = (((size / 2) * size + size / 2) * 4) as usize;
            (pixels[i], pixels[i + 1], pixels[i + 2])
        };
        // Dead centre of the composition is the white connector or a shape.
        assert_eq!(middle, (255, 255, 255), "the middle of the icon is the flowchart");

        // A point inside the plate but clear of the drawing is the gradient,
        // which must fall between the two declared stops.
        let i = (((size / 2) * size + size / 8) * 4) as usize;
        let (r, g, b, a) = (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]);
        assert_eq!(a, 255);
        assert!(
            r >= BOTTOM.r.min(TOP.r) && r <= BOTTOM.r.max(TOP.r),
            "red {r} is outside the gradient"
        );
        assert!(b > r && b > g, "the plate is indigo: blue leads");
    }

    #[test]
    fn the_composition_is_centred_horizontally() {
        // An icon that is off-centre by a few units reads as a mistake.
        for bounds in [START, DECISION] {
            assert!(
                (bounds.center().x - CANVAS / 2.0).abs() < 1.0,
                "{bounds:?} is not centred"
            );
        }
    }
}
