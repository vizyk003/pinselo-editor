// SPDX-License-Identifier: Apache-2.0
//! Pinselo geometry — the Rust port of `@oc/geometry` (see docs/adr/0001-rust-core.md).
//!
//! Pure, allocation-light functions over `oc-schema` types: affine matrices, parametric shape outlines (SVG path
//! data), element world transforms and hit-testing, smart-guide snapping, align/distribute, connectors, frame
//! outlines and line markers. Results match the TS implementation (verified against goldens generated from it in
//! `tests/golden/geometry/`): numbers within 1e-9, path strings byte-for-byte. No I/O, threads or clocks, so the
//! crate builds for `wasm32-unknown-unknown`.

pub mod align;
pub mod boolean;
pub mod connectors;
pub mod frames;
pub mod jsmath;
pub mod markers;
pub mod matrix;
pub mod scene;
pub mod shapes;
pub mod snap;
pub mod vector;

pub use align::{AlignEdge, AlignItem, Deltas, align_deltas, distribute_deltas};
pub use connectors::{
    ANCHORS, ConnectorGeometry, NearestAnchor, anchor_point, connector_geometry, connector_local_points,
    connector_page_points, connectors_of, nearest_anchor, route_connector,
};
pub use frames::{OverlayPath, frame_el_outline, frame_outline, frame_overlays};
pub use markers::{MarkerMode, MarkerShape, line_marker_shape, trim_line_for_markers};
pub use matrix::{
    IDENTITY, apply, apply_vector, bounds_of_points, invert, local_matrix, multiply, normalize_angle, rect_contains,
    rect_corners, rects_intersect, rotate, scale, translate, union_rects, xf_from_matrix,
};
pub use scene::{
    HitTestOpts, hit_element, hit_test, marquee_select, parent_inverse, selection_bounds, world_bounds, world_matrix,
    world_quad,
};
pub use shapes::{Params, hit_shape, scale_unit_path, shape_path};
pub use boolean::{BoolOp, boolean, even_odd_to_nonzero};
pub use vector::{NodePart, NodeRef, PathNode, SubPath, VectorPath};
pub use snap::{Axis, Guide, GuideKind, RulerGuide, SnapResult, SnapValue, SpacingMark, snap_angle, snap_move, snap_spacing, snap_value};
