//! The backend-neutral drawing list a vector export is written from.
//!
//! Both writers need the same traversal: walk a page in paint order, turn
//! each shape and connector into paths and text, and hand the result to a
//! serializer. Doing that once here keeps SVG and PDF from drifting apart,
//! and keeps both of them honest about where the drawing actually comes
//! from — [`dc_geom::geometry`] and the model's resolved-style accessors are
//! the same source the on-screen renderer draws from.
//!
//! Text is laid out through the same Parley engine the canvas uses, so a
//! label breaks onto a second line in an exported file exactly where it
//! breaks on screen. Each line is emitted as a piece of real text with a
//! position, not as outlines, so the result stays selectable.

use dc_geom::{MarkerFill, RoutedLine};
use dc_model::{BoundingBox, Color, Page, Shape, ShapeType, StrokeStyle};
use dc_render::text::{PositionedRun, TextEngine};
use kurbo::{Affine, BezPath, Rect};

/// How a path is painted.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
    pub fill: Option<Color>,
    pub stroke: Option<Stroke>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f64,
    /// On/off run lengths, already scaled for the stroke width.
    pub dashes: Option<Vec<f64>>,
}

/// The three families the editor offers, which is also what every PDF
/// reader can be relied on to have without a font being embedded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Serif,
    Mono,
}

impl FontFamily {
    /// Classifies a requested family name.
    ///
    /// The format names Liberation Sans by default, and a document that came
    /// from elsewhere may name anything at all; an unrecognised family is
    /// sans, which is what the canvas falls back to as well.
    pub fn classify(name: &str) -> FontFamily {
        let lower = name.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
        if has(&["mono", "courier", "consol", "menlo"]) {
            FontFamily::Mono
        } else if lower.contains("sans") {
            FontFamily::Sans
        } else if has(&["serif", "times", "georgia", "garamond", "book"]) {
            FontFamily::Serif
        } else {
            FontFamily::Sans
        }
    }
}

/// One drawing instruction.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Path {
        path: BezPath,
        paint: Paint,
        /// Transform to apply, carrying a rotated shape's rotation.
        transform: Affine,
        /// Alpha from the shape's opacity, applied by the writer.
        alpha: f64,
    },
    Text {
        run: PositionedRun,
        transform: Affine,
        alpha: f64,
        /// The box to confine the text to, when the label has outgrown its
        /// shape. `None` is the ordinary case, where it fits.
        clip: Option<BoundingBox>,
    },
    /// A placeholder for an image, which neither writer embeds yet. Drawn as
    /// a flat rectangle so the export is not silently missing the block.
    ImagePlaceholder {
        bounds: BoundingBox,
        transform: Affine,
        alpha: f64,
    },
}

/// A page flattened into drawing instructions, with the region they occupy.
#[derive(Debug)]
pub struct Drawing {
    pub items: Vec<Item>,
    /// The region to frame, in document units.
    pub region: BoundingBox,
    pub background: Color,
}

/// Margin left around the content of a page framed to its contents, matching
/// what the PNG export uses so the three formats frame alike.
pub const MARGIN: f64 = 24.0;

/// Builds the drawing list for one page.
pub fn build(page: &Page, text: &mut TextEngine) -> Drawing {
    let region = page
        .content_bounds()
        .map(|b| b.inflated(MARGIN))
        .or_else(|| {
            page.resolved_settings()
                .bounds_points(dc_model::Units::default())
                .map(|(w, h)| BoundingBox::new(0.0, 0.0, w, h))
        })
        .unwrap_or(BoundingBox::new(0.0, 0.0, 612.0, 792.0));

    let mut items = Vec::new();

    // Connectors are drawn under the shapes they join, the same order the
    // canvas paints in.
    for line in page.lines() {
        let Some(route) = dc_geom::route_line(line, page) else {
            continue;
        };
        push_line(&mut items, line, &route, page, text);
    }

    for shape in page.shapes_in_paint_order() {
        push_shape(&mut items, shape, text);
    }

    Drawing { items, region, background: page.resolved_settings().resolved_fill() }
}

fn shape_transform(shape: &Shape) -> Affine {
    let rotation = shape.bounding_box.rotation_degrees();
    if rotation == 0.0 {
        Affine::IDENTITY
    } else {
        let c = shape.bounding_box.center();
        Affine::rotate_about(rotation.to_radians(), kurbo::Point::new(c.x, c.y))
    }
}

fn push_shape(items: &mut Vec<Item>, shape: &Shape, text: &mut TextEngine) {
    let alpha = shape.opacity_factor();
    if alpha <= 0.0 {
        return;
    }
    // A hotspot is an invisible click target; it has nothing to export.
    if shape.shape_type == ShapeType::Hotspot {
        return;
    }

    let geo = dc_geom::geometry(shape);
    let transform = shape_transform(shape);
    let style = shape.resolved_style();
    let stroke = shape.effective_stroke();

    let fill = if geo.filled {
        match style.resolved_fill() {
            dc_model::Fill::Color { color } => Some(color),
            // An image fill is not embedded; the block below stands in.
            dc_model::Fill::Image { .. } => None,
        }
    } else {
        None
    };

    let stroke_out = (!stroke.is_invisible() && geo.stroked).then(|| Stroke {
        color: stroke.resolved_color(),
        width: stroke.resolved_width(),
        dashes: stroke.resolved_style().dash_pattern(stroke.resolved_width()),
    });

    if fill.is_some() || stroke_out.is_some() {
        items.push(Item::Path {
            path: geo.outline.clone(),
            paint: Paint { fill, stroke: stroke_out },
            transform,
            alpha,
        });
    }

    let is_image = shape.shape_type == ShapeType::Image
        || matches!(style.resolved_fill(), dc_model::Fill::Image { .. });
    if is_image {
        items.push(Item::ImagePlaceholder {
            bounds: shape.bounding_box,
            transform,
            alpha,
        });
    }

    // Interior detail lines, which are stroked but never filled.
    if let Some(decoration) = &geo.decoration {
        if !stroke.is_invisible() {
            items.push(Item::Path {
                path: decoration.clone(),
                paint: Paint {
                    fill: None,
                    stroke: Some(Stroke {
                        color: stroke.resolved_color(),
                        width: stroke.resolved_width(),
                        dashes: stroke
                            .resolved_style()
                            .dash_pattern(stroke.resolved_width()),
                    }),
                },
                transform,
                alpha,
            });
        }
    }

    push_shape_text(items, shape, &geo, transform, alpha, text);
}

fn push_shape_text(
    items: &mut Vec<Item>,
    shape: &Shape,
    geo: &dc_geom::ShapeGeometry,
    transform: Affine,
    alpha: f64,
    text: &mut TextEngine,
) {
    let raw = shape.text_str();
    if raw.is_empty() || !shape.shape_type.accepts_text() {
        return;
    }

    let mut base = dc_model::TextStyle::default();
    base.color = shape.resolved_style().resolved_text_color();
    let parsed = dc_model::markup::parse(raw, base);

    let bb = shape.bounding_box;
    let i = geo.text_insets;
    let area = Rect::new(bb.x + i.x0, bb.y + i.y0, bb.right() - i.x1, bb.bottom() - i.y1);
    if area.width() <= 1.0 || area.height() <= 0.0 {
        return;
    }

    let block = text.layout(&parsed, Some(area.width()));

    // Text taller than its area is confined to the shape, exactly as the
    // canvas confines it: an exported file must not spill a label over its
    // neighbours when the editor does not.
    let clip = (block.height() > area.height()).then_some(bb);

    for run in block.runs(area) {
        items.push(Item::Text { run, transform, alpha, clip });
    }
}

fn push_line(
    items: &mut Vec<Item>,
    line: &dc_model::Line,
    route: &RoutedLine,
    page: &Page,
    text: &mut TextEngine,
) {
    let stroke = line.resolved_stroke();
    let color = stroke.resolved_color();
    let width = stroke.resolved_width();

    if !stroke.is_invisible() {
        items.push(Item::Path {
            path: route.path.clone(),
            paint: Paint {
                fill: None,
                stroke: Some(Stroke {
                    color,
                    width,
                    dashes: stroke.resolved_style().dash_pattern(width),
                }),
            },
            transform: Affine::IDENTITY,
            alpha: 1.0,
        });
    }

    let background = page.resolved_settings().resolved_fill();
    for (endpoint, tip, dir) in [
        (&line.endpoint1, route.start, -route.start_dir),
        (&line.endpoint2, route.end, -route.end_dir),
    ] {
        let Some(marker) = dc_geom::endpoint_marker(endpoint.style(), tip, -dir, width) else {
            continue;
        };
        // Hollow means the page shows through, so it takes the sheet colour
        // rather than being left unfilled.
        let fill = match marker.fill {
            MarkerFill::Solid => Some(color),
            MarkerFill::Hollow => Some(background),
            MarkerFill::Open => None,
        };
        items.push(Item::Path {
            path: marker.path,
            paint: Paint {
                fill,
                stroke: Some(Stroke { color, width, dashes: None }),
            },
            transform: Affine::IDENTITY,
            alpha: 1.0,
        });
    }

    push_line_labels(items, line, route, background, text);
}

fn push_line_labels(
    items: &mut Vec<Item>,
    line: &dc_model::Line,
    route: &RoutedLine,
    background: Color,
    text: &mut TextEngine,
) {
    for label in line.labels() {
        if label.text.trim().is_empty() {
            continue;
        }
        let anchor = route.point_at_fraction(label.resolved_position());
        let mut base = dc_model::TextStyle::default();
        base.color = background.contrasting_text();
        let parsed = dc_model::markup::parse(&label.text, base);
        let block = text.layout(&parsed, None);

        // Centred on the anchor, the way the canvas places a connector label.
        let area = Rect::new(
            anchor.x - block.width() / 2.0,
            anchor.y - block.height() / 2.0,
            anchor.x + block.width() / 2.0,
            anchor.y + block.height() / 2.0,
        );

        // A pad of sheet colour so the connector does not run through the
        // letters, matching what the canvas draws behind a label.
        let pad = 2.0;
        items.push(Item::Path {
            path: rect_path(
                area.x0 - pad,
                area.y0 - pad,
                area.x1 + pad,
                area.y1 + pad,
            ),
            paint: Paint { fill: Some(background), stroke: None },
            transform: Affine::IDENTITY,
            alpha: 1.0,
        });

        for run in block.runs(area) {
            // A connector label is sized to its own text, so it never needs
            // confining.
            items.push(Item::Text {
                run,
                transform: Affine::IDENTITY,
                alpha: 1.0,
                clip: None,
            });
        }
    }
}

/// A closed rectangle as a path, which both writers can consume.
fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x0, y0));
    p.line_to((x1, y0));
    p.line_to((x1, y1));
    p.line_to((x0, y1));
    p.close_path();
    p
}

/// Converts a stroke style into the dash pattern a writer emits.
pub fn dashes_for(style: StrokeStyle, width: f64) -> Option<Vec<f64>> {
    style.dash_pattern(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_classified_from_their_names() {
        assert_eq!(FontFamily::classify("Liberation Sans"), FontFamily::Sans);
        assert_eq!(FontFamily::classify("Helvetica"), FontFamily::Sans);
        assert_eq!(FontFamily::classify("Times New Roman"), FontFamily::Serif);
        assert_eq!(FontFamily::classify("Georgia"), FontFamily::Serif);
        assert_eq!(FontFamily::classify("Courier New"), FontFamily::Mono);
        assert_eq!(FontFamily::classify("Menlo"), FontFamily::Mono);
    }

    #[test]
    fn sans_serif_is_sans_despite_holding_serif() {
        assert_eq!(FontFamily::classify("sans-serif"), FontFamily::Sans);
        assert_eq!(FontFamily::classify("DejaVu Sans Serif"), FontFamily::Sans);
    }

    #[test]
    fn an_unknown_family_falls_back_to_sans() {
        assert_eq!(FontFamily::classify("Wingdings"), FontFamily::Sans);
        assert_eq!(FontFamily::classify(""), FontFamily::Sans);
    }
}
