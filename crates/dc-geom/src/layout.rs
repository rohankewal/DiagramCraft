//! Automatic arrangement of a connected diagram.
//!
//! A layered layout, which is what a flowchart and an org chart both want:
//! nodes are assigned to ranks by how far they sit from a start, ordered
//! within each rank to pull connected nodes near one another, and then
//! placed. A tree falls out as the case where every node has one parent, so
//! one algorithm serves both.
//!
//! The three stages are the usual ones:
//!
//! 1. **Rank.** Longest-path layering, so an edge always points from a lower
//!    rank to a higher one and nothing is drawn pointing backwards.
//! 2. **Order.** Repeated barycentre passes, which pull a node towards the
//!    average position of what it connects to and shortens crossings.
//! 3. **Place.** Each rank is laid out along an axis and centred, so the
//!    result is symmetric rather than piled against one edge.
//!
//! Everything here is pure: it takes sizes and edges and returns positions,
//! with no notion of a document, so it can be tested without building one.

use dc_model::Point;
use std::collections::HashMap;

/// A node to place, with the size it occupies.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode {
    pub id: String,
    pub w: f64,
    pub h: f64,
}

/// A directed connection between two nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    /// Whether ranks advance down the page. The alternative runs them across
    /// it, which suits a wide hierarchy better than a deep one.
    pub vertical: bool,
    /// Space between neighbours within a rank.
    pub gap_within: f64,
    /// Space between one rank and the next.
    pub gap_between: f64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions { vertical: true, gap_within: 40.0, gap_between: 60.0 }
    }
}

/// How many ordering passes to run.
///
/// The barycentre heuristic converges quickly and then wanders, so a small
/// fixed number beats running to a fixed point.
const ORDERING_PASSES: usize = 4;

/// Arranges nodes, returning the new top-left corner of each.
///
/// Nodes connected to nothing are gathered into a final rank rather than
/// left where they were, which would have the tidied diagram overlap them.
pub fn layered(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    options: LayoutOptions,
) -> Vec<(String, Point)> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Only edges between nodes being laid out count; one pointing outside the
    // set says nothing about where these should go.
    let links: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|e| {
            let from = *index.get(e.from.as_str())?;
            let to = *index.get(e.to.as_str())?;
            (from != to).then_some((from, to))
        })
        .collect();

    let ranks = assign_ranks(nodes.len(), &links);
    let orders = order_within_ranks(nodes.len(), &links, &ranks);
    place(nodes, &ranks, &orders, options)
}

/// Longest-path layering, tolerant of cycles.
///
/// Kahn's algorithm settles everything reachable without going round in a
/// circle. Whatever is left is in a cycle, and is broken into by taking the
/// node with the fewest unsatisfied predecessors — an arbitrary but stable
/// choice, which matters more than which node it picks.
fn assign_ranks(count: usize, links: &[(usize, usize)]) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];
    for (from, to) in links {
        out[*from].push(*to);
        indegree[*to] += 1;
    }

    let mut rank = vec![0usize; count];
    let mut remaining = indegree.clone();
    let mut settled = vec![false; count];
    let mut queue: Vec<usize> = (0..count).filter(|i| remaining[*i] == 0).collect();
    let mut done = 0;

    loop {
        while let Some(node) = queue.pop() {
            if settled[node] {
                continue;
            }
            settled[node] = true;
            done += 1;
            for &next in &out[node] {
                rank[next] = rank[next].max(rank[node] + 1);
                remaining[next] = remaining[next].saturating_sub(1);
                if remaining[next] == 0 && !settled[next] {
                    queue.push(next);
                }
            }
        }

        if done == count {
            break;
        }

        // A cycle. Take the most nearly satisfied node still waiting and let
        // it through, which unblocks the rest of the loop.
        let Some(next) = (0..count)
            .filter(|i| !settled[*i])
            .min_by_key(|i| (remaining[*i], *i))
        else {
            break;
        };
        remaining[next] = 0;
        queue.push(next);
    }

    rank
}

/// The order of nodes within each rank, as a list of node indices per rank.
fn order_within_ranks(
    count: usize,
    links: &[(usize, usize)],
    ranks: &[usize],
) -> Vec<Vec<usize>> {
    let depth = ranks.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for node in 0..count {
        rows[ranks[node]].push(node);
    }

    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (from, to) in links {
        predecessors[*to].push(*from);
        successors[*from].push(*to);
    }

    for pass in 0..ORDERING_PASSES {
        // Alternating direction lets information travel both ways through
        // the ranks instead of only downwards.
        let downward = pass % 2 == 0;
        let order: Vec<usize> = if downward {
            (1..depth).collect()
        } else {
            (0..depth.saturating_sub(1)).rev().collect()
        };

        for rank in order {
            let positions = position_lookup(&rows);
            let neighbours = if downward { &predecessors } else { &successors };
            let mut scored: Vec<(f64, usize, usize)> = rows[rank]
                .iter()
                .enumerate()
                .map(|(current, &node)| {
                    let mut total = 0.0;
                    let mut seen = 0;
                    for &other in &neighbours[node] {
                        if let Some(p) = positions.get(&other) {
                            total += *p as f64;
                            seen += 1;
                        }
                    }
                    // A node with nothing to be pulled towards keeps its
                    // place rather than collapsing to zero.
                    let barycentre = if seen > 0 {
                        total / seen as f64
                    } else {
                        current as f64
                    };
                    (barycentre, current, node)
                })
                .collect();

            // The current index breaks ties, which keeps the sort stable and
            // the result the same from one run to the next.
            scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            rows[rank] = scored.into_iter().map(|(_, _, node)| node).collect();
        }
    }

    rows
}

fn position_lookup(rows: &[Vec<usize>]) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    for row in rows {
        for (i, node) in row.iter().enumerate() {
            out.insert(*node, i);
        }
    }
    out
}

/// Turns ranks and orders into coordinates.
fn place(
    nodes: &[LayoutNode],
    ranks: &[usize],
    rows: &[Vec<usize>],
    options: LayoutOptions,
) -> Vec<(String, Point)> {
    let _ = ranks;
    let mut out = Vec::with_capacity(nodes.len());
    let mut along = 0.0;

    // The widest rank sets the centre line every other rank is centred on.
    let extent_of = |row: &Vec<usize>| -> f64 {
        if row.is_empty() {
            return 0.0;
        }
        let sizes: f64 = row
            .iter()
            .map(|i| if options.vertical { nodes[*i].w } else { nodes[*i].h })
            .sum();
        sizes + options.gap_within * (row.len() - 1) as f64
    };
    let widest = rows.iter().map(extent_of).fold(0.0, f64::max);

    for row in rows {
        if row.is_empty() {
            continue;
        }
        let thickness = row
            .iter()
            .map(|i| if options.vertical { nodes[*i].h } else { nodes[*i].w })
            .fold(0.0, f64::max);

        let mut across = (widest - extent_of(row)) / 2.0;
        for &node in row {
            let n = &nodes[node];
            let (size_across, size_along) =
                if options.vertical { (n.w, n.h) } else { (n.h, n.w) };

            // Centred within the rank's thickness, so a short node sits
            // level with a tall one rather than hanging from its top.
            let offset = (thickness - size_along) / 2.0;
            let point = if options.vertical {
                Point::new(across, along + offset)
            } else {
                Point::new(along + offset, across)
            };
            out.push((n.id.clone(), point));
            across += size_across + options.gap_within;
        }

        along += thickness + options.gap_between;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, w: f64, h: f64) -> LayoutNode {
        LayoutNode { id: id.into(), w, h }
    }

    fn edge(from: &str, to: &str) -> LayoutEdge {
        LayoutEdge { from: from.into(), to: to.into() }
    }

    fn at(result: &[(String, Point)], id: &str) -> Point {
        result.iter().find(|(i, _)| i == id).expect("a position").1
    }

    #[test]
    fn an_empty_input_lays_out_to_nothing() {
        assert!(layered(&[], &[], LayoutOptions::default()).is_empty());
    }

    #[test]
    fn a_chain_runs_down_the_page_in_order() {
        let nodes = vec![node("a", 100.0, 50.0), node("b", 100.0, 50.0), node("c", 100.0, 50.0)];
        let edges = vec![edge("a", "b"), edge("b", "c")];
        let result = layered(&nodes, &edges, LayoutOptions::default());

        assert_eq!(result.len(), 3);
        assert!(at(&result, "a").y < at(&result, "b").y);
        assert!(at(&result, "b").y < at(&result, "c").y);
        assert_eq!(at(&result, "a").x, at(&result, "b").x, "a chain stays in a column");
    }

    #[test]
    fn ranks_are_spaced_by_the_gap_between_them() {
        let nodes = vec![node("a", 100.0, 50.0), node("b", 100.0, 50.0)];
        let options = LayoutOptions { gap_between: 30.0, ..Default::default() };
        let result = layered(&nodes, &vec![edge("a", "b")], options);
        assert_eq!(at(&result, "b").y - at(&result, "a").y, 80.0, "height plus gap");
    }

    #[test]
    fn siblings_sit_side_by_side_and_centred_under_their_parent() {
        let nodes = vec![
            node("root", 100.0, 50.0),
            node("l", 100.0, 50.0),
            node("r", 100.0, 50.0),
        ];
        let edges = vec![edge("root", "l"), edge("root", "r")];
        let result = layered(&nodes, &edges, LayoutOptions::default());

        let (l, r, root) = (at(&result, "l"), at(&result, "r"), at(&result, "root"));
        assert_eq!(l.y, r.y, "siblings share a rank");
        assert!(l.y > root.y, "and sit below the parent");

        let midpoint = (l.x + r.x + 100.0) / 2.0;
        assert!(
            (midpoint - (root.x + 50.0)).abs() < 0.01,
            "the parent should be centred over its children"
        );
    }

    #[test]
    fn a_diamond_puts_the_join_below_both_branches() {
        // a → b → d, a → c → d
        let nodes = vec![
            node("a", 80.0, 40.0),
            node("b", 80.0, 40.0),
            node("c", 80.0, 40.0),
            node("d", 80.0, 40.0),
        ];
        let edges = vec![edge("a", "b"), edge("a", "c"), edge("b", "d"), edge("c", "d")];
        let result = layered(&nodes, &edges, LayoutOptions::default());

        let d = at(&result, "d");
        assert!(d.y > at(&result, "b").y, "the join is below its inputs");
        assert!(d.y > at(&result, "c").y);
        assert_eq!(at(&result, "b").y, at(&result, "c").y, "branches share a rank");
    }

    #[test]
    fn a_long_path_decides_the_rank_rather_than_the_short_one() {
        // a → b → c and a → c, so c must clear b rather than sit beside it.
        let nodes = vec![node("a", 80.0, 40.0), node("b", 80.0, 40.0), node("c", 80.0, 40.0)];
        let edges = vec![edge("a", "b"), edge("b", "c"), edge("a", "c")];
        let result = layered(&nodes, &edges, LayoutOptions::default());
        assert!(at(&result, "c").y > at(&result, "b").y);
    }

    #[test]
    fn a_cycle_is_laid_out_rather_than_hanging() {
        let nodes = vec![node("a", 80.0, 40.0), node("b", 80.0, 40.0), node("c", 80.0, 40.0)];
        let edges = vec![edge("a", "b"), edge("b", "c"), edge("c", "a")];
        let result = layered(&nodes, &edges, LayoutOptions::default());
        assert_eq!(result.len(), 3, "every node still gets a position");
    }

    #[test]
    fn a_self_edge_is_ignored_rather_than_ranking_a_node_below_itself() {
        let nodes = vec![node("a", 80.0, 40.0), node("b", 80.0, 40.0)];
        let edges = vec![edge("a", "a"), edge("a", "b")];
        let result = layered(&nodes, &edges, LayoutOptions::default());
        assert!(at(&result, "b").y > at(&result, "a").y);
    }

    #[test]
    fn an_edge_pointing_outside_the_set_is_ignored() {
        let nodes = vec![node("a", 80.0, 40.0), node("b", 80.0, 40.0)];
        let edges = vec![edge("a", "elsewhere"), edge("a", "b")];
        let result = layered(&nodes, &edges, LayoutOptions::default());
        assert_eq!(result.len(), 2);
        assert!(at(&result, "b").y > at(&result, "a").y);
    }

    #[test]
    fn unconnected_nodes_share_the_first_rank_rather_than_overlapping() {
        let nodes = vec![node("a", 80.0, 40.0), node("b", 80.0, 40.0), node("c", 80.0, 40.0)];
        let result = layered(&nodes, &[], LayoutOptions::default());
        let xs: Vec<f64> = ["a", "b", "c"].iter().map(|id| at(&result, id).x).collect();
        let mut sorted = xs.clone();
        sorted.sort_by(f64::total_cmp);
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "they must not land on top of one another");
    }

    #[test]
    fn the_horizontal_option_runs_ranks_across_instead_of_down() {
        let nodes = vec![node("a", 100.0, 50.0), node("b", 100.0, 50.0)];
        let options = LayoutOptions { vertical: false, ..Default::default() };
        let result = layered(&nodes, &vec![edge("a", "b")], options);
        assert!(at(&result, "b").x > at(&result, "a").x, "b should be to the right");
        assert_eq!(at(&result, "a").y, at(&result, "b").y, "and level with a");
    }

    #[test]
    fn differently_sized_nodes_are_centred_within_their_rank() {
        let nodes = vec![
            node("tall", 80.0, 100.0),
            node("short", 80.0, 20.0),
            node("root", 80.0, 40.0),
        ];
        let edges = vec![edge("root", "tall"), edge("root", "short")];
        let result = layered(&nodes, &edges, LayoutOptions::default());

        let tall = at(&result, "tall");
        let short = at(&result, "short");
        assert_eq!(
            tall.y + 50.0,
            short.y + 10.0,
            "their centres should line up, not their tops"
        );
    }

    #[test]
    fn the_result_is_the_same_every_run() {
        let nodes = vec![
            node("a", 80.0, 40.0),
            node("b", 80.0, 40.0),
            node("c", 80.0, 40.0),
            node("d", 80.0, 40.0),
        ];
        let edges = vec![edge("a", "b"), edge("a", "c"), edge("b", "d"), edge("c", "d")];
        let first = layered(&nodes, &edges, LayoutOptions::default());
        for _ in 0..5 {
            assert_eq!(layered(&nodes, &edges, LayoutOptions::default()), first);
        }
    }
}
