use super::*;

/// Decay physics with the given friction and no bounds.
fn coasting(friction: f64) -> Physics {
    Physics::Decay {
        friction,
        min_velocity: 1.0,
        bounds: None,
        gravity: 0.0,
        restitution: 0.0,
    }
}

#[test]
fn a_fling_coasts_to_a_halt_and_further_the_harder_it_is_thrown() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);

    let landing = |velocity: f64| {
        let mut scene = Scene::new();
        let node = scene.create(Element::Rect);
        scene.fling(node, "x", velocity, coasting(600.0)).unwrap();
        for _ in 0..400 {
            scene.tick_animations(Duration::from_millis(16)).unwrap();
        }
        scene.number(node, "x").unwrap()
    };

    let gentle = landing(300.0);
    let hard = landing(900.0);
    assert!(gentle > 0.0, "a fling moves the property: {gentle}");
    assert!(
        hard > gentle * 2.0,
        "a harder throw goes further: {hard} against {gentle}"
    );

    // And it stops on its own rather than running forever.
    scene.fling(rect, "x", 500.0, coasting(600.0)).unwrap();
    let mut frames = 0;
    loop {
        let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
        frames += 1;
        if !frame.active {
            break;
        }
        assert!(frames < 600, "the fling never settled");
    }
}

#[test]
fn a_bound_catches_a_fling_where_it_is_set() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .fling(
            rect,
            "x",
            4000.0,
            Physics::Decay {
                friction: 200.0,
                min_velocity: 1.0,
                bounds: Some((-100.0, 250.0)),
                gravity: 0.0,
                restitution: 0.0,
            },
        )
        .unwrap();
    for _ in 0..400 {
        scene.tick_animations(Duration::from_millis(16)).unwrap();
    }

    // Thrown far past the limit, it rests exactly on it rather than beyond.
    assert_eq!(scene.number(rect, "x").unwrap(), 250.0);
}

#[test]
fn writing_a_property_mid_fling_catches_it_at_speed() {
    // The interaction that makes a fling worth having as physics rather than a
    // curve: a flick can be caught. Whatever animates the property next starts
    // from where the fling had reached, still moving at its speed.
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .set_physics(
            rect,
            "x",
            Some(Physics::Spring {
                mass: 1.0,
                damping: 18.0,
                stiffness: 200.0,
                epsilon: 0.05,
            }),
        )
        .unwrap();
    scene.fling(rect, "x", 1200.0, coasting(400.0)).unwrap();
    for _ in 0..6 {
        scene.tick_animations(Duration::from_millis(16)).unwrap();
    }
    let caught_at = scene.number(rect, "x").unwrap();
    assert!(caught_at > 0.0, "the fling had travelled: {caught_at}");

    // The spring takes over from there, and the momentum carries it past the
    // target before it comes back.
    scene.assign(rect, "x", caught_at).unwrap();
    let mut overshot = false;
    for _ in 0..200 {
        scene.tick_animations(Duration::from_millis(16)).unwrap();
        overshot |= scene.number(rect, "x").unwrap() > caught_at + 1.0;
    }
    assert!(overshot, "the spring inherited the fling's velocity");
}

#[test]
fn decay_is_a_verb_rather_than_a_behavior() {
    // A behavior answers "when this is assigned, how does it travel there".
    // Decay has no there, so installing it as one would be motion no
    // assignment could start.
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let error = scene
        .set_physics(rect, "x", Some(coasting(500.0)))
        .unwrap_err();
    assert!(format!("{error:?}").contains("fling"), "{error:?}");

    // And the verb refuses the specs that do pursue a target.
    let error = scene
        .fling(rect, "x", 100.0, Physics::Smoothed { velocity: 10.0 })
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("pursues a target"),
        "{error:?}"
    );
}

#[test]
fn gravity_falls_and_a_bounce_returns_less_each_time() {
    // With gravity the motion is not going anywhere either — but it does not
    // stop when it runs out of speed, because the top of a bounce is
    // momentarily still. It stops when it comes to rest against the floor.
    let mut scene = Scene::new();
    let ball = scene.create(Element::Rect);
    scene.assign(ball, "y", 0.0).unwrap();
    scene
        .fling(
            ball,
            "y",
            0.0,
            Physics::Decay {
                friction: 0.0,
                min_velocity: 12.0,
                bounds: Some((0.0, 300.0)),
                gravity: 2000.0,
                restitution: 0.6,
            },
        )
        .unwrap();

    // It falls, and the peaks it returns to get lower.
    let mut peaks = Vec::new();
    let mut rising = false;
    let mut previous = 0.0;
    let mut settled = false;
    for _ in 0..600 {
        let frame = scene.tick_animations(Duration::from_millis(8)).unwrap();
        let y = scene.number(ball, "y").unwrap();
        assert!(
            (0.0..=300.0).contains(&y),
            "the floor and ceiling hold: {y}"
        );
        if y < previous && !rising {
            rising = true;
        } else if y > previous && rising {
            rising = false;
            peaks.push(previous);
        }
        previous = y;
        if !frame.active {
            settled = true;
            break;
        }
    }

    assert!(settled, "it came to rest rather than bouncing forever");
    assert_eq!(
        scene.number(ball, "y").unwrap(),
        300.0,
        "resting on the floor"
    );
    assert!(peaks.len() >= 2, "it bounced more than once: {peaks:?}");
    for pair in peaks.windows(2) {
        assert!(pair[1] > pair[0], "each bounce returns less far: {peaks:?}");
    }
}

#[test]
fn a_perfect_bounce_keeps_going_and_a_dead_one_stops_at_once() {
    let drop = |restitution: f64| {
        let mut scene = Scene::new();
        let ball = scene.create(Element::Rect);
        scene
            .fling(
                ball,
                "y",
                600.0,
                Physics::Decay {
                    friction: 0.0,
                    min_velocity: 5.0,
                    bounds: Some((0.0, 200.0)),
                    gravity: 0.0,
                    restitution,
                },
            )
            .unwrap();
        let mut frames = 0;
        for _ in 0..400 {
            let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
            frames += 1;
            if !frame.active {
                break;
            }
        }
        (frames, scene.number(ball, "y").unwrap())
    };

    // Restitution of zero: the bound takes every bit of speed.
    let (dead_frames, dead_rest) = drop(0.0);
    assert_eq!(dead_rest, 200.0);
    // A full bounce comes back off the wall instead, so it is still going.
    let (live_frames, _) = drop(1.0);
    assert!(
        live_frames > dead_frames,
        "a bounce lasts longer than a stop: {live_frames} against {dead_frames}"
    );
}

#[test]
fn physics_moves_the_layout_revision_like_any_other_geometry_change() {
    // Physics writes a property with no assignment behind it, so it is the one
    // path that has to invalidate the layout on its own. When it did not, a
    // paint reused the layout it already held and drew every frame at the
    // starting positions — the scene animated behind a still picture.
    let mut scene = Scene::new();
    let ball = scene.create(Element::Rect);
    scene.fling(ball, "x", 400.0, coasting(200.0)).unwrap();

    let before = scene.layout_revision();
    scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert!(
        scene.number(ball, "x").unwrap() > 0.0,
        "the fling moved the property"
    );
    assert_ne!(
        scene.layout_revision(),
        before,
        "a moved box has to invalidate the layout"
    );

    // And a physics-driven property layout never reads does not — in a scene of
    // its own, because the fling above is still running and would move it.
    let mut scene = Scene::new();
    let fade = scene.create(Element::Rect);
    scene
        .fling(
            fade,
            "opacity",
            0.5,
            Physics::Decay {
                friction: 1.0,
                min_velocity: 0.001,
                bounds: Some((0.0, 1.0)),
                gravity: 0.0,
                restitution: 0.0,
            },
        )
        .unwrap();
    let before = scene.layout_revision();
    scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert_eq!(
        scene.layout_revision(),
        before,
        "an easing opacity never moves a box"
    );
}

#[test]
fn an_impulse_adds_to_a_coasting_property_rather_than_replacing_its_speed() {
    let travelled = |push: f64| {
        let mut scene = Scene::new();
        let node = scene.create(Element::Rect);
        scene.fling(node, "x", 300.0, coasting(0.0)).unwrap();
        scene.tick_animations(Duration::from_millis(16)).unwrap();
        assert!(
            scene.impulse(node, "x", push).unwrap(),
            "a coasting property takes a push"
        );
        // Measured from where the push landed, so the ground already covered
        // on the way there does not count towards it.
        let pushed_at = scene.number(node, "x").unwrap();
        for _ in 0..30 {
            scene.tick_animations(Duration::from_millis(16)).unwrap();
        }
        scene.number(node, "x").unwrap() - pushed_at
    };

    // A push forwards goes further than no push, and a push backwards goes
    // less far: the two speeds add. Were the impulse setting the speed the way
    // a fling does, `forwards` and `backwards` would be symmetric about zero
    // instead of about the speed the property already had.
    let coasting_alone = travelled(0.0);
    let forwards = travelled(300.0);
    let backwards = travelled(-300.0);
    assert!(
        forwards > coasting_alone && coasting_alone > backwards,
        "the push adds: {backwards} < {coasting_alone} < {forwards}"
    );
    assert!(
        backwards.abs() < 1.0,
        "an equal and opposite push stops it dead, not reverses it: {backwards}"
    );
    assert!(
        (forwards - 2.0 * coasting_alone).abs() < 1.0,
        "doubling the speed doubles the distance: {forwards} against {coasting_alone}"
    );
}

#[test]
fn an_impulse_leaves_a_property_that_is_not_coasting_alone() {
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);

    // Nothing running: there is no speed to add to, and starting one would be
    // a fling wearing the wrong name.
    assert!(!scene.impulse(node, "x", 500.0).unwrap());
    scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert_eq!(scene.number(node, "x").unwrap(), 0.0);

    // A spring is pursuing a target, and a push that the target would simply
    // undo is not a force — it is a fight with the animation.
    scene
        .set_physics(
            node,
            "x",
            Some(Physics::Spring {
                mass: 1.0,
                damping: 18.0,
                stiffness: 200.0,
                epsilon: 0.05,
            }),
        )
        .unwrap();
    scene.assign(node, "x", Value::Number(100.0)).unwrap();
    assert!(!scene.impulse(node, "x", 500.0).unwrap());

    assert!(scene.impulse(node, "x", f64::NAN).is_err());
}

#[test]
fn a_property_can_be_written_back_to_where_it_was_before_it_was_flung() {
    // A fling is never aimed anywhere, so nothing writes the target it lands
    // on. Left stale, the target-equality guard in `assign` reads the value
    // from before the throw and silently drops a write asking for it again —
    // a reset button that does nothing, with no way to see why.
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    scene.fling(node, "x", 400.0, coasting(600.0)).unwrap();
    for _ in 0..400 {
        scene.tick_animations(Duration::from_millis(16)).unwrap();
    }
    let landed = scene.number(node, "x").unwrap();
    assert!(landed > 0.0, "the fling travelled: {landed}");
    assert_eq!(
        scene.target(node, "x").unwrap(),
        &Value::Number(landed),
        "the target followed it down"
    );

    scene.assign(node, "x", Value::Number(0.0)).unwrap();
    assert_eq!(
        scene.number(node, "x").unwrap(),
        0.0,
        "and the property can be sent home again"
    );
}

#[test]
fn a_zero_duration_animation_announces_the_geometry_it_moved() {
    // `animate_from` writes `current` itself rather than going through
    // `assign`, so it has to invalidate the layout itself too. At a zero
    // duration it installs no animation either, so nothing later would do it:
    // the property moves and every paint afterwards reuses a layout computed
    // for where it used to be.
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    let before = scene.layout_revision();
    scene
        .animate_from(node, "x", 0.0, 120.0, Behavior::default())
        .unwrap();

    assert_eq!(scene.number(node, "x").unwrap(), 120.0, "it landed at once");
    assert_ne!(
        scene.layout_revision(),
        before,
        "and said so, or the paint keeps the layout it had"
    );
}

#[test]
fn a_destroyed_node_is_reported_once_to_whoever_holds_state_for_it() {
    // Every cache keyed on a node lives in another crate. This list is the only
    // way any of them finds out a node is gone, so it has to name the whole
    // subtree, not just the node the caller asked about, and it has to hand
    // each one over exactly once.
    let mut scene = Scene::new();
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Text);
    let grandchild = scene.create(Element::Text);
    scene.reparent(child, Some(parent)).unwrap();
    scene.reparent(grandchild, Some(child)).unwrap();
    assert!(!scene.has_removed_nodes(), "nothing has died yet");

    scene.remove(parent).unwrap();
    let removed = scene.take_removed_nodes();
    assert_eq!(
        removed.len(),
        3,
        "the whole subtree is reported: {removed:?}"
    );
    assert!(removed.contains(&grandchild), "including the deepest node");
    assert!(
        scene.take_removed_nodes().is_empty(),
        "and only once — a second drain has nothing left"
    );
}

#[test]
fn replacing_a_behavior_with_physics_says_the_animation_was_canceled() {
    // Four paths replace one kind of motion with another, and two of them used
    // to tear the old one down without a word — so whether a configuration
    // heard `on_finished` for an animation it had just replaced depended on
    // which of the four it happened to use.
    let mut scene = Scene::new();
    let node = scene.create(Element::Rect);
    scene
        .set_behavior(
            node,
            "x",
            Some(Behavior::timed(Duration::from_millis(200), Easing::Linear)),
        )
        .unwrap();
    scene.assign(node, "x", Value::Number(100.0)).unwrap();
    let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert!(frame.active, "the animation is running");

    scene
        .set_physics(node, "x", Some(Physics::Smoothed { velocity: 400.0 }))
        .unwrap();
    let frame = scene.tick_animations(Duration::from_millis(16)).unwrap();
    assert!(
        frame
            .events
            .iter()
            .any(|event| event.end == AnimationEnd::Canceled),
        "and the configuration is told it was replaced: {:?}",
        frame.events
    );
}
