use crate::*;
use morf_scene::{Easing, Value as SceneValue};
use std::time::Duration;

#[test]
fn repeater_builds_one_delegate_per_model_entry() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "repeater.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local model = morf.list_model({ "one", "two", "three" })
                ui.Repeater {
                    model = model,
                    delegate = function(item) return ui.Text { text = item } end,
                }
            "#,
        )
        .unwrap();
    let scene = runtime.scene();
    let children = scene.children(scene.roots()[0]).unwrap();

    assert_eq!(children.len(), 3);
    assert_eq!(scene.string_value(children[2], "text").unwrap(), "three");
}

#[test]
fn a_flicked_view_coasts_on_the_engine_clock_within_its_bounds() {
    // There used to be a second momentum mechanism for this — `morf.flickable`,
    // a Lua-clocked integrator with its own decay law — living beside the
    // fling. A `Flickable`'s content offset is an ordinary scene property, so
    // the fling already does the job, with the engine keeping time and the
    // bounds it already understands. Two answers to "where does a flick end
    // up", and only one of them integrates on the frame clock.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "flick.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local list = ui.Flickable { width = 200, height = 400, content_y = 100 }
                morf.animation.fling {
                    node = list,
                    property = "content_y",
                    velocity = 200,
                    friction = 100,
                    min = 0,
                    max = 500,
                }
            "#,
        )
        .unwrap();
    let list = runtime.scene().roots()[0];

    for _ in 0..400 {
        runtime
            .tick_animations(std::time::Duration::from_millis(16))
            .unwrap();
    }
    let settled = runtime.scene().number(list, "content_y").unwrap();
    assert!(settled > 100.0, "it coasted onwards: {settled}");
    assert!(settled <= 500.0, "and the bound held: {settled}");
}

#[test]
fn lua_queues_parent_and_anchor_transition() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "parent.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local tile = ui.Rect { width = 20, height = 20 }
                local left = ui.Item { x = 10, width = 100, height = 100, tile }
                local right = ui.Item { x = 200, width = 100, height = 100 }
                ui.Item { left, right }
                morf.transition_parent(tile, right, {
                  duration = 300,
                  easing = "out_cubic",
                  anchors = { center_in = true },
                })
            "#,
        )
        .unwrap();

    let transitions = runtime.take_parent_transitions();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].behavior.duration, Duration::from_millis(300));
    assert_eq!(transitions[0].behavior.easing, Easing::OutCubic);
    assert_eq!(
        transitions[0].anchors.as_ref().unwrap().get("center_in"),
        Some(&SceneValue::Bool(true))
    );
}

#[test]
fn lua_named_state_animates_properties_and_queues_reparent() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "states.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local expanded = morf.signal("expanded", false)
                local shelf = ui.Item { width = 100, height = 100 }
                local page = ui.Item { x = 200, width = 200, height = 100 }
                local tile = ui.Rect {
                  states = {
                    compact = {
                      property_changes = { width = 40, height = 40 },
                      parent_change = shelf,
                    },
                    expanded = {
                      property_changes = { width = 180, height = 80 },
                      anchor_changes = { center_in = true },
                      parent_change = page,
                    },
                  },
                  transitions = {
                    {
                      from = "compact",
                      to = "expanded",
                      reversible = true,
                      duration = 200,
                      easing = "linear",
                    },
                  },
                  state = function()
                    return expanded:get() and "expanded" or "compact"
                  end,
                }
                ui.Item { shelf, page }
                local ok, err = expanded:set(true)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let children = runtime.scene().children(root).unwrap().to_vec();
    let tile = runtime.scene().children(children[0]).unwrap()[0];

    assert_eq!(runtime.scene().number(tile, "width").unwrap(), 40.0);
    assert_eq!(
        runtime.scene().target(tile, "width").unwrap(),
        &SceneValue::Number(180.0)
    );
    let transitions = runtime.take_parent_transitions();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].parent, children[1]);
    runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(runtime.scene().number(tile, "width").unwrap(), 110.0);
}

#[test]
fn state_property_bindings_recapture_dependencies() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "state-binding.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local size = morf.signal("size", 40)
                ui.Rect {
                  states = {
                    active = {
                      property_changes = {
                        width = function() return size:get() end,
                      },
                    },
                  },
                  state = function() return "active" end,
                }
                local ok, err = size:set(80)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 80.0);
}
