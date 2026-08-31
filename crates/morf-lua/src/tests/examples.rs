use crate::*;
use morf_scene::{Element, Value as SceneValue};
use std::time::Duration;

use super::*;

// The shipped examples, exercised through the runtime they are written for.

#[test]
fn fluid_transform_example_animates_square_to_circle_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "examples/fluid-transform.lua",
            include_bytes!("../../../../examples/fluid-transform.lua"),
        )
        .unwrap();
    runtime.tick_animations(Duration::from_secs(2)).unwrap();
    let root = runtime.scene().roots()[0];
    let shape = runtime.scene().children(root).unwrap()[1];
    let pointer = runtime.scene().children(shape).unwrap()[0];
    assert_eq!(runtime.scene().number(shape, "radius").unwrap(), 12.0);

    assert_eq!(
        runtime.scene().element(pointer).unwrap(),
        Element::MouseArea
    );
    assert!(runtime.dispatch_ui_event(pointer, UiEvent::Clicked));
    assert_eq!(
        runtime.scene().target(shape, "radius").unwrap(),
        &SceneValue::Number(60.0)
    );
    assert_eq!(
        runtime.scene().target(shape, "translate_x").unwrap(),
        &SceneValue::Number(270.0)
    );

    let frame = runtime.tick_animations(Duration::from_millis(16)).unwrap();
    let radius = runtime.scene().number(shape, "radius").unwrap();
    assert!(radius > 12.0 && radius < 60.0);
    assert!(frame.active);
    assert!(frame.changed > 0, "the transform advanced");
    let moved = runtime.scene().number(shape, "translate_x").unwrap();
    assert!(
        moved != 0.0,
        "and it is the translation that moved: {moved}"
    );
}

#[test]
fn morph_stack_example_combines_native_animation_and_geometry() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "examples/morph-stack.lua",
            include_bytes!("../../../../examples/morph-stack.lua"),
        )
        .unwrap();
    runtime.tick_animations(Duration::from_secs(2)).unwrap();
    let root = runtime.scene().roots()[0];
    let root_children = runtime.scene().children(root).unwrap().to_vec();
    let field = root_children[10];
    let shape = runtime.scene().children(field).unwrap()[0];
    let second_stage = root_children[6];
    let second_stage_children = runtime.scene().children(second_stage).unwrap().to_vec();
    let pointer = second_stage_children[4];

    assert_eq!(runtime.scene().element(field).unwrap(), Element::Sdf);
    assert_eq!(
        runtime.scene().string_value(shape, "shape").unwrap(),
        "circle"
    );
    assert_eq!(
        runtime.scene().string_value(shape, "morph_to").unwrap(),
        "star"
    );
    assert_eq!(
        runtime.scene().element(pointer).unwrap(),
        Element::MouseArea
    );
    assert!(runtime.dispatch_ui_event(pointer, UiEvent::Clicked));
    assert_eq!(
        runtime.scene().target(shape, "morph_progress").unwrap(),
        &SceneValue::Number(1.0 / 3.0)
    );

    let frame = runtime.tick_animations(Duration::from_millis(16)).unwrap();
    let progress = runtime.scene().number(shape, "morph_progress").unwrap();
    assert!(progress > 0.0 && progress < 1.0);
    assert!(frame.changed > 0, "a property advanced");
}
#[test]
fn motion_lab_example_drives_loops_shapes_and_field_edges_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "examples/motion-lab.lua",
            include_bytes!("../../../../examples/motion-lab.lua"),
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let children = runtime.scene().children(root).unwrap().to_vec();
    let of_element = |element| {
        children
            .iter()
            .copied()
            .find(|node| runtime.scene().element(*node).unwrap() == element)
            .expect("example is missing an element the test needs")
    };
    let badge = of_element(Element::Sdf);
    let glyph = of_element(Element::Image);

    // The badge morphs between two shape families as fields, and its point
    // count is an ordinary number that may sit between whole values.
    let layer = runtime.scene().children(badge).unwrap()[0];
    assert_eq!(runtime.scene().element(layer).unwrap(), Element::SdfShape);
    assert_eq!(
        runtime.scene().string_value(layer, "shape").unwrap(),
        "circle"
    );
    assert_eq!(
        runtime.scene().string_value(layer, "morph_to").unwrap(),
        "star"
    );
    assert_eq!(runtime.scene().number(layer, "points").unwrap(), 5.0);

    // The intro group runs first and reports its completion into Lua, which is
    // the one thing in this example that does run Lua on a tick.
    let mut completed = false;
    for _ in 0..20 {
        let frame = runtime.tick_animations(Duration::from_millis(50)).unwrap();
        completed |= !frame.groups.is_empty();
    }
    assert!(completed, "the intro group never reported its completion");
    // The group's parallel leg targets a second node, which ends up faded in.
    assert_eq!(runtime.scene().number(glyph, "opacity").unwrap(), 1.0);
    // The field edge is a plain animatable property sitting at its idle value.
    assert_eq!(runtime.scene().number(glyph, "thickness").unwrap(), 0.0);

    // With the group done, the endless behaviors keep asking for frames without
    // running a single Lua effect to do it.
    let runs = runtime.effect_runs();
    for _ in 0..40 {
        let frame = runtime.tick_animations(Duration::from_millis(50)).unwrap();
        assert!(frame.active);
        assert!(frame.groups.is_empty());
    }
    assert_eq!(runtime.effect_runs(), runs);

    // Pausing the sweep holds it in place; resuming continues from there.
    // Found by its target rather than its current value: a ping-pong sweep
    // passes through zero twice a cycle.
    let sweep = children
        .iter()
        .copied()
        .find(|node| {
            runtime.scene().element(*node).unwrap() == Element::Rect
                && runtime.scene().target(*node, "translate_x").unwrap()
                    == &SceneValue::Number(252.0)
        })
        .expect("example is missing its sweep");
    runtime
        .scene_mut()
        .set_animation_paused(sweep, "translate_x", true)
        .unwrap();
    let held = runtime.scene().number(sweep, "translate_x").unwrap();
    runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(runtime.scene().number(sweep, "translate_x").unwrap(), held);
}
