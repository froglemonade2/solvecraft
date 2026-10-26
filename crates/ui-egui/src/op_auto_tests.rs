//! Feature windows choose Join, New Body or Cut from what their material does (#6901).

use std::time::{Duration, Instant};

use serde_json::json;
use solidcraft_engine::geom::Vec3;

use super::{CUT, JOIN, NEW};
use crate::dialogs::Kind;
use crate::scenario::Harness;

/// A 10 mm cube at the origin, named `Base`.
fn cube() -> Harness {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Base"})).unwrap();
    h
}

fn rect_sketch(h: &mut Harness, p0: [f64; 2], p1: [f64; 2]) {
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    h.app.run("sketch.rectangle.two_point", json!({"p0": p0, "p1": p1})).unwrap();
    h.app.run("sketch.finish", json!({})).unwrap();
    h.app.run("select.clear", json!({})).unwrap();
}

/// Frames until the window's check is in.
fn checked(h: &mut Harness) {
    let began = Instant::now();
    while (h.app.op_check.busy() || h.app.dialog.as_ref().is_some_and(|d| d.op_auto.pending)) && began.elapsed() < Duration::from_secs(60) {
        h.frames(1);
        std::thread::sleep(Duration::from_millis(1));
    }
    h.frames(2);
}

fn op(h: &mut Harness) -> usize {
    match h.app.dialog.as_mut().map(|d| &mut d.kind) {
        Some(
            Kind::Extrude { operation, .. } | Kind::Revolve { operation, .. } | Kind::Primitive { operation, .. } | Kind::Loft { operation, .. },
        ) => *operation,
        other => panic!("a feature window: {other:?}"),
    }
}

/// Pick an operation in the window's list (the shown one too): what [`super::op_row`] notes.
fn pick(h: &mut Harness, to: usize) {
    if let Some(d) = h.app.dialog.as_mut() {
        match &mut d.kind {
            Kind::Extrude { operation, .. } | Kind::Revolve { operation, .. } | Kind::Primitive { operation, .. } | Kind::Loft { operation, .. } => {
                *operation = to
            }
            other => panic!("a feature window: {other:?}"),
        }
    }
    h.ctx.data_mut(|m| m.insert_temp(super::picked_id(), true));
}

fn bodies(h: &Harness) -> Vec<String> {
    h.app.session.model.state().bodies.iter().map(|b| b.name.clone()).collect()
}

fn ok(h: &mut Harness) {
    h.step(&json!({"key": "Enter"})).unwrap();
    checked(h);
    let began = Instant::now();
    while h.app.dialog.is_some() && began.elapsed() < Duration::from_secs(60) {
        h.frames(1);
    }
    assert!(h.app.dialog.is_none(), "OK closes the window");
}

#[test]
fn an_extrude_apart_is_a_new_body_and_one_touching_joins() {
    for (p0, p1, want) in [([30.0, 0.0], [40.0, 10.0], NEW), ([10.0, 0.0], [20.0, 10.0], JOIN), ([10.0, 5.0], [20.0, 15.0], JOIN)] {
        let mut h = cube();
        rect_sketch(&mut h, p0, p1);
        h.step(&json!({"start": "solid.extrude"})).unwrap();
        checked(&mut h);
        assert_eq!(op(&mut h), want, "{p0:?}–{p1:?}");
        ok(&mut h);
        assert_eq!(bodies(&h).len(), if want == NEW { 2 } else { 1 }, "{p0:?}–{p1:?}");
    }
}

#[test]
fn a_ring_round_a_hole_joins() {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 40, "width": 40, "height": 10, "corner": [-20, -20, 0], "body_name": "Base"})).unwrap();
    h.app.run("solid.cylinder", json!({"radius": 5, "height": 10, "operation": "cut"})).unwrap();
    h.app.run("sketch.create", json!({"plane": {"origin": [0, 0, 10], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}})).unwrap();
    h.app.run("sketch.circle.center", json!({"center": [0, 0], "radius": 5})).unwrap();
    h.app.run("sketch.circle.center", json!({"center": [0, 0], "radius": 10})).unwrap();
    h.app.run("sketch.finish", json!({})).unwrap();
    h.app.run("select.clear", json!({})).unwrap();
    h.step(&json!({"start": "solid.extrude"})).unwrap();
    checked(&mut h);
    if let Some(d) = h.app.dialog.as_mut() {
        // The ring alone, not the disc inside it.
        d.inputs[0].items.retain(|s| {
            matches!(s, solidcraft_engine::Sel::Profile { index, .. } if *index == 1) || !matches!(s, solidcraft_engine::Sel::Profile { .. })
        });
    }
    checked(&mut h);
    assert_eq!(op(&mut h), JOIN);
}

#[test]
fn primitives_and_revolves_join_cut_or_stand_apart() {
    let at = |h: &mut Harness, point: [f64; 3]| {
        if let Some(Kind::Primitive { at, .. }) = h.app.dialog.as_mut().map(|d| &mut d.kind) {
            *at = Some((Vec3::new(point[0], point[1], point[2]), Some(Vec3::Z)));
        }
    };
    for (point, want) in [([50.0, 0.0, 0.0], NEW), ([5.0, 5.0, 10.0], JOIN), ([5.0, 5.0, 5.0], CUT)] {
        let mut h = cube();
        h.step(&json!({"start": "solid.sphere"})).unwrap();
        if let Some(Kind::Primitive { fields, .. }) = h.app.dialog.as_mut().map(|d| &mut d.kind) {
            fields.iter_mut().for_each(|(_, v)| *v = "4 mm".into());
        }
        at(&mut h, point);
        checked(&mut h);
        assert_eq!(op(&mut h), want, "sphere at {point:?}");
    }
}

#[test]
fn a_hand_picked_operation_stays_even_while_the_check_runs() {
    // Picked before the first check is in: that check doesn't undo it.
    let mut h = cube();
    rect_sketch(&mut h, [30.0, 0.0], [40.0, 10.0]);
    h.step(&json!({"start": "solid.extrude"})).unwrap();
    pick(&mut h, JOIN);
    checked(&mut h);
    assert_eq!(op(&mut h), JOIN, "Join picked by hand stays, apart or not");
    // The same in a window without Extrude's own choice.
    let mut h = cube();
    h.step(&json!({"start": "solid.sphere"})).unwrap();
    h.frames(1);
    pick(&mut h, NEW);
    checked(&mut h);
    assert_eq!(op(&mut h), NEW, "New Body picked by hand on a body stays");
}

#[test]
fn ok_runs_what_the_check_chose() {
    // Enter at once: OK waits for the check, then makes a new body for the block apart.
    let mut h = cube();
    rect_sketch(&mut h, [30.0, 0.0], [40.0, 10.0]);
    h.step(&json!({"start": "solid.extrude"})).unwrap();
    ok(&mut h);
    assert_eq!(bodies(&h).len(), 2, "{:?}", bodies(&h));
}

#[test]
fn hidden_bodies_take_no_part() {
    let mut h = cube();
    h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [40, 0, 0], "body_name": "Other"})).unwrap();
    h.app.run("browser.visibility", json!({"items": ["Base"], "visible": false})).unwrap();
    // A block on the hidden cube: apart from what is shown.
    rect_sketch(&mut h, [10.0, 0.0], [20.0, 10.0]);
    h.step(&json!({"start": "solid.extrude"})).unwrap();
    checked(&mut h);
    assert_eq!(op(&mut h), NEW, "the hidden cube is not joined");
    // Join picked by hand joins only the shown bodies: the hidden cube stays as it is.
    pick(&mut h, JOIN);
    ok(&mut h);
    let st = h.app.session.model.state();
    assert!(st.body("Base").is_some_and(|b| b.body.face_count() == 6), "the hidden cube is untouched");
    // Nothing shown: New Body at once.
    let mut h = cube();
    h.app.run("browser.visibility", json!({"items": ["Base"], "visible": false})).unwrap();
    rect_sketch(&mut h, [10.0, 0.0], [20.0, 10.0]);
    h.step(&json!({"start": "solid.extrude"})).unwrap();
    h.frames(2);
    assert_eq!(op(&mut h), NEW);
}

#[test]
fn an_edited_feature_keeps_its_operation() {
    let mut h = cube();
    rect_sketch(&mut h, [30.0, 0.0], [40.0, 10.0]);
    h.app.run("solid.extrude", json!({"distance": 5, "operation": "join", "name": "Apart"})).unwrap();
    h.step(&json!({"call": "ui.editFeature", "params": {"feature": "Apart"}})).unwrap();
    checked(&mut h);
    assert_eq!(op(&mut h), JOIN, "the saved Join stays");
}
