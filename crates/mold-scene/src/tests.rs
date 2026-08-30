use super::*;

#[test]
fn reparenting_preserves_identity_and_order() {
    let mut scene = Scene::new();
    let first_parent = scene.create(Element::Item);
    let second_parent = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    scene.reparent(child, Some(first_parent)).unwrap();
    scene.reparent(child, Some(second_parent)).unwrap();

    assert!(scene.children(first_parent).unwrap().is_empty());
    assert_eq!(scene.children(second_parent).unwrap(), vec![child]);
    assert_eq!(scene.parent(child).unwrap(), Some(second_parent));
}

#[test]
fn reparenting_rejects_descendant_cycles() {
    let mut scene = Scene::new();
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Item);
    scene.reparent(child, Some(parent)).unwrap();

    assert_eq!(
        scene.reparent(parent, Some(child)),
        Err(SceneError::ParentCycle)
    );
}

#[test]
fn removed_handles_are_detectably_stale() {
    let mut scene = Scene::new();
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Text);
    scene.reparent(child, Some(parent)).unwrap();
    scene.remove(parent).unwrap();

    assert!(!scene.contains(parent));
    assert!(!scene.contains(child));
    assert_eq!(scene.parent(child), Err(SceneError::StaleNode));
}

#[test]
fn properties_coerce_colors_and_update_both_levels() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene.assign(rect, "color", "#7c3aed").unwrap();

    let expected = Value::Color(Color::rgba8(0x7c, 0x3a, 0xed, 0xff));
    assert_eq!(scene.current(rect, "color").unwrap(), &expected);
    assert_eq!(scene.target(rect, "color").unwrap(), &expected);
}

#[test]
fn property_errors_name_the_element_and_property() {
    let mut scene = Scene::new();
    let text = scene.create(Element::Text);

    let unknown = scene.assign(text, "radius", 4.0).unwrap_err();
    assert_eq!(unknown.to_string(), "unknown Text property `radius`");
    let wrong = scene.assign(text, "font_size", "large").unwrap_err();
    assert!(wrong.to_string().contains("Text property `font_size`"));
}

#[test]
fn text_has_regular_font_weight_by_default() {
    let mut scene = Scene::new();
    let text = scene.create(Element::Text);

    assert_eq!(scene.number(text, "font_weight").unwrap(), 400.0);
    scene.assign(text, "font_weight", 700.0).unwrap();
    assert_eq!(scene.number(text, "font_weight").unwrap(), 700.0);
}

#[test]
fn behavior_intercepts_writes_and_keeps_target_live() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "width",
            Some(Behavior {
                duration: Duration::from_millis(200),
                easing: Easing::Linear,
            }),
        )
        .unwrap();

    scene.assign(rect, "width", 100.0).unwrap();
    assert_eq!(scene.current(rect, "width").unwrap(), &Value::Number(0.0));
    assert_eq!(scene.target(rect, "width").unwrap(), &Value::Number(100.0));
    let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();

    assert_eq!(scene.current(rect, "width").unwrap(), &Value::Number(50.0));
    assert_eq!(frame.changes[0].class, PropertyClass::Layout);
    assert!(frame.active);
}

#[test]
fn interrupted_animation_retargets_without_a_jump() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let behavior = Behavior {
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    };
    scene.set_behavior(rect, "opacity", Some(behavior)).unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();
    let before = scene.number(rect, "opacity").unwrap();

    scene.assign(rect, "opacity", 0.8).unwrap();
    let retargeted = scene.number(rect, "opacity").unwrap();
    scene.tick_animations(Duration::from_millis(1)).unwrap();
    let after = scene.number(rect, "opacity").unwrap();

    assert_eq!(before, retargeted);
    assert!((after - before).abs() < 0.02);
    assert_eq!(scene.target(rect, "opacity").unwrap(), &Value::Number(0.8));
}

#[test]
fn paint_animation_finishes_at_the_exact_target() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "color",
            Some(Behavior {
                duration: Duration::from_millis(120),
                easing: Easing::OutCubic,
            }),
        )
        .unwrap();
    scene.assign(rect, "color", "#7c3aed").unwrap();

    let frame = scene.tick_animations(Duration::from_millis(120)).unwrap();

    assert_eq!(scene.current(rect, "color"), scene.target(rect, "color"));
    assert_eq!(frame.changes[0].class, PropertyClass::Paint);
    assert!(!frame.active);
}

#[test]
fn easing_families_preserve_endpoints_and_interpolate() {
    for easing in [
        Easing::Linear,
        Easing::InQuad,
        Easing::OutQuad,
        Easing::InOutQuad,
        Easing::InCubic,
        Easing::OutCubic,
        Easing::InOutCubic,
        Easing::InQuart,
        Easing::OutQuart,
        Easing::InOutQuart,
        Easing::InQuint,
        Easing::OutQuint,
        Easing::InOutQuint,
        Easing::InSine,
        Easing::OutSine,
        Easing::InOutSine,
        Easing::InExpo,
        Easing::OutExpo,
        Easing::InOutExpo,
        Easing::InCirc,
        Easing::OutCirc,
        Easing::InOutCirc,
        Easing::InBack,
        Easing::OutBack,
        Easing::InOutBack,
        Easing::InBounce,
        Easing::OutBounce,
        Easing::InOutBounce,
    ] {
        assert!((easing.value_at(0.0) - 0.0).abs() < 1e-9);
        assert!((easing.value_at(1.0) - 1.0).abs() < 1e-9);
    }
    assert_eq!(Easing::InQuad.interpolate(0.5, 10.0, 20.0), 12.5);
    assert_eq!(Easing::OutQuad.value_at(-1.0), 0.0);
    assert_eq!(Easing::OutQuad.value_at(2.0), 1.0);
}

#[test]
fn spring_retargets_with_continuous_position_and_velocity() {
    let mut scene = Scene::new();
    let item = scene.create(Element::Item);
    scene
        .set_physics(
            item,
            "x",
            Some(Physics::Spring {
                mass: 1.0,
                damping: 18.0,
                stiffness: 180.0,
                epsilon: 0.001,
            }),
        )
        .unwrap();
    scene.assign(item, "x", 100.0).unwrap();
    scene.tick_animations(Duration::from_millis(80)).unwrap();
    let before = scene.number(item, "x").unwrap();

    scene.assign(item, "x", -20.0).unwrap();
    assert_eq!(scene.number(item, "x").unwrap(), before);
    scene.tick_animations(Duration::from_millis(1)).unwrap();
    assert!((scene.number(item, "x").unwrap() - before).abs() < 2.0);
}

#[test]
fn smoothed_motion_obeys_velocity_limit() {
    let mut scene = Scene::new();
    let item = scene.create(Element::Item);
    scene
        .set_physics(item, "x", Some(Physics::Smoothed { velocity: 200.0 }))
        .unwrap();
    scene.assign(item, "x", 100.0).unwrap();

    let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(scene.number(item, "x").unwrap(), 20.0);
    assert!(frame.active);
    scene.tick_animations(Duration::from_millis(400)).unwrap();
    assert_eq!(scene.number(item, "x").unwrap(), 100.0);
}
