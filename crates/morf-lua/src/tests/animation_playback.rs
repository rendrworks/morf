use crate::*;
use std::time::Duration;

// Behavior repetition, lifecycle handlers, and the `morf.animation` controls.

#[test]
fn lua_declares_delay_loops_and_ping_pong_on_a_behavior() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "loops.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local target = morf.signal("target", 1)
                ui.Item {
                    behavior = {
                        opacity = {
                            duration = 100,
                            delay = 100,
                            ping_pong = true,
                            loops = 2,
                        },
                    },
                    opacity = function() return target:get() end,
                }
                local ok, err = target:set(0)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    // Nothing moves while the delay drains.
    runtime.tick_animations(Duration::from_millis(90)).unwrap();
    assert_eq!(runtime.scene().number(node, "opacity").unwrap(), 1.0);

    // The forward pass runs, then the backward pass returns it to the start.
    runtime.tick_animations(Duration::from_millis(110)).unwrap();
    assert!(runtime.scene().number(node, "opacity").unwrap() < 0.5);
    let frame = runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(runtime.scene().number(node, "opacity").unwrap(), 1.0);
    assert!(!frame.active);
}

#[test]
fn lua_receives_the_behavior_finished_handler_with_a_reason() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "finished.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local target = morf.signal("target", 0)
                _G.finished = {}
                ui.Item {
                    behavior = {
                        x = {
                            duration = 100,
                            on_finished = function(property, reason)
                                _G.finished = { property = property, reason = reason }
                            end,
                        },
                    },
                    x = function() return target:get() end,
                }
                local ok, err = target:set(100)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    let frame = runtime.tick_animations(Duration::from_millis(50)).unwrap();
    assert!(frame.events.is_empty());

    let frame = runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(runtime.scene().number(node, "x").unwrap(), 100.0);
    assert_eq!(frame.events.len(), 1);
    assert_eq!(frame.events[0].end, morf_scene::AnimationEnd::Completed);
    assert_eq!(frame.events[0].property, "x");
}

#[test]
fn lua_controls_playback_of_a_running_animation() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "control.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local target = morf.signal("target", 0)
                local node = ui.Item {
                    behavior = { x = { duration = 200 } },
                    x = function() return target:get() end,
                }
                _G.node = node
                local ok, err = target:set(100)
                assert(ok, err)
                assert(morf.animation.active(node, "x"))
                assert(not morf.animation.paused(node, "x"))
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    runtime.tick_animations(Duration::from_millis(100)).unwrap();
    let held = runtime.scene().number(node, "x").unwrap();
    assert!(held > 0.0 && held < 100.0);

    runtime
        .execute(
            "pause.lua",
            br#"
                local morf = require("morf")
                assert(morf.animation.pause(_G.node, "x"))
                assert(morf.animation.paused(_G.node, "x"))
            "#,
        )
        .unwrap();
    runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(runtime.scene().number(node, "x").unwrap(), held);

    runtime
        .execute(
            "finish.lua",
            br#"
                local morf = require("morf")
                assert(morf.animation.resume(_G.node, "x"))
                assert(morf.animation.finish(_G.node, "x"))
                assert(not morf.animation.active(_G.node, "x"))
                -- Nothing is running any more, so the controls report so
                -- instead of raising.
                assert(not morf.animation.stop(_G.node, "x"))
                assert(morf.animation.progress(_G.node, "x") == nil)
            "#,
        )
        .unwrap();
    assert_eq!(runtime.scene().number(node, "x").unwrap(), 100.0);
}

#[test]
fn lua_suppresses_one_write_by_disabling_an_installed_behavior() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "enabled.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local target = morf.signal("target", 0)
                local node = ui.Item {
                    behavior = { x = { duration = 200 } },
                    x = function() return target:get() end,
                }
                assert(morf.animation.set_enabled(node, "x", false))
                local ok, err = target:set(100)
                assert(ok, err)
                assert(not morf.animation.active(node, "x"))
                assert(morf.animation.set_enabled(node, "x", true))
                ok, err = target:set(0)
                assert(ok, err)
                assert(morf.animation.active(node, "x"))
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert!(runtime.scene().number(node, "x").unwrap() > 0.0);
}

#[test]
fn lua_evaluates_timing_curves_directly() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "easing.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")

                -- A curve evaluated by name matches the same curve used to
                -- interpolate a pair of numbers.
                local eased = morf.easing.value("in_out_cubic", 0.3)
                local between = morf.easing.number("in_out_cubic", 0.3, 0, 100)
                assert(math.abs(between - eased * 100) < 1e-9, "number disagrees with value")

                -- One curve is shared across the components, so a diagonal
                -- stays straight rather than bowing.
                local point = morf.easing.point("out_quad", 0.4, { x = 0, y = 0 }, { x = 200, y = 100 })
                assert(math.abs(point.x / 2 - point.y) < 1e-9, "point axes drifted apart")

                local rect = morf.easing.rect(
                    "linear", 0.5,
                    { x = 0, y = 0, width = 10, height = 20 },
                    { x = 10, y = 10, width = 30, height = 40 }
                )
                assert(rect.x == 5 and rect.width == 20 and rect.height == 30, "rect is wrong")

                -- A cubic Bezier table is accepted wherever a curve name is.
                local bezier = morf.easing.value({ x1 = 0.4, y1 = 0, x2 = 0.2, y2 = 1 }, 0.5)
                assert(bezier > 0 and bezier < 1, "bezier out of range")

                local color = morf.easing.color("linear", 0.5, "#000000", "#ffffff")
                assert(math.abs(color.r - 0.5) < 1e-6, "colour did not interpolate")
                assert(color.a == 1, "colour lost its alpha")

                local ok = pcall(morf.easing.value, "not_a_curve", 0.5)
                assert(not ok, "an unknown curve name must be rejected")

                ui.Item {}
            "##,
        )
        .unwrap();
    assert_eq!(runtime.scene().roots().len(), 1);
}

#[test]
fn a_declared_behavior_does_not_animate_the_element_into_existence() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "construct.lua",
            br#"
                local ui = require("morf.ui")
                ui.Item {
                    width = 400,
                    opacity = 0,
                    behavior = {
                        width = { duration = 400 },
                        opacity = { duration = 400 },
                    },
                }
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    // The declared values are in place immediately: a behavior intercepts
    // later writes, not the element's own construction.
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 400.0);
    assert_eq!(runtime.scene().number(node, "opacity").unwrap(), 0.0);
    assert!(!runtime.scene().is_animating(node, "width").unwrap());
    assert!(!runtime.scene().is_animating(node, "opacity").unwrap());

    // Nothing is moving, so the shell may idle straight away.
    let frame = runtime.tick_animations(Duration::from_millis(16)).unwrap();
    assert!(!frame.active, "the element animated its own construction");

    // A later write still animates.
    runtime.scene_mut().assign(node, "width", 100.0).unwrap();
    assert!(runtime.scene().is_animating(node, "width").unwrap());
}
