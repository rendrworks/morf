use crate::*;
use mold_scene::NodeHandle;
use std::time::Duration;

use super::*;

#[test]
fn lua_plays_a_sequence_of_property_steps() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "group.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local card = ui.Item { x = 0, y = 0, opacity = 0 }
                _G.card = card
                _G.group = mold.animation.play {
                    { node = card, property = "opacity", to = 1, duration = 100 },
                    { pause = 50 },
                    { parallel = {
                        { node = card, property = "x", to = 200, duration = 100 },
                        { node = card, property = "y", to = 100, duration = 100 },
                    }},
                    on_finished = function(reason) _G.reason = reason end,
                }
                assert(_G.group:active())
            "#,
        )
        .unwrap();
    let card = runtime.scene().roots()[0];

    // Only the first step is under way.
    runtime.tick_animations(Duration::from_millis(50)).unwrap();
    assert!(runtime.scene().is_animating(card, "opacity").unwrap());
    assert!(!runtime.scene().is_animating(card, "x").unwrap());

    // Past the first step and its pause, the parallel leg runs both at once.
    runtime.tick_animations(Duration::from_millis(110)).unwrap();
    assert_eq!(runtime.scene().number(card, "opacity").unwrap(), 1.0);
    assert!(runtime.scene().is_animating(card, "x").unwrap());
    assert!(runtime.scene().is_animating(card, "y").unwrap());

    let frame = runtime.tick_animations(Duration::from_millis(150)).unwrap();
    assert_eq!(runtime.scene().number(card, "x").unwrap(), 200.0);
    assert_eq!(runtime.scene().number(card, "y").unwrap(), 100.0);
    assert_eq!(frame.groups.len(), 1);
    assert_eq!(frame.groups[0].end, mold_scene::AnimationEnd::Completed);

    runtime
        .execute(
            "assert.lua",
            br#"
                assert(_G.reason == "completed", "group did not report completion")
                assert(not _G.group:active(), "group is still active")
            "#,
        )
        .unwrap();
}

#[test]
fn lua_stops_and_finishes_a_group_through_its_handle() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "control.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local card = ui.Item { x = 0 }
                _G.card = card
                _G.group = mold.animation.play {
                    { node = card, property = "x", to = 100, duration = 200 },
                    { node = card, property = "y", to = 50, duration = 200 },
                }
            "#,
        )
        .unwrap();
    let card = runtime.scene().roots()[0];
    runtime.tick_animations(Duration::from_millis(50)).unwrap();

    runtime
        .execute(
            "finish.lua",
            br#"
                local mold = require("mold")
                assert(_G.group:finish(), "finish did not act on a running group")
                assert(not _G.group:active())
                -- The handle stays usable after the group is gone and simply
                -- reports that there was nothing left to act on.
                assert(not _G.group:stop())
            "#,
        )
        .unwrap();
    assert_eq!(runtime.scene().number(card, "x").unwrap(), 100.0);
    assert_eq!(runtime.scene().number(card, "y").unwrap(), 50.0);
}

#[test]
fn a_group_specification_is_validated_where_it_is_declared() {
    let mut runtime = Runtime::default();
    let rejected = |source: &'static [u8]| {
        let mut runtime = Runtime::default();
        runtime.execute("bad.lua", source).is_err()
    };

    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "not_a_property", to = 1, duration = 100 },
            }
        "#
    ));
    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "x", to = 1 },
            }
        "#
    ));
    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "x", to = 1, duration = 100, loops = "forever" },
            }
        "#
    ));

    // A well-formed group is still accepted after the rejections above.
    runtime
        .execute(
            "good.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local handle = mold.animation.play {
                    loops = "forever",
                    { node = ui.Item {}, property = "x", to = 1, duration = 100 },
                }
                assert(handle:active())
            "#,
        )
        .unwrap();
}

#[test]
fn a_keyframe_track_runs_one_property_through_its_stops() {
    // The documented gap: several waypoints over one property, each segment
    // with its own curve, expressed as fractions of one duration so a stop can
    // be moved without recomputing the ones after it.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "keyframes.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local node = ui.Rect { width = 10, height = 10, x = 0 }
                mold.ipc["track.start"] = function()
                  mold.animation.play {
                    {
                      node = node,
                      property = "x",
                      duration = 1000,
                      keyframes = {
                        { at = 0.0, value = 0 },
                        { at = 0.25, value = 100, easing = "linear" },
                        { at = 0.5, value = 40, easing = "linear" },
                        { at = 1.0, value = 400, easing = "linear" },
                      },
                    },
                  }
                end
            "#,
        )
        .unwrap();
    let node = {
        let scene = runtime.scene();
        scene.roots()[0]
    };
    runtime.call_ipc("track.start", &[]).unwrap();

    let x = |runtime: &Runtime| runtime.scene().number(node, "x").unwrap();
    // A linear segment is checkable at its midpoint, which is what makes the
    // stops rather than the endpoints the thing under test: the track goes up,
    // back down, then up again, so a single interpolation cannot produce it.
    runtime
        .tick_animations(std::time::Duration::from_millis(125))
        .unwrap();
    assert!((x(&runtime) - 50.0).abs() < 1.0, "{}", x(&runtime));
    runtime
        .tick_animations(std::time::Duration::from_millis(125))
        .unwrap();
    assert!((x(&runtime) - 100.0).abs() < 1.0, "{}", x(&runtime));
    // Second segment falls back to 40.
    runtime
        .tick_animations(std::time::Duration::from_millis(250))
        .unwrap();
    assert!((x(&runtime) - 40.0).abs() < 1.0, "{}", x(&runtime));
    // Third segment climbs to 400 over the remaining half.
    runtime
        .tick_animations(std::time::Duration::from_millis(250))
        .unwrap();
    assert!((x(&runtime) - 220.0).abs() < 2.0, "{}", x(&runtime));
    runtime
        .tick_animations(std::time::Duration::from_millis(300))
        .unwrap();
    assert!((x(&runtime) - 400.0).abs() < 1.0, "{}", x(&runtime));
}

#[test]
fn a_keyframe_track_rejects_the_shapes_it_cannot_run() {
    for (track, expected) in [
        ("{ { at = 0, value = 0 } }", "at least two stops"),
        (
            "{ { at = 0, value = 0 }, { at = 1.5, value = 1 } }",
            "outside zero through one",
        ),
        (
            "{ { at = 0.8, value = 0 }, { at = 0.2, value = 1 } }",
            "out of order",
        ),
    ] {
        let mut runtime = Runtime::default();
        let source = format!(
            r#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local node = ui.Rect {{ width = 10, height = 10, x = 0 }}
                mold.ipc["track.start"] = function()
                  mold.animation.play {{
                    {{ node = node, property = "x", duration = 1000, keyframes = {track} }},
                  }}
                end
            "#
        );
        runtime.execute("bad.lua", source.as_bytes()).unwrap();
        let error = runtime.call_ipc("track.start", &[]).unwrap_err();
        assert!(
            format!("{error}").contains(expected),
            "expected `{expected}` in: {error}"
        );
    }
}

#[test]
fn a_fling_from_lua_coasts_and_respects_its_bounds() {
    // A flick is thrown, not sent somewhere: the configuration hands over a
    // speed and the engine decides where that lands.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "fling.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local panel = ui.Rect { width = 100, height = 100, x = 0 }
                mold.ipc["throw"] = function(velocity)
                  mold.animation.fling {
                    node = panel,
                    property = "x",
                    velocity = velocity,
                    preset = "snappy",
                    min = 0,
                    max = 240,
                  }
                end
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    runtime
        .call_ipc("throw", &[IpcValue::Number(900.0)])
        .unwrap();
    // It is moving, and it is not there yet.
    runtime
        .tick_animations(std::time::Duration::from_millis(16))
        .unwrap();
    let early = runtime.scene().number(node, "x").unwrap();
    assert!(early > 0.0 && early < 240.0, "coasting: {early}");

    // It comes to rest on its own.
    let mut settled = None;
    for _ in 0..300 {
        let frame = runtime
            .tick_animations(std::time::Duration::from_millis(16))
            .unwrap();
        if !frame.active {
            settled = Some(runtime.scene().number(node, "x").unwrap());
            break;
        }
    }
    let settled = settled.expect("the fling settled");
    assert!(settled > early, "it carried on after the first frame");
    assert!(settled <= 240.0, "the bound held: {settled}");
}

#[test]
fn a_fling_rejects_what_it_cannot_act_on() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "bad.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local panel = ui.Rect { width = 10, height = 10 }
                mold.ipc["preset"] = function()
                  mold.animation.fling {
                    node = panel, property = "x", velocity = 10, preset = "springy",
                  }
                end
                mold.ipc["half_bound"] = function()
                  mold.animation.fling {
                    node = panel, property = "x", velocity = 10, min = 0,
                  }
                end
                mold.ipc["not_numeric"] = function()
                  mold.animation.fling {
                    node = panel, property = "color", velocity = 10,
                  }
                end
            "#,
        )
        .unwrap();

    for (name, expected) in [
        ("preset", "unknown fling preset"),
        ("half_bound", "needs both"),
        ("not_numeric", "not numeric"),
    ] {
        let error = runtime.call_ipc(name, &[]).unwrap_err();
        assert!(
            format!("{error}").contains(expected),
            "expected `{expected}` in: {error}"
        );
    }
}

#[test]
fn an_impulse_from_lua_pushes_a_coasting_node_without_taking_it_over() {
    // What a configuration needs to express a force — one shape pulling on
    // another, a drift towards the middle — without becoming the clock. It
    // computes the push at whatever rate suits it and the engine keeps moving
    // the node every frame in between.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "impulse.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local panel = ui.Rect { width = 100, height = 100, x = 0 }
                mold.ipc["throw"] = function()
                  mold.animation.fling {
                    node = panel, property = "x", velocity = 300,
                    friction = 0, min_velocity = 0,
                  }
                end
                mold.ipc["push"] = function(delta)
                  return mold.animation.impulse(panel, "x", delta)
                end
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    fn run(runtime: &mut Runtime, node: NodeHandle, frames: usize) -> f64 {
        for _ in 0..frames {
            runtime
                .tick_animations(std::time::Duration::from_millis(16))
                .unwrap();
        }
        runtime.scene().number(node, "x").unwrap()
    }

    // Nothing is coasting yet, so there is no speed to add to.
    assert_eq!(
        runtime
            .call_ipc("push", &[IpcValue::Number(300.0)])
            .unwrap(),
        vec![IpcValue::Boolean(false)]
    );
    runtime.call_ipc("throw", &[]).unwrap();
    let alone = run(&mut runtime, node, 10);

    // The same ten frames again, but pushed to twice the speed at the start of
    // them, cover twice the ground: the push added to the speed rather than
    // replacing it, and the node kept moving on its own afterwards.
    runtime.call_ipc("throw", &[]).unwrap();
    let restarted = runtime.scene().number(node, "x").unwrap();
    assert_eq!(
        runtime
            .call_ipc("push", &[IpcValue::Number(300.0)])
            .unwrap(),
        vec![IpcValue::Boolean(true)]
    );
    let pushed = run(&mut runtime, node, 10) - restarted;
    assert!(
        (pushed - 2.0 * alone).abs() < 1.0,
        "the push doubled the speed: {pushed} against {alone}"
    );
}
