use crate::*;
use morf_layout::Layout;
use std::collections::HashSet;

use super::*;

#[test]
fn executes_a_chunk() {
    let mut runtime = Runtime::default();
    runtime
        .execute("test.lua", b"local answer = 40 + 2")
        .unwrap();
}

#[test]
fn config_chunk_results_are_discarded() {
    let mut runtime = Runtime::default();

    runtime
        .execute("result.lua", b"return { answer = 42 }")
        .unwrap();
}

#[test]
fn settings_are_assigned_and_nested_values_stay_nested() {
    let mut runtime = Runtime::default();

    runtime
        .execute(
            "settings.lua",
            br#"
                local morf = require("morf")
                assert(morf.user_render == nil)
                morf.user_render = {
                  scale_policy = "fractional",
                  damage = { enabled = true },
                }
                assert(morf.user_render.scale_policy == "fractional")
                assert(morf.user_render.damage.enabled == true)
            "#,
        )
        .unwrap();
}

#[test]
fn engine_modules_are_preloaded_by_rust() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "native-modules.lua",
            br#"
                local morf = require("morf")
                local core = require("morf.core")
                local ui = require("morf.ui")
                local io = require("morf.io")
                local window = require("morf.window")
                assert(package.loaded["morf"] == morf)
                assert(package.loaded["morf.core"] == core)
                assert(package.loaded["morf.ui"] == ui)
                assert(package.loaded["morf.io"] == io)
                assert(package.loaded["morf.window"] == window)
                assert(type(ui.Item) == "function")
                assert(core.signal == morf.signal)
                assert(io.process == morf.process)
                assert(io.dbus == nil)
                assert(window.layer_surface == morf.surface)
            "#,
        )
        .unwrap();
}

#[test]
fn layer_surface_settings_are_native_and_typed() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "surface.lua",
            br#"
                local morf = require("morf")
                morf.surface.namespace = "board"
                morf.surface.width = 1200
                morf.surface.height = 800
                morf.surface.exclusive_zone = 0
                morf.surface.anchors = { top = true, left = true }
                morf.surface.margin_top = 100
                morf.surface.margin_left = 200
                morf.surface.layer = "overlay"
                morf.surface.keyboard_focus = "none"
                assert(morf.surface.width == 1200)
                assert(morf.surface.anchors.right == false)
            "#,
        )
        .unwrap();

    assert_eq!(
        runtime.layer_surface_config(),
        LayerSurfaceConfig {
            namespace: "board".to_owned(),
            width: 1200,
            height: 800,
            exclusive_zone: 0,
            anchors: SurfaceAnchors {
                top: true,
                right: false,
                bottom: false,
                left: true,
            },
            margin_top: 100,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 200,
            layer: "overlay".to_owned(),
            keyboard_focus: "none".to_owned(),
            input_regions: None,
            reserve: SurfaceReserve::default(),
        }
    );
}

#[test]
fn surface_masks_are_native_composable_regions() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "regions.lua",
            br#"
                local morf = require("morf")
                local window = require("morf.window")
                morf.surface.width = 10
                morf.surface.height = 10
                morf.surface.mask = window.region {
                    x = 0, y = 0, width = 10, height = 10, radius = 2,
                    regions = {
                        window.region {
                            x = 4, y = 4, width = 2, height = 2,
                            intersection = "subtract",
                        },
                    },
                }
                assert(morf.surface.mask[1].shape == "rect")
                assert(morf.surface.mask[1].regions[1].intersection == "subtract")
            "#,
        )
        .unwrap();
    let regions = runtime.layer_surface_config().input_regions.unwrap();
    let rectangles = morf_region::build(10, 10, &regions).unwrap();
    assert!(!rectangles.is_empty());
    assert_eq!(
        rectangles
            .iter()
            .map(|rect| rect.width * rect.height)
            .sum::<i32>(),
        92
    );
}

#[test]
fn general_window_models_validate_popup_and_floating_state() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "windows.lua",
            br##"
                local ui = require("morf.ui")
                local window = require("morf.window")
                local popup_root = ui.Rect { color = "#111111" }
                local popup = window.popup {
                    root = popup_root,
                    visible = true,
                    width = 240,
                    height = 120,
                    anchor = { x = 10, y = 20, width = 30, height = 40 },
                    anchor_edge = "bottom_right",
                    gravity = "top_right",
                    offset_x = 4,
                    offset_y = -2,
                    constraints = { resize_x = true, flip_y = false },
                    grab_focus = true,
                    updates_enabled = false,
                }
                local floating_root = ui.Item {}
                local floating = window.floating {
                    root = floating_root,
                    width = 800,
                    height = 600,
                    minimum_width = 320,
                    minimum_height = 200,
                    maximum_width = 1920,
                    maximum_height = 1080,
                    title = "Morf Example",
                    app_id = "dev.morf.example",
                    maximized = true,
                }
                assert(popup:kind() == "popup" and popup:visible())
                assert(not popup:updates_enabled())
                assert(popup:updates_enabled(true))
                assert(popup:size().width == 240)
                assert(popup:size(260, 140).height == 140)
                assert(popup:anchor_rect().x == 10)
                assert(popup:anchor_rect(12, 22, 32, 42).width == 32)
                assert(popup:offset().x == 4)
                assert(popup:offset(6, -3).y == -3)
                assert(popup:anchor_edge() == "bottom_right")
                assert(popup:anchor_edge("top_left") == "top_left")
                assert(popup:gravity("bottom_left") == "bottom_left")
                assert(popup:grab_focus())
                assert(not popup:grab_focus(false))
                assert(popup:constraints().resize_x)
                assert(popup:constraints({ resize_x = false, flip_y = true }).flip_y)
                assert(floating:kind() == "floating" and not floating:visible())
                assert(floating:maximized())
                assert(not floating:fullscreen())
                assert(floating:fullscreen(true))
                assert(not floating:maximized(false))
                assert(floating:title() == "Morf Example")
                assert(floating:title("Changed") == "Changed")
                assert(floating:app_id("dev.morf.changed") == "dev.morf.changed")
                assert(floating:size().width == 800)
                assert(floating:size(900, 700).height == 700)
                assert(floating:minimum_size(400, 300).width == 400)
                assert(floating:maximum_size(0, 0).width == 0)
                popup:close()
                floating:open()
                assert(floating:start_system_move())
                assert(floating:start_system_resize("bottom_right"))
                assert(not pcall(floating.start_system_resize, floating, "middle"))
            "##,
        )
        .unwrap();

    let surfaces = runtime.window_surface_configs();
    assert_eq!(surfaces.len(), 2);
    assert!(!surfaces[0].visible);
    assert!(surfaces[0].updates_enabled);
    let WindowSurfaceKind::Popup(popup) = &surfaces[0].kind else {
        panic!("first surface was not a popup");
    };
    assert_eq!((popup.width, popup.height), (260, 140));
    assert_eq!((popup.anchor_x, popup.anchor_y), (12, 22));
    assert_eq!((popup.anchor_width, popup.anchor_height), (32, 42));
    assert_eq!((popup.offset_x, popup.offset_y), (6, -3));
    assert_eq!(popup.anchor_edge, "top_left");
    assert_eq!(popup.gravity, "bottom_left");
    assert!(!popup.constraints.resize_x);
    assert!(popup.constraints.flip_y);
    assert!(!popup.grab_focus);
    assert!(surfaces[1].visible);
    let WindowSurfaceKind::Floating(floating) = &surfaces[1].kind else {
        panic!("second surface was not floating");
    };
    assert_eq!(floating.title, "Changed");
    assert_eq!(floating.app_id, "dev.morf.changed");
    assert_eq!((floating.width, floating.height), (900, 700));
    assert_eq!(
        (floating.minimum_width, floating.minimum_height),
        (400, 300)
    );
    assert_eq!(
        (floating.maximum_width, floating.maximum_height),
        (None, None)
    );
    assert!(!floating.maximized);
    assert!(!floating.minimized);
    assert!(floating.fullscreen);
    assert!(runtime.take_window_surface_change());
    assert!(!runtime.take_window_surface_change());
    assert_eq!(
        runtime.take_window_surface_actions(),
        [
            WindowSurfaceAction::Move { id: 1 },
            WindowSurfaceAction::Resize {
                id: 1,
                edge: "bottom_right".to_owned(),
            },
        ]
    );
}

#[test]
fn window_models_keep_multiple_surfaces_independent() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "multiple-windows.lua",
            br#"
                local ui = require("morf.ui")
                local window = require("morf.window")
                local first = window.floating {
                  root = ui.Item {}, visible = true, title = "one"
                }
                local second = window.floating {
                  root = ui.Item {}, visible = true, title = "two", parent = first
                }
                local popup = window.popup {
                  root = ui.Item {}, visible = true, width = 100, height = 50,
                  parent = second,
                }
                assert(second:parent_id() == 0)
                assert(popup:parent_id() == 1)
                assert(popup:set_parent(first) == 0)
                assert(not pcall(second.set_parent, second, nil))
                second:close()
                assert(second:set_parent(nil) == nil)
                assert(second:set_parent(first) == 0)
                second:open()
                window.popup {
                  root = ui.Item {}, visible = true, width = 200, height = 60,
                  anchor = { window = first },
                }
            "#,
        )
        .unwrap();

    let surfaces = runtime.window_surface_configs();
    assert_eq!(surfaces.len(), 4);
    assert_eq!(
        surfaces
            .iter()
            .filter(|surface| matches!(surface.kind, WindowSurfaceKind::Popup(_)))
            .count(),
        2
    );
    assert_eq!(
        surfaces
            .iter()
            .filter(|surface| matches!(surface.kind, WindowSurfaceKind::Floating(_)))
            .count(),
        2
    );
    assert!(surfaces.iter().all(|surface| surface.visible));
    let WindowSurfaceKind::Floating(second) = &surfaces[1].kind else {
        panic!("second surface was not floating");
    };
    assert_eq!(second.parent, Some(0));
    let WindowSurfaceKind::Popup(first_popup) = &surfaces[2].kind else {
        panic!("third surface was not a popup");
    };
    assert_eq!(first_popup.parent, Some(0));
    let WindowSurfaceKind::Popup(second_popup) = &surfaces[3].kind else {
        panic!("fourth surface was not a popup");
    };
    assert_eq!(second_popup.parent, Some(0));
    assert_eq!(
        surfaces
            .iter()
            .map(|surface| surface.id)
            .collect::<HashSet<_>>()
            .len(),
        4
    );
}

#[test]
fn native_reparenting_wraps_and_unwraps_scene_items() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "reparent.lua",
            br#"
                local ui = require("morf.ui")
                local child = ui.Text { text = "content" }
                local wrapper = ui.Item {}
                ui.reparent(child, wrapper)
                ui.reparent(child, nil)
                ui.reparent(child, wrapper)
            "#,
        )
        .unwrap();

    let scene = runtime.scene();
    let roots = scene.roots();
    assert_eq!(roots.len(), 1);
    assert_eq!(scene.children(roots[0]).unwrap().len(), 1);
}

#[test]
fn popup_anchor_tracks_native_item_geometry() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "popup-anchor.lua",
            br#"
                local ui = require("morf.ui")
                local window = require("morf.window")
                local anchor = ui.Item {
                  x = 20, y = 30, implicit_width = 40, implicit_height = 20,
                }
                ui.Item { anchor }
                local popup_root = ui.Item {}
                window.popup {
                  root = popup_root,
                  anchor = { node = anchor, x = 2, y = 3, margin = 4 },
                  width = 100,
                  height = 80,
                }
            "#,
        )
        .unwrap();
    let popup_root = runtime.window_surface_configs()[0].root;
    let primary = runtime
        .scene()
        .roots()
        .into_iter()
        .find(|root| *root != popup_root)
        .unwrap();
    let layout = Layout::compute(
        &runtime.scene(),
        primary,
        morf_layout::Size {
            width: 200.0,
            height: 100.0,
        },
        &mut NoText,
    )
    .unwrap();
    runtime.take_window_surface_change();
    runtime.observe_layout(&layout);

    let surfaces = runtime.window_surface_configs();
    let WindowSurfaceKind::Popup(config) = &surfaces[0].kind else {
        panic!("surface was not a popup");
    };
    assert_eq!(
        (
            config.anchor_x,
            config.anchor_y,
            config.anchor_width,
            config.anchor_height
        ),
        (18, 29, 48, 28)
    );
    assert!(runtime.take_window_surface_change());
}
