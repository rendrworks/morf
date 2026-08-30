// Sequential and parallel scheduling over ordinary property animations.

use super::*;

fn step(node: NodeHandle, property: &str, to: f64, millis: u64) -> AnimationStep {
    AnimationStep::Property {
        node,
        property: property.to_owned(),
        from: None,
        to: Value::Number(to),
        behavior: Behavior::timed(Duration::from_millis(millis), Easing::Linear),
    }
}

#[test]
fn a_sequence_starts_each_step_only_after_the_one_before_it() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(
            AnimationStep::Sequential(vec![
                step(rect, "x", 100.0, 100),
                AnimationStep::Pause(Duration::from_millis(50)),
                step(rect, "y", 40.0, 100),
            ]),
            Repeat::Once,
        )
        .unwrap();

    // The first step starts on the first tick; the second has not been reached.
    scene.tick_animations(Duration::from_millis(50)).unwrap();
    assert!(scene.is_animating(rect, "x").unwrap());
    assert!(!scene.is_animating(rect, "y").unwrap());

    // Past the first step and its pause, the second is under way.
    scene.tick_animations(Duration::from_millis(110)).unwrap();
    assert_eq!(scene.number(rect, "x").unwrap(), 100.0);
    assert!(scene.is_animating(rect, "y").unwrap());

    let frame = scene.tick_animations(Duration::from_millis(150)).unwrap();
    assert_eq!(scene.number(rect, "y").unwrap(), 40.0);
    assert_eq!(
        frame.groups,
        vec![GroupEvent {
            group,
            end: AnimationEnd::Completed,
        }]
    );
    assert!(!scene.is_group_active(group));
}

#[test]
fn a_parallel_group_starts_every_child_on_the_same_tick() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .start_group(
            AnimationStep::Parallel(vec![
                step(rect, "x", 100.0, 100),
                step(rect, "y", 100.0, 200),
            ]),
            Repeat::Once,
        )
        .unwrap();

    scene.tick_animations(Duration::from_millis(50)).unwrap();
    assert!(scene.is_animating(rect, "x").unwrap());
    assert!(scene.is_animating(rect, "y").unwrap());

    // The group runs as long as its longest child, not its first.
    scene.tick_animations(Duration::from_millis(60)).unwrap();
    assert_eq!(scene.number(rect, "x").unwrap(), 100.0);
    assert!(scene.is_animating(rect, "y").unwrap());
}

#[test]
fn a_repeating_group_replays_its_whole_schedule() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(
            AnimationStep::Sequential(vec![step(rect, "x", 100.0, 100), step(rect, "x", 0.0, 100)]),
            Repeat::Times(2),
        )
        .unwrap();

    // Two passes over a two-step, 200ms schedule: three ticks still have work.
    for _ in 0..3 {
        scene.tick_animations(Duration::from_millis(100)).unwrap();
        assert!(scene.is_group_active(group));
    }
    let frame = scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert!(!scene.is_group_active(group));
    assert!(frame.groups.iter().any(|event| event.group == group));
}

#[test]
fn a_group_step_departs_from_an_explicit_start_when_given_one() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene.assign(rect, "opacity", 1.0).unwrap();
    scene
        .start_group(
            AnimationStep::Property {
                node: rect,
                property: "opacity".to_owned(),
                from: Some(Value::Number(0.0)),
                to: Value::Number(1.0),
                behavior: Behavior::timed(Duration::from_millis(100), Easing::Linear),
            },
            Repeat::Once,
        )
        .unwrap();

    scene.tick_animations(Duration::from_millis(50)).unwrap();
    assert_eq!(scene.number(rect, "opacity").unwrap(), 0.5);
}

#[test]
fn finishing_a_group_lands_every_step_on_its_target() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(
            AnimationStep::Sequential(vec![
                step(rect, "x", 100.0, 200),
                step(rect, "y", 60.0, 200),
            ]),
            Repeat::Once,
        )
        .unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();

    assert!(scene.finish_group(group).unwrap());
    assert_eq!(scene.number(rect, "x").unwrap(), 100.0);
    assert_eq!(scene.number(rect, "y").unwrap(), 60.0);
    assert!(!scene.is_group_active(group));
    assert!(!scene.finish_group(group).unwrap());
}

#[test]
fn stopping_a_group_abandons_the_steps_it_had_not_started() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(
            AnimationStep::Sequential(vec![
                step(rect, "x", 100.0, 100),
                step(rect, "y", 60.0, 100),
            ]),
            Repeat::Once,
        )
        .unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();

    assert!(scene.stop_group(group));
    let frame = scene.tick_animations(Duration::from_millis(200)).unwrap();
    assert_eq!(frame.groups[0].end, AnimationEnd::Stopped);
    // The step already in flight was left alone; the one after it never ran.
    assert_eq!(scene.number(rect, "x").unwrap(), 100.0);
    assert_eq!(scene.number(rect, "y").unwrap(), 0.0);
}

#[test]
fn a_paused_group_stops_scheduling_without_freezing_what_is_running() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(
            AnimationStep::Sequential(vec![
                step(rect, "x", 100.0, 100),
                step(rect, "y", 60.0, 100),
            ]),
            Repeat::Once,
        )
        .unwrap();
    scene.tick_animations(Duration::from_millis(20)).unwrap();
    assert!(scene.set_group_paused(group, true));

    scene.tick_animations(Duration::from_millis(200)).unwrap();
    // The started step ran to its end; the next one is still waiting.
    assert_eq!(scene.number(rect, "x").unwrap(), 100.0);
    assert!(!scene.is_animating(rect, "y").unwrap());
    assert!(scene.is_group_active(group));

    assert!(scene.set_group_paused(group, false));
    scene.tick_animations(Duration::from_millis(100)).unwrap();
    assert!(scene.is_animating(rect, "y").unwrap());
}

#[test]
fn a_group_refuses_a_schedule_it_could_never_finish() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let endless = AnimationStep::Property {
        node: rect,
        property: "opacity".to_owned(),
        from: None,
        to: Value::Number(0.0),
        behavior: Behavior {
            duration: Duration::from_millis(100),
            repeat: Repeat::Forever,
            ..Behavior::default()
        },
    };

    assert!(scene.start_group(endless, Repeat::Once).is_err());
    assert!(
        scene
            .start_group(step(rect, "x", 1.0, 100), Repeat::PingPong)
            .is_err()
    );
    // An unknown property is rejected where the group starts, not mid-playback.
    assert!(
        scene
            .start_group(step(rect, "not_a_property", 1.0, 100), Repeat::Once)
            .is_err()
    );
    // A schedule with no length cannot repeat forever without spinning.
    assert!(
        scene
            .start_group(AnimationStep::Sequential(Vec::new()), Repeat::Forever)
            .is_err()
    );
    // The same empty schedule is fine when it only has to run once.
    assert!(
        scene
            .start_group(AnimationStep::Sequential(Vec::new()), Repeat::Once)
            .is_ok()
    );
}

#[test]
fn removing_a_targeted_node_drops_the_group_scheduling_for_it() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let group = scene
        .start_group(step(rect, "x", 100.0, 100), Repeat::Forever)
        .unwrap();
    assert!(scene.is_group_active(group));

    scene.remove(rect).unwrap();
    assert!(!scene.is_group_active(group));

    // The cancellation is reported so anything waiting on the group hears about
    // it rather than waiting forever.
    let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert_eq!(
        frame.groups,
        vec![GroupEvent {
            group,
            end: AnimationEnd::Canceled,
        }]
    );
}
