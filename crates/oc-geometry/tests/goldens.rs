// SPDX-License-Identifier: Apache-2.0
//! Replays the TS-generated pure-function goldens in tests/golden/geometry/ against the Rust port.

mod common;

use common::*;
use oc_geometry::*;
use oc_schema::elements::{FrameClip, FrameOverlay, LineMarker, LineRoute};
use oc_schema::primitives::{Mat2D, Rect, Vec2, Xf};
use serde_json::{Value, json};

fn mat(v: &Value) -> Mat2D {
    de(v)
}
fn rect(v: &Value) -> Rect {
    de(v)
}
fn v2(v: &Value) -> Vec2 {
    de(v)
}
fn deltas(d: Deltas) -> Value {
    Value::Array(d.into_iter().map(|(k, v)| json!([k, j(&v)])).collect())
}
fn axis(v: &Value) -> Axis {
    de(v)
}

#[test]
fn matrix() {
    replay("matrix", |name, a| match name {
        "invert" => j(&invert(&mat(&a[0]))),
        "apply" => j(&apply(&mat(&a[0]), v2(&a[1]))),
        "applyVector" => j(&apply_vector(&mat(&a[0]), v2(&a[1]))),
        "xfFromMatrix" => j(&xf_from_matrix(&mat(&a[0]), f(&a[1]), f(&a[2]))),
        "multiply" => j(&multiply(&mat(&a[0]), &mat(&a[1]))),
        "translate" => j(&translate(f(&a[0]), f(&a[1]))),
        "scale" => j(&scale(f(&a[0]), f(&a[1]))),
        "rotate" => j(&rotate(f(&a[0]))),
        "normalizeAngle" => j(&normalize_angle(f(&a[0]))),
        "localMatrix" => j(&local_matrix(&de::<Xf>(&a[0]))),
        "rectCorners" => j(&rect_corners(&rect(&a[0]))),
        "rectsIntersect" => j(&rects_intersect(&rect(&a[0]), &rect(&a[1]))),
        "rectContains" => j(&rect_contains(&rect(&a[0]), &rect(&a[1]))),
        "unionRects" => j(&union_rects(&de::<Vec<Rect>>(&a[0]))),
        "boundsOfPoints" => j(&bounds_of_points(&de::<Vec<Vec2>>(&a[0]))),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn shapes() {
    replay("shapes", |name, a| match name {
        "shapePath" => {
            let d = a.get(4).and_then(Value::as_str);
            let out = shape_path(s(&a[0]), f(&a[1]), f(&a[2]), &de::<Params>(&a[3]), d);
            // Deviation: unknown kinds give "" where the TS switch falls through to `undefined`.
            if out.is_empty() { Value::String(UNDEF.into()) } else { j(&out) }
        }
        "scaleUnitPath" => j(&scale_unit_path(s(&a[0]), f(&a[1]), f(&a[2]))),
        "hitShape" => j(&hit_shape(s(&a[0]), f(&a[1]), f(&a[2]), f(&a[3]), f(&a[4]), &de::<Params>(&a[5]))),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn snap() {
    replay("snap", |name, a| match name {
        "snapMove" => j(&snap_move(&rect(&a[0]), &rect(&a[1]), &de::<Vec<Rect>>(&a[2]), f(&a[3]), &de::<Vec<RulerGuide>>(&a[4]))),
        "snapSpacing" => match snap_spacing(&rect(&a[0]), &de::<Vec<Rect>>(&a[1]), axis(&a[2]), f(&a[3])) {
            Some((delta, marks)) => json!({ "delta": j(&delta), "marks": j(&marks) }),
            None => Value::Null,
        },
        "snapValue" => j(&snap_value(f(&a[0]), &de::<Vec<f64>>(&a[1]), f(&a[2]))),
        "snapAngle" => j(&snap_angle(f(&a[0]), a[1].as_bool().expect("bool"))),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn align() {
    replay("align", |name, a| match name {
        "alignDeltas" => deltas(align_deltas(&de::<Vec<AlignItem>>(&a[0]), de(&a[1]), &rect(&a[2]))),
        "distributeDeltas" => deltas(distribute_deltas(&de::<Vec<AlignItem>>(&a[0]), axis(&a[1]))),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn connectors() {
    replay("connectors", |name, a| match name {
        "routeConnector" => {
            let route = if is_undef(&a[4]) { None } else { Some(LineRoute::parse(s(&a[4]))) };
            j(&route_connector(v2(&a[0]), v2(&a[1]), v2(&a[2]), v2(&a[3]), route.as_ref()))
        }
        "ANCHORS" => j(&ANCHORS),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn markers() {
    replay("markers", |name, a| match name {
        "lineMarkerShape" => j(&line_marker_shape(&LineMarker::parse(s(&a[0])), f(&a[1]))),
        "trimLineForMarkers" => j(&trim_line_for_markers(
            &de::<Vec<Vec2>>(&a[0]),
            &LineMarker::parse(s(&a[1])),
            &LineMarker::parse(s(&a[2])),
            f(&a[3]),
        )),
        _ => panic!("unknown fn {name}"),
    });
}

#[test]
fn frames() {
    replay("frames", |name, a| match name {
        "frameOutline" => {
            let clip = FrameClip::parse(s(&a[0]["clip"]));
            j(&frame_outline(&clip, a[0].get("clipPath").and_then(Value::as_str), f(&a[1]), f(&a[2])))
        }
        "frameOverlays" => {
            let overlay: Option<Vec<FrameOverlay>> = a[0].get("overlay").map(de);
            j(&frame_overlays(overlay.as_deref(), f(&a[1]), f(&a[2])))
        }
        _ => panic!("unknown fn {name}"),
    });
}
