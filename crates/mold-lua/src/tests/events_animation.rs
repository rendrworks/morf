#[test]
fn lua_dbus_arguments_preserve_positional_lists() {
    let mut runtime = Runtime::default();
    let value = runtime.lua.enter(|ctx| {
        let arguments = Table::new(&ctx);
        arguments.set(ctx, 1, "device").unwrap();
        let typed = Table::new(&ctx);
        typed.set_field(ctx, "signature", "u");
        typed.set_field(ctx, "value", 7_i64);
        arguments.set(ctx, 2, typed).unwrap();
        arguments.set(ctx, 3, true).unwrap();
        lua_to_dbus(ctx, LuaValue::Table(arguments), 0)
    });

    assert_eq!(
        value.unwrap(),
        DbusValue::List(vec![
            DbusValue::String("device".to_owned()),
            DbusValue::Typed {
                signature: "u".to_owned(),
                value: Box::new(DbusValue::Integer(7)),
            },
            DbusValue::Bool(true),
        ])
    );
}

#[test]
fn handler_fuel_failure_is_nonfatal() {
    let mut runtime = Runtime::new(Limits {
        effect_fuel: 1_000,
        slice_fuel: 64,
        ..Limits::default()
    });
    runtime
        .execute(
            "handler.lua",
            br#"
                local ui = require("mold.ui")
                ui.MouseArea { on_clicked = function() while true do end end }
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];

    assert!(runtime.dispatch_ui_event(node, UiEvent::Clicked));
    assert!(runtime.take_logs()[0].contains("handler fuel exhausted"));
    assert!(runtime.scene().contains(node));
}

#[test]
fn touch_handlers_receive_contact_identity_and_coordinates() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "touch.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local status = mold.signal("touch.status", "idle")
                ui.MouseArea {
                  width = 100,
                  height = 100,
                  on_touch_pressed = function(id, x, y)
                    status:set(string.format("down:%d:%.0f:%.0f", id, x, y))
                  end,
                  on_touch_moved = function(id, x, y)
                    status:set(string.format("move:%d:%.0f:%.0f", id, x, y))
                  end,
                  on_touch_released = function(id)
                    status:set("up:" .. id)
                  end,
                  ui.Text { text = function() return status:get() end },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = runtime.scene().children(root).unwrap()[0];

    assert!(runtime.dispatch_touch_event(root, UiEvent::TouchPressed, 7, 12.0, 18.0));
    assert_eq!(
        runtime.scene().string_value(text, "text").unwrap(),
        "down:7:12:18"
    );
    assert!(runtime.dispatch_touch_event(root, UiEvent::TouchMoved, 7, 20.0, 30.0));
    assert_eq!(
        runtime.scene().string_value(text, "text").unwrap(),
        "move:7:20:30"
    );
    assert!(runtime.dispatch_touch_event(root, UiEvent::TouchReleased, 7, 20.0, 30.0));
    assert_eq!(runtime.scene().string_value(text, "text").unwrap(), "up:7");
}

#[test]
fn pointer_drag_handlers_receive_position_and_displacement() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "drag.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local status = mold.signal("drag.status", "idle")
                ui.MouseArea {
                  accepted_buttons = { "right" },
                  on_dragged = function(x, y, dx, dy)
                    status:set(string.format("%.0f:%.0f:%.0f:%.0f", x, y, dx, dy))
                  end,
                  ui.Text { text = function() return status:get() end },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = runtime.scene().children(root).unwrap()[0];

    assert!(!runtime.accepts_pointer_button(root, 0x110));
    assert!(runtime.accepts_pointer_button(root, 0x111));
    assert!(runtime.dispatch_pointer_event(root, UiEvent::Dragged, 20.0, 30.0, 9.0, 12.0));
    assert_eq!(
        runtime.scene().string_value(text, "text").unwrap(),
        "20:30:9:12"
    );
}

#[test]
fn wheel_handlers_receive_pixels_and_steps() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "wheel.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local status = mold.signal("wheel.status", "idle")
                ui.MouseArea {
                  on_wheel = function(x, y, dx, dy, sx, sy)
                    status:set(string.format("%.0f:%.0f:%.0f:%.0f:%d:%d", x, y, dx, dy, sx, sy))
                  end,
                  ui.Text { text = function() return status:get() end },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = runtime.scene().children(root).unwrap()[0];

    assert!(runtime.dispatch_wheel_event(root, (8.0, 12.0), (-4.0, 15.0), (-1, 2)));
    assert_eq!(
        runtime.scene().string_value(text, "text").unwrap(),
        "8:12:-4:15:-1:2"
    );
}

#[test]
fn lua_scene_errors_name_unknown_properties() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "bad-scene.lua",
            br#"
                local ui = require("mold.ui")
                ui.Text { radius = 4 }
            "#,
        )
        .unwrap_err();

    assert!(error.to_string().contains("unknown Text property `radius`"));
}

#[test]
fn lua_binding_glides_without_lua_on_animation_ticks() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "behavior.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local expanded = mold.signal("expanded", false)
                ui.Rect {
                    behavior = {
                        width = { duration = 200, easing = "linear" },
                    },
                    width = function()
                        return expanded:get() and 100 or 0
                    end,
                }
                local ok, err = expanded:set(true)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 0.0);
    assert_eq!(
        runtime.scene().target(node, "width").unwrap(),
        &SceneValue::Number(100.0)
    );
    let runs = runtime.effect_runs();

    let frame = runtime.tick_animations(Duration::from_millis(100)).unwrap();

    assert_eq!(runtime.scene().number(node, "width").unwrap(), 50.0);
    assert_eq!(runtime.effect_runs(), runs);
    assert!(frame.active);
}

#[test]
fn lua_spring_chases_a_reactive_target_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "spring.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local target = mold.signal("target", 0)
                ui.Item {
                    behavior = {
                        x = ui.spring { damping = 18, stiffness = 180 },
                    },
                    x = function() return target:get() end,
                }
                local ok, err = target:set(100)
                assert(ok, err)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    let runs = runtime.effect_runs();

    let frame = runtime.tick_animations(Duration::from_millis(50)).unwrap();

    assert!(runtime.scene().number(node, "x").unwrap() > 0.0);
    assert!(runtime.scene().number(node, "x").unwrap() < 100.0);
    assert_eq!(runtime.effect_runs(), runs);
    assert!(frame.active);
}

#[test]
fn lua_list_model_virtualizes_five_hundred_items() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "list.lua",
            br#"
                local mold = require("mold")
                local items = {}
                for index = 1, 500 do items[index] = { name = "app" .. index } end
                local model = mold.list_model(items)
                local view = mold.virtual_list(model, 40, 400, 1)
                local initial = view:sync()
                assert(#initial == 12)
                assert(initial[1].kind == "populate")
                assert(#view:visible() == 12)
                model:move(3, 8)
                local changes = view:sync()
                local moved = false
                local displaced = false
                for _, change in ipairs(changes) do
                  moved = moved or change.kind == "move"
                  displaced = displaced or change.kind == "displaced"
                end
                assert(moved and displaced)
                view:set_offset(4000)
                assert(view:visible()[1].index == 100)
            "#,
        )
        .unwrap();
}

#[test]
fn lua_list_model_reconciles_structured_values_by_property() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "reconcile.lua",
            br#"
                local mold = require("mold")
                local model = mold.list_model {
                  { id = "a", label = "first" },
                  { id = "b", label = "second" },
                  { id = "c", label = "third" },
                }
                local view = mold.virtual_list(model, 20, 100, 0)
                view:sync()
                model:replace({
                  { id = "b", label = "changed" },
                  { id = "d", label = "new" },
                  { id = "a", label = "first" },
                }, "id")
                assert(model:len() == 3)
                assert(model:get(1).id == "b")
                assert(model:get(1).label == "changed")
                assert(model:get(2).id == "d")
                assert(model:index_of(model:get(2)) == 2)
                assert(model:index_of({ id = "missing" }) == nil)
                local moved, added, removed = false, false, false
                for _, change in ipairs(view:sync()) do
                  moved = moved or change.kind == "move"
                  added = added or change.kind == "add"
                  removed = removed or change.kind == "remove"
                end
                assert(moved and added and removed)
            "#,
        )
        .unwrap();
}

#[test]
fn list_view_builds_only_visible_lua_delegates() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "list-view.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local items = {}
                for index = 1, 500 do items[index] = "app" .. index end
                local model = mold.list_model(items)
                local delegate_runs = 0
                local updater_runs = 0
                local view = ui.ListView {
                    model = model,
                    height = 400,
                    item_extent = 40,
                    overscan = 1,
                    content_y = 4000,
                    delegate = function(item, index)
                        delegate_runs = delegate_runs + 1
                        local node = ui.Text { text = item, width = 100, height = 40 }
                        return node, function(next_item)
                            updater_runs = updater_runs + 1
                            node.text = next_item
                        end
                    end,
                }
                model:set(100, "changed")
                mold.sync_view(view, 4000)
                mold.sync_view(view, 8000)
                mold.sync_view(view, 4000)
                mold.sync_view(view, 12000)
                assert(delegate_runs == 27)
                assert(updater_runs == 13)
            "#,
        )
        .unwrap();
    let scene = runtime.scene();
    let root = scene.roots()[0];
    let children = scene.children(root).unwrap();

    assert_eq!(children.len(), 14);
    assert_eq!(scene.string_value(children[1], "text").unwrap(), "app300");
    assert_eq!(scene.number(children[1], "y").unwrap(), -40.0);
    assert!(scene.bool_value(root, "clip").unwrap());
}

#[test]
fn grid_view_virtualizes_complete_rows_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "grid-view.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local items = {}
                for index = 1, 500 do items[index] = "tile" .. index end
                ui.GridView {
                  model = mold.list_model(items),
                  width = 400,
                  height = 200,
                  cell_width = 100,
                  cell_height = 50,
                  columns = 4,
                  overscan = 1,
                  content_y = 75,
                  delegate = function(item)
                    return ui.Text { text = item, width = 100, height = 50 }
                  end,
                }
            "#,
        )
        .unwrap();
    let scene = runtime.scene();
    let root = scene.roots()[0];
    let children = scene.children(root).unwrap();

    assert_eq!(children.len(), 28);
    assert_eq!(scene.string_value(children[5], "text").unwrap(), "tile6");
    assert_eq!(scene.number(children[5], "x").unwrap(), 100.0);
    assert_eq!(scene.number(children[5], "y").unwrap(), -25.0);
    assert!(scene.bool_value(root, "clip").unwrap());
}

