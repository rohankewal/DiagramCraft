//! Geometry for diagram documents: shape outlines, connector routing, and
//! hit testing.
//!
//! This crate is pure computation over [`dc_model`] types with no rendering
//! backend, which keeps it testable without a GPU.

pub mod layout;
pub mod markers;
pub mod route;
pub mod shapes;

pub use layout::{LayoutEdge, LayoutNode, LayoutOptions, layered};
pub use markers::{Marker, MarkerFill, endpoint_marker};
pub use route::{Dir, Obstacles, RoutedLine, Router, route_line};
pub use shapes::{ShapeGeometry, geometry};
