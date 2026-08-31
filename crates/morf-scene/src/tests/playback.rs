// Delay, repetition, and the imperative playback controls over motion in flight.

use super::*;

#[test]
fn a_delayed_behavior_holds_the_start_value_until_the_delay_expires() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(100),
                delay: Duration::from_millis(100),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();

    scene.tick_animations(Duration::from_millis(50)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 1.0);

    scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert!(scene.number(rect, "opacity").unwrap() < 1.0);
}

#[test]
fn a_time_scaled_behavior_covers_the_interval_at_the_scaled_rate() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(100),
                time_scale: 2.0,
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();

    let frame = scene.tick_animations(Duration::from_millis(50)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 0.0);
    assert!(!frame.active);
}

#[test]
fn a_forever_behavior_never_settles_and_never_reports_completion() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "rotation",
            Some(Behavior {
                duration: Duration::from_millis(100),
                repeat: Repeat::Forever,
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "rotation", 360.0).unwrap();

    for _ in 0..10 {
        let frame = scene.tick_animations(Duration::from_millis(60)).unwrap();
        assert!(frame.active);
        assert!(frame.events.is_empty());
    }
    assert!(scene.is_animating(rect, "rotation").unwrap());
}

#[test]
fn a_ping_pong_behavior_returns_to_the_value_it_started_from() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(100),
                repeat: Repeat::PingPongTimes(2),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();

    scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert!(scene.number(rect, "opacity").unwrap() < 0.5);

    let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 1.0);
    assert!(!frame.active);
    assert_eq!(frame.events[0].end, AnimationEnd::Completed);
}

#[test]
fn a_disabled_behavior_lets_the_next_write_land_immediately() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    assert!(scene.set_behavior_enabled(rect, "opacity", false).unwrap());

    scene.assign(rect, "opacity", 0.0).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 0.0);
    assert!(!scene.is_animating(rect, "opacity").unwrap());

    // The declaration survives being switched off, so re-enabling animates again.
    assert!(scene.set_behavior_enabled(rect, "opacity", true).unwrap());
    scene.assign(rect, "opacity", 1.0).unwrap();
    assert!(scene.is_animating(rect, "opacity").unwrap());
}

#[test]
fn a_paused_animation_holds_its_value_and_resumes_from_there() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();
    scene.tick_animations(Duration::from_millis(100)).unwrap();
    let held = scene.number(rect, "opacity").unwrap();

    assert!(scene.set_animation_paused(rect, "opacity", true).unwrap());
    assert!(scene.is_animation_paused(rect, "opacity").unwrap());
    scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), held);

    assert!(scene.set_animation_paused(rect, "opacity", false).unwrap());
    scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 0.0);
}

#[test]
fn stopping_pins_the_target_where_the_property_stands() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();
    scene.tick_animations(Duration::from_millis(100)).unwrap();

    assert!(scene.stop_animation(rect, "opacity").unwrap());
    assert_eq!(
        scene.current(rect, "opacity").unwrap(),
        scene.target(rect, "opacity").unwrap()
    );
    let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(frame.events[0].end, AnimationEnd::Stopped);
    assert!(!frame.active);
}

#[test]
fn finishing_lands_on_the_target_without_further_ticks() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.25).unwrap();

    assert!(scene.finish_animation(rect, "opacity").unwrap());
    assert_eq!(scene.number(rect, "opacity").unwrap(), 0.25);
    assert!(!scene.is_animating(rect, "opacity").unwrap());
}

#[test]
fn finishing_a_spring_lands_on_the_target_it_was_chasing() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_physics(
            rect,
            "x",
            Some(Physics::Spring {
                mass: 1.0,
                damping: 18.0,
                stiffness: 180.0,
                epsilon: 0.001,
            }),
        )
        .unwrap();
    scene.assign(rect, "x", 120.0).unwrap();
    scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert!(scene.number(rect, "x").unwrap() < 120.0);

    assert!(scene.finish_animation(rect, "x").unwrap());
    assert_eq!(scene.number(rect, "x").unwrap(), 120.0);
}

#[test]
fn reversing_keeps_the_property_still_and_sends_it_back() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();
    let before = scene.number(rect, "opacity").unwrap();

    assert!(scene.reverse_animation(rect, "opacity").unwrap());
    // The property does not jump, and the value it set out from is the new target.
    assert!((scene.number(rect, "opacity").unwrap() - before).abs() < 1e-6);
    assert_eq!(scene.target(rect, "opacity").unwrap(), &Value::Number(1.0));

    // Momentum carries the return leg past the reversal point rather than
    // snapping direction, so the arrival is what the test pins down.
    scene.tick_animations(Duration::from_millis(200)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 1.0);
    assert!(!scene.is_animating(rect, "opacity").unwrap());
}

#[test]
fn seeking_scrubs_an_animation_without_ending_it() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "width",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "width", 100.0).unwrap();

    assert!(scene.seek_animation(rect, "width", 0.75).unwrap());
    assert_eq!(scene.number(rect, "width").unwrap(), 75.0);
    assert!(scene.is_animating(rect, "width").unwrap());
    assert_eq!(scene.animation_progress(rect, "width").unwrap(), Some(0.75));
}

#[test]
fn a_hard_write_over_an_animation_reports_the_cancellation() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_behavior(
            rect,
            "opacity",
            Some(Behavior {
                duration: Duration::from_millis(200),
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene.assign(rect, "opacity", 0.0).unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();
    scene.set_behavior(rect, "opacity", None).unwrap();

    let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert_eq!(frame.events[0].end, AnimationEnd::Canceled);
    assert_eq!(frame.events[0].property, "opacity");
}

#[test]
fn compound_easing_helpers_share_one_curve_across_components() {
    let curve = Easing::InOutCubic;
    let eased = curve.value_at(0.3);

    let point = curve.interpolate_point(0.3, [0.0, 10.0], [100.0, 20.0]);
    assert!((point[0] - eased * 100.0).abs() < 1e-9);
    assert!((point[1] - (10.0 + eased * 10.0)).abs() < 1e-9);

    let rect = curve.interpolate_rect(0.3, [0.0; 4], [10.0, 20.0, 30.0, 40.0]);
    assert!((rect[3] - eased * 40.0).abs() < 1e-9);

    let color = curve.interpolate_color(
        0.3,
        Color::rgba8(0, 0, 0, 255),
        Color::rgba8(255, 255, 255, 255),
    );
    assert!((f64::from(color.red) - eased).abs() < 1e-6);
}
