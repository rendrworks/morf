#[test]
fn inset_is_native_and_rejects_ambiguous_children() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "inset.lua",
            br#"
                local ui = require("mold.ui")
                ui.Inset {
                    margin = 8,
                    left_margin = 12,
                    ui.Text { text = "content" },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().element(root).unwrap(), Element::Inset);
    assert_eq!(runtime.scene().number(root, "margin").unwrap(), 8.0);
    assert_eq!(runtime.scene().children(root).unwrap().len(), 1);

    let error = Runtime::default()
        .execute(
            "ambiguous-inset.lua",
            br#"
                local ui = require("mold.ui")
                ui.Inset { ui.Item {}, ui.Item {} }
            "#,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Inset accepts at most one child")
    );
}

#[test]
fn clip_rect_is_native_and_clips_by_default() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "clip-rect.lua",
            br#"
                local ui = require("mold.ui")
                ui.ClipRect {
                    border_width = 2,
                    content_under_border = true,
                    content_inside_border = false,
                    antialiasing = false,
                    border_pixel_aligned = false,
                    ui.Item {},
                }
            "#,
        )
        .unwrap();

    let root = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().element(root).unwrap(), Element::ClipRect);
    assert!(runtime.scene().bool_value(root, "clip").unwrap());
    assert!(
        !runtime
            .scene()
            .bool_value(root, "content_inside_border")
            .unwrap()
    );
    assert!(
        runtime
            .scene()
            .bool_value(root, "content_under_border")
            .unwrap()
    );
    assert!(!runtime.scene().bool_value(root, "antialiasing").unwrap());
    assert!(
        !runtime
            .scene()
            .bool_value(root, "border_pixel_aligned")
            .unwrap()
    );
}

#[test]
fn transform_watcher_dispatches_after_rendered_geometry_changes() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "transform.lua",
            br#"
                local mold = require("mold")
                local core = require("mold.core")
                local ui = require("mold.ui")
                local calls = mold.signal("transform.calls", 0)
                local child = ui.Item { implicit_width = 20, implicit_height = 10 }
                local root = ui.Item { child }
                local watcher = core.transform_watcher {
                  a = root,
                  b = child,
                  common_parent = root,
                  on_changed = function(revision) calls:set(revision) end,
                }
                mold.ipc["transform.state"] = function()
                  return watcher:revision(), calls:get()
                end
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let child = runtime.scene().children(root).unwrap()[0];
    let layout = Layout::compute(
        &runtime.scene(),
        root,
        mold_layout::Size {
            width: 100.0,
            height: 50.0,
        },
        &mut NoText,
    )
    .unwrap();
    assert!(!runtime.observe_layout(&layout));

    runtime.scene_mut().assign(child, "x", 12.0).unwrap();
    let layout = Layout::compute(
        &runtime.scene(),
        root,
        mold_layout::Size {
            width: 100.0,
            height: 50.0,
        },
        &mut NoText,
    )
    .unwrap();
    assert!(runtime.observe_layout(&layout));
    assert!(runtime.poll_services());
    assert_eq!(
        runtime.call_ipc("transform.state", &[]).unwrap(),
        [IpcValue::Integer(1), IpcValue::Integer(1)]
    );

    let error = Runtime::default()
        .execute(
            "invalid-transform.lua",
            br#"
                local core = require("mold.core")
                local ui = require("mold.ui")
                local a = ui.Item {}
                local b = ui.Item {}
                core.transform_watcher { a = a, b = b, common_parent = a }
            "#,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("common_parent must contain both")
    );
}

#[test]
fn lua_constructs_image_icon_and_shape_elements() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "images.lua",
            br#"
                local ui = require("mold.ui")
                ui.Item {
                    ui.Image { source = "/tmp/picture.png", width = 64, height = 32 },
                    ui.Icon { name = "battery", theme = "hicolor", width = 24, height = 24 },
                    ui.Shape {
                      path = "M0 0 L16 0 L8 16 Z",
                      fill_color = "white",
                      stroke_width = 1,
                    },
                }
            "#,
        )
        .unwrap();

    let root = runtime.scene().roots()[0];
    let children = runtime.scene().children(root).unwrap();
    assert_eq!(
        runtime.scene().element(children[0]).unwrap(),
        Element::Image
    );
    assert_eq!(runtime.scene().element(children[1]).unwrap(), Element::Icon);
    assert_eq!(
        runtime.scene().element(children[2]).unwrap(),
        Element::Shape
    );
}

#[test]
fn clock_service_recomputes_text_bindings() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "clock.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                ui.Text { text = function() return mold.clock:get() end }
            "#,
        )
        .unwrap();
    runtime.update_clock("12:34:56").unwrap();

    let node = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().string_value(node, "text").unwrap(),
        "12:34:56"
    );
}

#[test]
fn mouse_area_emits_clicked() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "button.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local count = mold.signal("count", 0)
                ui.Item {
                    ui.Text { text = function() return "" .. count:get() end },
                    ui.MouseArea {
                        width = 80,
                        height = 24,
                        accepted_buttons = { "right", 274 },
                        on_clicked = function() count:set(count:get() + 1) end,
                    },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let children = runtime.scene().children(root).unwrap();

    assert!(!runtime.accepts_pointer_button(children[1], 0x110));
    assert!(runtime.accepts_pointer_button(children[1], 0x111));
    assert!(runtime.accepts_pointer_button(children[1], 0x112));
    assert!(runtime.dispatch_ui_event(children[1], UiEvent::Clicked));

    assert_eq!(
        runtime.scene().string_value(children[0], "text").unwrap(),
        "1"
    );
}

#[test]
fn key_handlers_receive_keysym_and_text() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "key.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local value = mold.signal("key", "")
                ui.Item {
                    ui.MouseArea {
                        width = 100,
                        height = 40,
                        on_key_pressed = function(keysym, text)
                            value:set(keysym .. ":" .. text)
                        end,
                    },
                    ui.Text { text = function() return value:get() end },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let children = runtime.scene().children(root).unwrap();

    assert!(runtime.dispatch_key_event(children[0], 65, Some("A")));
    assert_eq!(
        runtime.scene().string_value(children[1], "text").unwrap(),
        "65:A"
    );
}

#[test]
fn keyboard_focus_routes_ancestors_and_cycles() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "focus.lua",
            br#"
                local ui = require("mold.ui")
                ui.Item {
                  ui.MouseArea {
                    ui.Rect {},
                    on_key_pressed = function() end,
                  },
                  ui.MouseArea {
                    focus = true,
                    on_key_pressed = function() end,
                  },
                  ui.MouseArea {
                    enabled = false,
                    on_key_pressed = function() end,
                  },
                }
                ui.Item {
                  ui.MouseArea {
                    on_key_pressed = function() end,
                  },
                }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let second_root = runtime.scene().roots()[1];
    let children = runtime.scene().children(root).unwrap();
    let second_target = runtime.scene().children(second_root).unwrap()[0];
    let nested = runtime.scene().children(children[0]).unwrap()[0];

    assert_eq!(runtime.first_key_target(), Some(children[1]));
    assert_eq!(runtime.key_target_for_node(nested), Some(children[0]));
    assert!(runtime.node_in_subtree(root, nested));
    assert!(!runtime.node_in_subtree(second_root, nested));
    assert_eq!(runtime.first_key_target_in(root), Some(children[1]));
    assert_eq!(
        runtime.first_key_target_in(second_root),
        Some(second_target)
    );
    assert_eq!(
        runtime.next_key_target(Some(children[1])),
        Some(second_target)
    );
    assert_eq!(
        runtime.next_key_target_in(root, Some(children[1])),
        Some(children[0])
    );
}

#[test]
fn pam_callbacks_return_asynchronously_to_lua() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pam.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local result = mold.signal("pam.result", "pending")
                mold.pam.authenticate("mold\0test", "user", "secret", function(ok, error)
                    result:set(ok and "ok" or error)
                end)
                ui.Text { text = function() return result:get() end }
            "#,
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.poll_services() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let root = runtime.scene().roots()[0];

    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "service contains a null byte"
    );
}

#[test]
fn failed_pam_authentication_cannot_request_unlock() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "unlock.lua",
            br#"
                local mold = require("mold")
                mold.pam.authenticate_unlock("mold\0test", "user", "secret", function() end)
            "#,
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.poll_services() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(!runtime.take_session_unlock_request());
}

#[test]
fn fluid_transform_example_animates_square_to_circle_in_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "examples/fluid-transform.lua",
            include_bytes!("../../../../examples/fluid-transform.lua"),
        )
        .unwrap();
    runtime
        .tick_animations(Duration::from_secs(2))
        .unwrap();
    let root = runtime.scene().roots()[0];
    let shape = runtime.scene().children(root).unwrap()[1];
    assert_eq!(runtime.scene().number(shape, "radius").unwrap(), 12.0);

    assert!(runtime.dispatch_ui_event(shape, UiEvent::Clicked));
    assert_eq!(
        runtime.scene().target(shape, "radius").unwrap(),
        &SceneValue::Number(60.0)
    );
    assert_eq!(
        runtime.scene().target(shape, "translate_x").unwrap(),
        &SceneValue::Number(270.0)
    );

    let frame = runtime
        .tick_animations(Duration::from_millis(16))
        .unwrap();
    let radius = runtime.scene().number(shape, "radius").unwrap();
    assert!(radius > 12.0 && radius < 60.0);
    assert!(frame.active);
    assert!(frame.changes.iter().any(|change| {
        change.node == shape
            && change.property == "translate_x"
            && change.class == mold_scene::PropertyClass::Transform
    }));
}

#[test]
fn morph_stack_example_combines_native_animation_and_geometry() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "examples/morph-stack.lua",
            include_bytes!("../../../../examples/morph-stack.lua"),
        )
        .unwrap();
    runtime
        .tick_animations(Duration::from_secs(2))
        .unwrap();
    let root = runtime.scene().roots()[0];
    let shape = runtime.scene().children(root).unwrap()[1];

    assert_eq!(
        runtime.scene().string_value(shape, "morph_from").unwrap(),
        "square"
    );
    assert_eq!(
        runtime.scene().string_value(shape, "morph_to").unwrap(),
        "circle"
    );
    assert!(runtime.dispatch_ui_event(shape, UiEvent::Clicked));
    assert_eq!(
        runtime.scene().target(shape, "morph_progress").unwrap(),
        &SceneValue::Number(1.0)
    );

    let frame = runtime
        .tick_animations(Duration::from_millis(16))
        .unwrap();
    let progress = runtime.scene().number(shape, "morph_progress").unwrap();
    assert!(progress > 0.0 && progress < 1.0);
    assert!(frame.changes.iter().any(|change| {
        change.node == shape
            && change.property == "morph_progress"
            && change.class == mold_scene::PropertyClass::Paint
    }));
}
