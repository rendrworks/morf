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
