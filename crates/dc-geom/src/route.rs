//! Connector routing.
//!
//! Routing happens in three stages:
//!
//! 1. Resolve each endpoint to a document-space point and an outward direction.
//!    An endpoint attached to a shape without an explicit position picks the
//!    side facing its counterpart, which is the format's smart-line behavior.
//! 2. Build a route for the line's type. Elbow routes are generated as several
//!    candidate orthogonal polylines, scored against the other shapes on the
//!    page, and the cleanest one wins.
//! 3. Convert the route to a path, rounding elbow corners and pulling the ends
//!    back far enough to leave room for their terminators.

use dc_model::{Endpoint, Line, LineType, Page, Point, Shape};
use kurbo::{BezPath, Vec2};
use std::collections::HashMap;

/// How far a connector travels straight out of a shape before it may turn.
pub const DEFAULT_STUB: f64 = 18.0;
/// Corner radius applied to elbow bends.
pub const DEFAULT_CORNER_RADIUS: f64 = 5.0;
/// Clearance kept between a route and the shapes it avoids.
const OBSTACLE_MARGIN: f64 = 6.0;
/// Guard against cycles in line-to-line attachments.
const MAX_DEPTH: u32 = 8;

/// A cardinal direction, used for the outward normal at an attachment point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    pub fn to_vec2(self) -> Vec2 {
        match self {
            // +y is down, so north is negative y.
            Dir::North => Vec2::new(0.0, -1.0),
            Dir::East => Vec2::new(1.0, 0.0),
            Dir::South => Vec2::new(0.0, 1.0),
            Dir::West => Vec2::new(-1.0, 0.0),
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Dir::East | Dir::West)
    }

    /// The direction pointing from `from` toward `to`, choosing whichever axis
    /// dominates the separation.
    pub fn toward(from: Point, to: Point) -> Dir {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        if dx.abs() >= dy.abs() {
            if dx >= 0.0 { Dir::East } else { Dir::West }
        } else if dy >= 0.0 {
            Dir::South
        } else {
            Dir::North
        }
    }
}

/// A resolved connector: its path plus the information terminators need.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutedLine {
    /// The stroked path, with ends already inset for terminators.
    pub path: BezPath,
    /// Route vertices in document space, before inset and corner rounding.
    pub points: Vec<Point>,
    /// The anchor point at endpoint 1, where its terminator tip sits.
    pub start: Point,
    /// The anchor point at endpoint 2.
    pub end: Point,
    /// Unit vector pointing out of the shape at endpoint 1.
    pub start_dir: Vec2,
    /// Unit vector pointing out of the shape at endpoint 2.
    pub end_dir: Vec2,
}

impl RoutedLine {
    /// The point a fraction of the way along the route, measured by arc length
    /// over the polyline. Used for labels and line-to-line attachments.
    pub fn point_at_fraction(&self, t: f64) -> Point {
        point_along(&self.points, t.clamp(0.0, 1.0))
    }

    /// The route direction at a fraction along it, for orienting labels.
    pub fn direction_at_fraction(&self, t: f64) -> Vec2 {
        let t = t.clamp(0.0, 1.0);
        let eps = 1e-3;
        let a = point_along(&self.points, (t - eps).max(0.0));
        let b = point_along(&self.points, (t + eps).min(1.0));
        let v = Vec2::new(b.x - a.x, b.y - a.y);
        if v.hypot() < 1e-9 { Vec2::new(1.0, 0.0) } else { v.normalize() }
    }

    pub fn total_length(&self) -> f64 {
        polyline_length(&self.points)
    }
}

/// The shapes a route should avoid.
#[derive(Clone, Debug, Default)]
pub struct Obstacles {
    boxes: Vec<kurbo::Rect>,
}

impl Obstacles {
    /// Collects every shape on the page except those named in `exclude`, which
    /// are the shapes the connector attaches to.
    pub fn from_page(page: &Page, exclude: &[&str]) -> Self {
        let boxes = page
            .shapes()
            .iter()
            .filter(|s| !exclude.contains(&s.id.as_str()))
            .filter(|s| s.bounding_box.w > 0.0 && s.bounding_box.h > 0.0)
            .map(|s| {
                let bb = s.bounding_box.aabb().inflated(OBSTACLE_MARGIN);
                kurbo::Rect::new(bb.x, bb.y, bb.x + bb.w, bb.y + bb.h)
            })
            .collect();
        Obstacles { boxes }
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// How many obstacles a polyline passes through.
    fn crossings(&self, points: &[Point]) -> usize {
        let mut count = 0;
        for w in points.windows(2) {
            for b in &self.boxes {
                if segment_hits_rect(w[0], w[1], b) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Coordinates of the clear corridors just outside each obstacle.
    ///
    /// Detours at a fixed distance fail against a tall or wide obstacle, so
    /// candidate routes are also generated along these lines, which are
    /// guaranteed to clear the obstacle that produced them.
    fn corridors(&self) -> (Vec<f64>, Vec<f64>) {
        const CLEARANCE: f64 = 2.0;
        // Bounded so a dense page cannot blow up the candidate count.
        const MAX_OBSTACLES: usize = 8;

        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for b in self.boxes.iter().take(MAX_OBSTACLES) {
            xs.push(b.x0 - CLEARANCE);
            xs.push(b.x1 + CLEARANCE);
            ys.push(b.y0 - CLEARANCE);
            ys.push(b.y1 + CLEARANCE);
        }
        (xs, ys)
    }
}

/// Routes the connectors on a page.
pub struct Router<'a> {
    page: &'a Page,
    /// Straight run out of a shape before the first turn.
    pub stub: f64,
    /// Radius applied to elbow bends.
    pub corner_radius: f64,
    /// Whether elbow routes should steer around other shapes.
    pub avoid_obstacles: bool,
}

impl<'a> Router<'a> {
    pub fn new(page: &'a Page) -> Self {
        Router {
            page,
            stub: DEFAULT_STUB,
            corner_radius: DEFAULT_CORNER_RADIUS,
            avoid_obstacles: true,
        }
    }

    /// Routes one line. Returns `None` when an endpoint cannot be resolved,
    /// which happens for a connector pointing at a shape that is not present.
    pub fn route(&self, line: &Line) -> Option<RoutedLine> {
        self.route_inner(line, 0)
    }

    /// Routes every line on the page, skipping those that cannot resolve.
    pub fn route_all(&self) -> HashMap<String, RoutedLine> {
        self.page
            .lines()
            .iter()
            .filter_map(|l| self.route(l).map(|r| (l.id.clone(), r)))
            .collect()
    }

    fn route_inner(&self, line: &Line, depth: u32) -> Option<RoutedLine> {
        if depth > MAX_DEPTH {
            return None;
        }

        // Each end needs the other's position to choose a facing side, so
        // resolve raw positions first, then refine.
        let raw1 = self.endpoint_position(&line.endpoint1, depth)?;
        let raw2 = self.endpoint_position(&line.endpoint2, depth)?;

        let (start, start_dir) = self.refine(&line.endpoint1, raw1, raw2);
        let (end, end_dir) = self.refine(&line.endpoint2, raw2, start);

        let exclude: Vec<&str> = [line.endpoint1.shape_id(), line.endpoint2.shape_id()]
            .into_iter()
            .flatten()
            .collect();

        let joints: Vec<Point> = line.joints.clone().unwrap_or_default();

        let points = match line.line_type {
            LineType::Straight => {
                let mut v = vec![start];
                v.extend(joints.iter().copied());
                v.push(end);
                v
            }
            LineType::Curved => {
                let mut v = vec![start];
                v.extend(joints.iter().copied());
                v.push(end);
                v
            }
            LineType::Elbow => {
                let obstacles = if self.avoid_obstacles {
                    Obstacles::from_page(self.page, &exclude)
                } else {
                    Obstacles::default()
                };
                if joints.is_empty() {
                    self.elbow_route(start, start_dir, end, end_dir, &obstacles)
                } else {
                    self.elbow_route_via(start, start_dir, end, end_dir, &joints, &obstacles)
                }
            }
        };

        let points = dedupe(points);
        let stroke_width = line.resolved_stroke().resolved_width();
        let path = self.build_path(line, &points, start_dir, end_dir, stroke_width);

        Some(RoutedLine {
            path,
            points,
            start,
            end,
            start_dir: start_dir.to_vec2(),
            end_dir: end_dir.to_vec2(),
        })
    }

    /// The unrefined position of an endpoint.
    fn endpoint_position(&self, ep: &Endpoint, depth: u32) -> Option<Point> {
        match ep {
            Endpoint::PositionEndpoint { position, .. } => Some(*position),
            Endpoint::ShapeEndpoint { shape_id, position, .. } => {
                let shape = self.page.shape(shape_id)?;
                Some(match position {
                    Some(rel) => shape.bounding_box.point_at(*rel),
                    // Start from the center; `refine` moves it to an edge.
                    None => shape.bounding_box.center(),
                })
            }
            Endpoint::LineEndpoint { line_id, position, .. } => {
                let other = self.page.line(line_id)?;
                if other.id == self.page.line(line_id)?.id && depth >= MAX_DEPTH {
                    return None;
                }
                let routed = self.route_inner(other, depth + 1)?;
                Some(routed.point_at_fraction(*position))
            }
        }
    }

    /// Moves a shape-attached endpoint onto the edge facing its counterpart and
    /// reports the outward direction there.
    fn refine(&self, ep: &Endpoint, here: Point, toward: Point) -> (Point, Dir) {
        match ep {
            Endpoint::ShapeEndpoint { shape_id, position, .. } => {
                let Some(shape) = self.page.shape(shape_id) else {
                    return (here, Dir::toward(here, toward));
                };
                match position {
                    // An explicit position is honored; only the direction is
                    // inferred, from which edge the point sits nearest.
                    Some(_) => (here, outward_dir_at(shape, here)),
                    // No position means smart attachment: pick the facing side.
                    None => best_facing_anchor(shape, toward),
                }
            }
            _ => (here, Dir::toward(here, toward)),
        }
    }

    /// Routes orthogonally from `start` to `end` by way of user waypoints,
    /// splicing a route for each leg.
    fn elbow_route_via(
        &self,
        start: Point,
        start_dir: Dir,
        end: Point,
        end_dir: Dir,
        joints: &[Point],
        obstacles: &Obstacles,
    ) -> Vec<Point> {
        let mut points = vec![start];
        let mut cursor = start;
        let mut cursor_dir = start_dir;

        // Every waypoint, then the destination.
        let targets: Vec<Point> = joints.iter().copied().chain(std::iter::once(end)).collect();

        for (i, target) in targets.iter().enumerate() {
            let is_last = i + 1 == targets.len();

            // The leg arrives at a waypoint head-on, so the waypoint's outward
            // direction is the reverse of the approach.
            let approach = Dir::toward(cursor, *target);
            let target_dir = if is_last { end_dir } else { opposite(approach) };

            let leg = self.elbow_route(cursor, cursor_dir, *target, target_dir, obstacles);
            points.extend(leg.into_iter().skip(1));

            if !is_last {
                let next = targets[i + 1];
                let mut depart = Dir::toward(*target, next);
                // Departing back along the approach would retrace the leg just
                // drawn. Leave along the perpendicular instead, which produces
                // the conventional down-across-up detour through a waypoint.
                if depart == opposite(approach) {
                    depart = perpendicular_toward(approach, *target, next);
                }
                cursor_dir = depart;
                cursor = *target;
            }
        }

        points
    }

    /// Generates candidate orthogonal routes and returns the best-scoring one.
    fn elbow_route(
        &self,
        start: Point,
        start_dir: Dir,
        end: Point,
        end_dir: Dir,
        obstacles: &Obstacles,
    ) -> Vec<Point> {
        let s1 = offset(start, start_dir, self.stub);
        let e1 = offset(end, end_dir, self.stub);

        let mid_x = (s1.x + e1.x) / 2.0;
        let mid_y = (s1.y + e1.y) / 2.0;

        // Each candidate connects the two stub tips with right angles.
        let candidates: Vec<Vec<Point>> = vec![
            // Horizontal first, then vertical.
            vec![s1, Point::new(e1.x, s1.y), e1],
            // Vertical first, then horizontal.
            vec![s1, Point::new(s1.x, e1.y), e1],
            // Split the horizontal gap, giving a Z.
            vec![
                s1,
                Point::new(mid_x, s1.y),
                Point::new(mid_x, e1.y),
                e1,
            ],
            // Split the vertical gap.
            vec![
                s1,
                Point::new(s1.x, mid_y),
                Point::new(e1.x, mid_y),
                e1,
            ],
        ];

        // Routes that travel along a corridor beside a blocking shape. These
        // are what let a connector get around an obstacle that spans the
        // direct path on both axes.
        let mut candidates = candidates;
        let (corridor_xs, corridor_ys) = obstacles.corridors();
        for y in corridor_ys {
            candidates.push(vec![
                s1,
                Point::new(s1.x, y),
                Point::new(e1.x, y),
                e1,
            ]);
        }
        for x in corridor_xs {
            candidates.push(vec![
                s1,
                Point::new(x, s1.y),
                Point::new(x, e1.y),
                e1,
            ]);
        }

        let mut best: Option<(f64, Vec<Point>)> = None;
        for candidate in candidates {
            let mut full = vec![start];
            full.extend(candidate);
            full.push(end);
            let full = dedupe(full);

            let score = self.score(&full, start_dir, end_dir, obstacles);
            if best.as_ref().is_none_or(|(b, _)| score < *b) {
                best = Some((score, full));
            }
        }

        best.map(|(_, p)| p).unwrap_or_else(|| vec![start, end])
    }

    /// Lower is better. Obstacle crossings dominate, then bends, then length.
    fn score(&self, points: &[Point], start_dir: Dir, end_dir: Dir, obstacles: &Obstacles) -> f64 {
        let crossings = obstacles.crossings(points) as f64;
        let bends = count_bends(points) as f64;
        let length = polyline_length(points);

        // A route that leaves the shape across the wrong axis looks wrong even
        // when it is short, so mismatches are penalized heavily.
        let mut mismatch = 0.0;
        if points.len() >= 2 {
            let first = Vec2::new(points[1].x - points[0].x, points[1].y - points[0].y);
            if !axis_matches(first, start_dir) {
                mismatch += 1.0;
            }
            let n = points.len();
            let last = Vec2::new(points[n - 2].x - points[n - 1].x, points[n - 2].y - points[n - 1].y);
            if !axis_matches(last, end_dir) {
                mismatch += 1.0;
            }
        }

        crossings * 1000.0 + mismatch * 400.0 + bends * 12.0 + length * 0.02
    }

    /// Converts a route to a path, applying corner rounding and end inset.
    fn build_path(
        &self,
        line: &Line,
        points: &[Point],
        start_dir: Dir,
        end_dir: Dir,
        stroke_width: f64,
    ) -> BezPath {
        let inset_start = line.endpoint1.style().inset_factor() * stroke_width;
        let inset_end = line.endpoint2.style().inset_factor() * stroke_width;

        let mut pts = points.to_vec();
        if pts.len() >= 2 {
            if inset_start > 0.0 {
                pts[0] = offset(pts[0], start_dir, inset_start);
            }
            let n = pts.len() - 1;
            if inset_end > 0.0 {
                pts[n] = offset(pts[n], end_dir, inset_end);
            }
        }
        let pts = dedupe(pts);

        match line.line_type {
            LineType::Straight => polyline_path(&pts),
            LineType::Elbow => rounded_polyline_path(&pts, self.corner_radius),
            LineType::Curved => curved_path(&pts, start_dir, end_dir, self.stub),
        }
    }
}

/// Convenience wrapper that routes a single line against its page.
pub fn route_line(line: &Line, page: &Page) -> Option<RoutedLine> {
    Router::new(page).route(line)
}

// ---------------------------------------------------------------------------
// Attachment helpers
// ---------------------------------------------------------------------------

/// The axis perpendicular to `axis`, oriented from `from` toward `to`.
fn perpendicular_toward(axis: Dir, from: Point, to: Point) -> Dir {
    if axis.is_horizontal() {
        if to.y >= from.y { Dir::South } else { Dir::North }
    } else if to.x >= from.x {
        Dir::East
    } else {
        Dir::West
    }
}

fn opposite(d: Dir) -> Dir {
    match d {
        Dir::North => Dir::South,
        Dir::South => Dir::North,
        Dir::East => Dir::West,
        Dir::West => Dir::East,
    }
}

/// The four edge midpoints of a shape in local space, with their outward
/// normals before rotation.
const EDGE_ANCHORS: [((f64, f64), (f64, f64)); 4] = [
    ((0.5, 0.0), (0.0, -1.0)), // north
    ((1.0, 0.5), (1.0, 0.0)),  // east
    ((0.5, 1.0), (0.0, 1.0)),  // south
    ((0.0, 0.5), (-1.0, 0.0)), // west
];

fn rotate_vec(v: Vec2, degrees: f64) -> Vec2 {
    if degrees == 0.0 {
        return v;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

/// The cardinal direction closest to an arbitrary vector. Orthogonal routing
/// needs a cardinal axis, so a rotated shape's true normal is snapped here.
fn snap_to_dir(v: Vec2) -> Dir {
    if v.x.abs() >= v.y.abs() {
        if v.x >= 0.0 { Dir::East } else { Dir::West }
    } else if v.y >= 0.0 {
        Dir::South
    } else {
        Dir::North
    }
}

/// Picks the edge of a shape that faces `toward`, returning its midpoint in
/// document space and the cardinal direction of its outward normal.
///
/// Choosing by rotated normal rather than by local axis matters for rotated
/// shapes: the edge a viewer sees facing the target is not the shape's own
/// east edge once it has been turned.
fn best_facing_anchor(shape: &Shape, toward: Point) -> (Point, Dir) {
    use dc_model::RelPoint;
    let bb = shape.bounding_box;
    let rotation = bb.rotation_degrees();

    let mut best: Option<(f64, Point, Dir)> = None;
    for ((rx, ry), (nx, ny)) in EDGE_ANCHORS {
        let anchor = bb.point_at(RelPoint::new(rx, ry));
        let normal = rotate_vec(Vec2::new(nx, ny), rotation);
        let to_target = Vec2::new(toward.x - anchor.x, toward.y - anchor.y);

        // How well this edge's normal points at the target.
        let score = if to_target.hypot() < 1e-9 {
            0.0
        } else {
            to_target.normalize().dot(normal)
        };

        if best.as_ref().is_none_or(|(b, _, _)| score > *b) {
            best = Some((score, anchor, snap_to_dir(normal)));
        }
    }

    let (_, point, dir) = best.expect("EDGE_ANCHORS is never empty");
    (point, dir)
}

/// Which edge a point sits nearest, used to infer the outward direction for an
/// endpoint pinned to an explicit relative position.
fn outward_dir_at(shape: &Shape, p: Point) -> Dir {
    let bb = shape.bounding_box;
    let rotation = bb.rotation_degrees();
    // Compare edge distances in unrotated space, where they are meaningful.
    let local = if rotation != 0.0 { p.rotate_about(bb.center(), -rotation) } else { p };

    let distances = [
        ((local.y - bb.y).abs(), Vec2::new(0.0, -1.0)),
        ((bb.right() - local.x).abs(), Vec2::new(1.0, 0.0)),
        ((bb.bottom() - local.y).abs(), Vec2::new(0.0, 1.0)),
        ((local.x - bb.x).abs(), Vec2::new(-1.0, 0.0)),
    ];

    let nearest = distances
        .iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("four edges");

    // Rotate the local normal into document space before snapping.
    snap_to_dir(rotate_vec(nearest.1, rotation))
}

fn offset(p: Point, d: Dir, by: f64) -> Point {
    let v = d.to_vec2();
    Point::new(p.x + v.x * by, p.y + v.y * by)
}

fn axis_matches(v: Vec2, d: Dir) -> bool {
    if v.hypot() < 1e-9 {
        return true;
    }
    let horizontal = v.x.abs() > v.y.abs();
    if horizontal != d.is_horizontal() {
        return false;
    }
    // The sign must point outward too.
    let expected = d.to_vec2();
    (v.x * expected.x + v.y * expected.y) >= 0.0
}

// ---------------------------------------------------------------------------
// Polyline helpers
// ---------------------------------------------------------------------------

fn dedupe(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|l| l.distance_to(p) < 1e-6) {
            continue;
        }
        out.push(p);
    }
    // Drop points that sit on the straight line between their neighbors.
    let mut i = 1;
    while out.len() > 2 && i < out.len() - 1 {
        let (a, b, c) = (out[i - 1], out[i], out[i + 1]);
        let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if cross.abs() < 1e-6 {
            out.remove(i);
        } else {
            i += 1;
        }
    }
    out
}

fn count_bends(points: &[Point]) -> usize {
    points.len().saturating_sub(2)
}

fn polyline_length(points: &[Point]) -> f64 {
    points.windows(2).map(|w| w[0].distance_to(w[1])).sum()
}

fn point_along(points: &[Point], t: f64) -> Point {
    match points.len() {
        0 => Point::ORIGIN,
        1 => points[0],
        _ => {
            let total = polyline_length(points);
            if total <= 0.0 {
                return points[0];
            }
            let target = total * t;
            let mut walked = 0.0;
            for w in points.windows(2) {
                let seg = w[0].distance_to(w[1]);
                if walked + seg >= target {
                    let local = if seg > 0.0 { (target - walked) / seg } else { 0.0 };
                    return w[0].lerp(w[1], local);
                }
                walked += seg;
            }
            *points.last().unwrap()
        }
    }
}

fn polyline_path(points: &[Point]) -> BezPath {
    let mut p = BezPath::new();
    if let Some(first) = points.first() {
        p.move_to((first.x, first.y));
        for q in &points[1..] {
            p.line_to((q.x, q.y));
        }
    }
    p
}

/// A polyline whose interior corners are rounded, which is what gives elbow
/// connectors their finished look.
fn rounded_polyline_path(points: &[Point], radius: f64) -> BezPath {
    if points.len() < 3 || radius <= 0.0 {
        return polyline_path(points);
    }

    let mut p = BezPath::new();
    p.move_to((points[0].x, points[0].y));

    for i in 1..points.len() - 1 {
        let (prev, corner, next) = (points[i - 1], points[i], points[i + 1]);
        let in_len = prev.distance_to(corner);
        let out_len = corner.distance_to(next);
        // Never round more than half of either adjacent segment, or corners
        // would overlap on a tight route.
        let r = radius.min(in_len / 2.0).min(out_len / 2.0);
        if r <= 0.01 {
            p.line_to((corner.x, corner.y));
            continue;
        }

        let a = corner.lerp(prev, r / in_len);
        let b = corner.lerp(next, r / out_len);
        p.line_to((a.x, a.y));
        p.quad_to((corner.x, corner.y), (b.x, b.y));
    }

    let last = points[points.len() - 1];
    p.line_to((last.x, last.y));
    p
}

/// A smooth curve leaving each end along its outward normal.
fn curved_path(points: &[Point], start_dir: Dir, end_dir: Dir, stub: f64) -> BezPath {
    let mut p = BezPath::new();
    let Some(&first) = points.first() else {
        return p;
    };
    p.move_to((first.x, first.y));

    if points.len() == 2 {
        let (a, b) = (points[0], points[1]);
        // Scale the handles with separation so short connectors do not loop.
        let reach = (a.distance_to(b) * 0.4).max(stub);
        let c1 = offset(a, start_dir, reach);
        let c2 = offset(b, end_dir, reach);
        p.curve_to((c1.x, c1.y), (c2.x, c2.y), (b.x, b.y));
        return p;
    }

    // With waypoints, pass a smooth curve through them using midpoint
    // quadratics, which keeps the curve inside the control polygon.
    for i in 1..points.len() - 1 {
        let c = points[i];
        let next = points[i + 1];
        let mid = c.lerp(next, 0.5);
        p.quad_to((c.x, c.y), (mid.x, mid.y));
    }
    let last = points[points.len() - 1];
    let ctrl = points[points.len() - 2];
    p.quad_to((ctrl.x, ctrl.y), (last.x, last.y));
    p
}

/// Whether an axis-aligned or sloped segment passes through a rectangle.
fn segment_hits_rect(a: Point, b: Point, r: &kurbo::Rect) -> bool {
    // Trivial reject when the segment's own box misses the rectangle.
    let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
    let (min_y, max_y) = (a.y.min(b.y), a.y.max(b.y));
    if max_x < r.x0 || min_x > r.x1 || max_y < r.y0 || min_y > r.y1 {
        return false;
    }
    // Either endpoint inside is an immediate hit.
    if (a.x > r.x0 && a.x < r.x1 && a.y > r.y0 && a.y < r.y1)
        || (b.x > r.x0 && b.x < r.x1 && b.y > r.y0 && b.y < r.y1)
    {
        return true;
    }
    // Otherwise test against each edge.
    let corners = [
        (r.x0, r.y0, r.x1, r.y0),
        (r.x1, r.y0, r.x1, r.y1),
        (r.x1, r.y1, r.x0, r.y1),
        (r.x0, r.y1, r.x0, r.y0),
    ];
    corners.iter().any(|&(x0, y0, x1, y1)| {
        segments_cross(a, b, Point::new(x0, y0), Point::new(x1, y1))
    })
}

fn segments_cross(p1: Point, p2: Point, p3: Point, p4: Point) -> bool {
    let d = |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let d1 = d(p3, p4, p1);
    let d2 = d(p3, p4, p2);
    let d3 = d(p1, p2, p3);
    let d4 = d(p1, p2, p4);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{BoundingBox, EndpointStyle, RelPoint, ShapeType};
    use kurbo::ParamCurve as _;

    fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Shape {
        Shape::new(id, ShapeType::Rectangle, BoundingBox::new(x, y, w, h))
    }

    /// Two shapes side by side, 100pt apart horizontally.
    fn side_by_side() -> Page {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 200.0, 0.0, 100.0, 60.0));
        page
    }

    fn connect(page: &mut Page, from: &str, to: &str, t: LineType) -> Line {
        let line = Line::new(
            "l1",
            Endpoint::at_shape(from, EndpointStyle::None),
            Endpoint::at_shape(to, EndpointStyle::Arrow),
        )
        .with_line_type(t);
        page.add_line(line.clone());
        line
    }

    #[test]
    fn smart_attachment_picks_the_facing_edges() {
        let mut page = side_by_side();
        let line = connect(&mut page, "a", "b", LineType::Elbow);
        let r = route_line(&line, &page).unwrap();

        // `a` is left of `b`, so the connector should leave a's right edge and
        // arrive at b's left edge.
        assert!((r.start.x - 100.0).abs() < 0.01, "start was {:?}", r.start);
        assert!((r.start.y - 30.0).abs() < 0.01);
        assert!((r.end.x - 200.0).abs() < 0.01, "end was {:?}", r.end);
        assert_eq!(r.start_dir, Vec2::new(1.0, 0.0));
        assert_eq!(r.end_dir, Vec2::new(-1.0, 0.0));
    }

    #[test]
    fn smart_attachment_flips_when_shapes_swap_sides() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 200.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 0.0, 0.0, 100.0, 60.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);
        let r = route_line(&line, &page).unwrap();

        assert_eq!(r.start_dir, Vec2::new(-1.0, 0.0), "a now exits leftward");
        assert_eq!(r.end_dir, Vec2::new(1.0, 0.0));
    }

    #[test]
    fn vertical_stack_attaches_top_to_bottom() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 0.0, 200.0, 100.0, 60.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);
        let r = route_line(&line, &page).unwrap();

        assert_eq!(r.start_dir, Vec2::new(0.0, 1.0), "exits downward");
        assert!((r.start.y - 60.0).abs() < 0.01);
        assert_eq!(r.end_dir, Vec2::new(0.0, -1.0));
        assert!((r.end.y - 200.0).abs() < 0.01);
    }

    #[test]
    fn explicit_position_is_honored() {
        let mut page = side_by_side();
        let line = Line::new(
            "l1",
            Endpoint::ShapeEndpoint {
                style: EndpointStyle::None,
                shape_id: "a".into(),
                position: Some(RelPoint::new(0.5, 0.0)),
            },
            Endpoint::at_shape("b", EndpointStyle::Arrow),
        );
        page.add_line(line.clone());
        let r = route_line(&line, &page).unwrap();

        // Top-center of `a`, not the facing edge.
        assert!((r.start.x - 50.0).abs() < 0.01, "start was {:?}", r.start);
        assert!((r.start.y - 0.0).abs() < 0.01);
        assert_eq!(r.start_dir, Vec2::new(0.0, -1.0), "and it exits upward");
    }

    #[test]
    fn straight_route_is_a_direct_segment() {
        let mut page = side_by_side();
        let line = connect(&mut page, "a", "b", LineType::Straight);
        let r = route_line(&line, &page).unwrap();
        assert_eq!(r.points.len(), 2, "no intermediate vertices");
    }

    #[test]
    fn elbow_route_is_orthogonal() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 300.0, 400.0, 100.0, 60.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);
        let r = route_line(&line, &page).unwrap();

        assert!(r.points.len() > 2, "an offset pair needs bends");
        for w in r.points.windows(2) {
            let dx = (w[1].x - w[0].x).abs();
            let dy = (w[1].y - w[0].y).abs();
            assert!(
                dx < 0.01 || dy < 0.01,
                "segment {:?} -> {:?} is neither horizontal nor vertical",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn elbow_route_steers_around_a_blocking_shape() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 400.0, 0.0, 100.0, 60.0));
        // Directly between them, straddling the straight path.
        page.add_shape(rect("wall", 200.0, -40.0, 60.0, 140.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);

        let r = route_line(&line, &page).unwrap();
        let obstacles = Obstacles::from_page(&page, &["a", "b"]);
        assert_eq!(
            obstacles.crossings(&r.points),
            0,
            "route {:?} should clear the wall",
            r.points
        );
    }

    #[test]
    fn obstacle_avoidance_can_be_disabled() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 400.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("wall", 200.0, -40.0, 60.0, 140.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);

        let mut router = Router::new(&page);
        router.avoid_obstacles = false;
        let r = router.route(&line).unwrap();
        // The straight shot is shortest, so without avoidance it wins.
        assert_eq!(r.points.len(), 2, "got {:?}", r.points);
    }

    #[test]
    fn curved_route_produces_curve_segments() {
        let mut page = side_by_side();
        let line = connect(&mut page, "a", "b", LineType::Curved);
        let r = route_line(&line, &page).unwrap();

        let has_curve = r
            .path
            .segments()
            .any(|s| !matches!(s, kurbo::PathSeg::Line(_)));
        assert!(has_curve, "a curved connector must contain a curve");
    }

    #[test]
    fn elbow_corners_are_rounded() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        page.add_shape(rect("b", 300.0, 400.0, 100.0, 60.0));
        let line = connect(&mut page, "a", "b", LineType::Elbow);
        let r = route_line(&line, &page).unwrap();

        let has_quad = r
            .path
            .segments()
            .any(|s| matches!(s, kurbo::PathSeg::Quad(_)));
        assert!(has_quad, "elbow bends should be rounded");
    }

    #[test]
    fn unresolvable_endpoint_yields_no_route() {
        let mut page = Page::new("p", "P");
        page.add_shape(rect("a", 0.0, 0.0, 100.0, 60.0));
        let line = Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("ghost", EndpointStyle::Arrow),
        );
        page.add_line(line.clone());
        assert!(route_line(&line, &page).is_none());
    }

    #[test]
    fn line_attached_to_another_line_resolves() {
        let mut page = side_by_side();
        let base = Line::new(
            "base",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("b", EndpointStyle::None),
        )
        .with_line_type(LineType::Straight);
        page.add_line(base);

        let branch = Line::new(
            "branch",
            Endpoint::at_point(Point::new(150.0, 300.0), EndpointStyle::None),
            Endpoint::LineEndpoint {
                style: EndpointStyle::Arrow,
                line_id: "base".into(),
                position: 0.5,
            },
        )
        .with_line_type(LineType::Straight);
        page.add_line(branch.clone());

        let r = route_line(&branch, &page).unwrap();
        // The midpoint of a straight run from a's right edge to b's left edge.
        assert!((r.end.x - 150.0).abs() < 0.01, "end was {:?}", r.end);
        assert!((r.end.y - 30.0).abs() < 0.01);
    }

    #[test]
    fn mutually_attached_lines_terminate() {
        let mut page = Page::new("p", "P");
        for (id, other) in [("l1", "l2"), ("l2", "l1")] {
            page.add_line(Line::new(
                id,
                Endpoint::at_point(Point::new(0.0, 0.0), EndpointStyle::None),
                Endpoint::LineEndpoint {
                    style: EndpointStyle::None,
                    line_id: other.into(),
                    position: 0.5,
                },
            ));
        }
        // The depth cap must stop this rather than overflowing the stack.
        let line = page.line("l1").unwrap().clone();
        let _ = route_line(&line, &page);
    }

    #[test]
    fn route_all_skips_broken_lines_and_keeps_good_ones() {
        let mut page = side_by_side();
        connect(&mut page, "a", "b", LineType::Elbow);
        page.add_line(Line::new(
            "broken",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("ghost", EndpointStyle::None),
        ));

        let routes = Router::new(&page).route_all();
        assert!(routes.contains_key("l1"));
        assert!(!routes.contains_key("broken"));
    }

    #[test]
    fn terminator_inset_pulls_the_stroke_back() {
        let mut page = side_by_side();
        // An arrow at endpoint 2 should shorten the stroke there.
        let line = connect(&mut page, "a", "b", LineType::Straight);
        let r = route_line(&line, &page).unwrap();

        let stroke_end = r.path.segments().last().unwrap().end();
        assert!(
            stroke_end.x < r.end.x - 1.0,
            "stroke ended at {:?} but anchor is {:?}",
            stroke_end,
            r.end
        );
    }

    #[test]
    fn no_inset_when_there_is_no_terminator() {
        let mut page = side_by_side();
        let line = Line::new(
            "l1",
            Endpoint::at_shape("a", EndpointStyle::None),
            Endpoint::at_shape("b", EndpointStyle::None),
        )
        .with_line_type(LineType::Straight);
        page.add_line(line.clone());
        let r = route_line(&line, &page).unwrap();

        let stroke_end = r.path.segments().last().unwrap().end();
        assert!((stroke_end.x - r.end.x).abs() < 0.01);
    }

    #[test]
    fn joints_are_routed_through() {
        let mut page = side_by_side();
        let mut line = connect(&mut page, "a", "b", LineType::Elbow);
        line.joints = Some(vec![Point::new(150.0, 200.0)]);
        page.lines_mut()[0] = line.clone();

        let r = route_line(&line, &page).unwrap();
        // The joint may be absorbed into a straight run rather than kept as a
        // vertex, so test that the route passes through it geometrically.
        let joint = Point::new(150.0, 200.0);
        let nearest = r
            .points
            .windows(2)
            .map(|w| distance_to_segment(joint, w[0], w[1]))
            .fold(f64::MAX, f64::min);
        assert!(nearest < 1.0, "route {:?} misses the joint by {nearest}", r.points);
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

    #[test]
    fn point_at_fraction_walks_arc_length() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
        ];
        assert!(point_along(&points, 0.0).distance_to(Point::new(0.0, 0.0)) < 1e-9);
        assert!(point_along(&points, 0.5).distance_to(Point::new(100.0, 0.0)) < 1e-9);
        assert!(point_along(&points, 1.0).distance_to(Point::new(100.0, 100.0)) < 1e-9);
        assert!(point_along(&points, 0.25).distance_to(Point::new(50.0, 0.0)) < 1e-9);
    }

    #[test]
    fn dedupe_drops_duplicates_and_collinear_points() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(50.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 50.0),
        ];
        let out = dedupe(points);
        assert_eq!(out.len(), 3, "got {out:?}");
    }

    #[test]
    fn segment_rect_intersection_is_correct() {
        let r = kurbo::Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(segment_hits_rect(Point::new(-5.0, 5.0), Point::new(15.0, 5.0), &r), "straight through");
        assert!(segment_hits_rect(Point::new(5.0, 5.0), Point::new(15.0, 5.0), &r), "starts inside");
        assert!(!segment_hits_rect(Point::new(-5.0, 20.0), Point::new(15.0, 20.0), &r), "passes below");
        assert!(!segment_hits_rect(Point::new(-5.0, -5.0), Point::new(-1.0, -5.0), &r), "fully outside");
    }

    #[test]
    fn rotated_shape_attachment_lands_on_the_rotated_edge() {
        let mut page = Page::new("p", "P");
        let mut a = rect("a", 0.0, 0.0, 100.0, 100.0);
        a.bounding_box = a.bounding_box.with_rotation(90.0);
        page.add_shape(a);
        page.add_shape(rect("b", 300.0, 0.0, 100.0, 100.0));
        let line = connect(&mut page, "a", "b", LineType::Straight);
        let r = route_line(&line, &page).unwrap();

        // Rotating a square by 90 degrees leaves the same footprint, so the
        // east anchor is still the right edge midpoint.
        assert!((r.start.x - 100.0).abs() < 0.01, "start was {:?}", r.start);
        assert!((r.start.y - 50.0).abs() < 0.01);
    }
}
