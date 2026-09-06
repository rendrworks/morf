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

#[test]
fn a_repeater_follows_its_model_and_keeps_the_rows_it_already_had() {
    // Built once and never touched again was the whole behaviour: a model
    // replaced under a Repeater changed the model's count and nothing on
    // screen. Now the frame loop reconciles it by item identity, so a row
    // that stayed keeps its node, a row that went is gone, and the order on
    // screen is the model's order.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "repeater.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local model = morf.list_model({ "one", "two", "three" })
                ui.Repeater {
                    as = "column",
                    model = model,
                    delegate = function(item) return ui.Text { text = item } end,
                }
                morf.ipc.reorder = function()
                    model:replace({ "three", "one", "four" })
                end
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let before = runtime.scene().children(root).unwrap().to_vec();
    assert_eq!(before.len(), 3);
    assert_eq!(
        runtime.scene().element(root).unwrap(),
        morf_scene::Element::Column
    );

    runtime.call_ipc("reorder", &[]).unwrap();
    assert!(runtime.poll_services());

    let scene = runtime.scene();
    let after = scene.children(root).unwrap();
    let texts = after
        .iter()
        .map(|node| scene.string_value(*node, "text").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["three", "one", "four"]);
    // "one" and "three" kept their nodes; "two" is gone; "four" is new.
    assert_eq!(after[0], before[2]);
    assert_eq!(after[1], before[0]);
    assert!(!scene.contains(before[1]));
}

#[test]
fn a_repeater_delegate_with_an_updater_is_patched_rather_than_rebuilt() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "repeater-update.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local model = morf.list_model({ { id = "a", label = "A" }, { id = "b", label = "B" } })
                ui.Repeater {
                    model = model,
                    delegate = function(item)
                        local text = ui.Text { text = item.label }
                        return text, function(next) text.text = next.label end
                    end,
                }
                morf.ipc.rename = function()
                    model:replace({ { id = "a", label = "A2" }, { id = "b", label = "B" } }, "id")
                end
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let before = runtime.scene().children(root).unwrap().to_vec();

    runtime.call_ipc("rename", &[]).unwrap();
    runtime.poll_services();

    let scene = runtime.scene();
    let after = scene.children(root).unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], before[0]);
    assert_eq!(scene.string_value(after[0], "text").unwrap(), "A2");
    assert_eq!(scene.string_value(after[1], "text").unwrap(), "B");
}

#[test]
fn a_state_with_when_selects_itself_and_default_takes_over_otherwise() {
    // Hover and press used to be a signal each, written from four handlers
    // and read by a `state` binding somebody wrote by hand. A state that
    // says `when` chooses itself; `default` is where the node goes when
    // none does.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "when.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local hover = morf.signal("hover", false)
                local down = morf.signal("down", false)
                ui.Rect {
                    width = 10, height = 10, color = "#000000",
                    states = {
                        default = { property_changes = { width = 10 } },
                        hovered = {
                            when = function() return hover:get() end,
                            property_changes = { width = 20 },
                        },
                        pressed = {
                            when = function() return down:get() end,
                            property_changes = { width = 5 },
                        },
                    },
                }
                morf.ipc.hover = function(on) hover:set(on) end
                morf.ipc.press = function(on) down:set(on) end
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 10.0);

    runtime
        .call_ipc("hover", &[IpcValue::Boolean(true)])
        .unwrap();
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 20.0);
    // Both true: name order, and `hovered` sorts before `pressed`.
    runtime
        .call_ipc("press", &[IpcValue::Boolean(true)])
        .unwrap();
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 20.0);
    runtime
        .call_ipc("hover", &[IpcValue::Boolean(false)])
        .unwrap();
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 5.0);
    runtime
        .call_ipc("press", &[IpcValue::Boolean(false)])
        .unwrap();
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 10.0);
}

#[test]
fn a_when_state_and_a_state_binding_together_are_refused() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "when-conflict.lua",
            br#"
                local ui = require("morf.ui")
                ui.Rect {
                    states = { a = { when = function() return true end, property_changes = {} } },
                    state = "a",
                }
            "#,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("choose themselves"), "{error}");
}

#[test]
fn a_state_with_order_is_asked_before_its_alphabetical_betters() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "order.lua",
            br#"
                local ui = require("morf.ui")
                ui.Rect {
                    width = 10, height = 10,
                    states = {
                        alpha = { when = function() return true end, property_changes = { width = 1 } },
                        omega = { order = -1, when = function() return true end, property_changes = { width = 2 } },
                    },
                }
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 2.0);
}

#[test]
fn a_repeater_can_lay_its_rows_out_as_a_flex() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "repeater-flex.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                ui.Repeater {
                    as = "flex", direction = "column", gap = 4,
                    model = morf.list_model({ "a", "b" }),
                    delegate = function(item) return ui.Text { text = item } end,
                }
            "#,
        )
        .unwrap();
    let scene = runtime.scene();
    let root = scene.roots()[0];
    assert_eq!(scene.element(root).unwrap(), morf_scene::Element::Flex);
    assert_eq!(scene.string_value(root, "direction").unwrap(), "column");
    assert_eq!(scene.children(root).unwrap().len(), 2);
}

#[test]
fn a_ui_kind_that_does_not_exist_is_named() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "no-kind.lua",
            br#"
                local ui = require("morf.ui")
                ui.RowLayout { }
            "#,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("no ui kind `RowLayout`"), "{error}");
}
