#[test]
fn repeater_builds_one_delegate_per_model_entry() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "repeater.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local model = mold.list_model({ "one", "two", "three" })
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
fn flickable_state_drags_and_ticks_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "flickable.lua",
            br#"
                local mold = require("mold")
                local flick = mold.flickable {
                    offset = 100,
                    minimum = 0,
                    maximum = 500,
                    deceleration = 100,
                }
                assert(flick:drag_by(25) == 125)
                flick:release(200)
                local offset, active = flick:tick(100)
                assert(offset == 145 and active)
            "#,
        )
        .unwrap();
}

#[test]
fn lua_queues_parent_and_anchor_transition() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "parent.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local tile = ui.Rect { width = 20, height = 20 }
                local left = ui.Item { x = 10, width = 100, height = 100, tile }
                local right = ui.Item { x = 200, width = 100, height = 100 }
                ui.Item { left, right }
                mold.transition_parent(tile, right, {
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
                local mold = require("mold")
                local ui = require("mold.ui")
                local expanded = mold.signal("expanded", false)
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
    let children = runtime.scene().children(root).unwrap();
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
                local mold = require("mold")
                local ui = require("mold.ui")
                local size = mold.signal("size", 40)
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

