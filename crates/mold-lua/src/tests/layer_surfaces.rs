#[test]
fn configured_layer_surfaces_carry_their_own_settings() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layers.lua",
            br##"
                local ui = require("mold.ui")
                local window = require("mold.window")
                local shell = ui.Item {}
                local edge = window.layer {
                    root = ui.Rect { color = "#111111" },
                    visible = true,
                    namespace = "border-top",
                    width = 0,
                    height = 6,
                    anchors = { top = true, left = true, right = true },
                    margin_top = -6,
                    layer = "overlay",
                    keyboard_focus = "none",
                }
                local corner = window.layer {
                    root = ui.Item {},
                    namespace = "border-corner",
                    width = 24,
                    height = 24,
                    updates_enabled = false,
                }
                assert(edge:kind() == "layer" and edge:visible())
                assert(corner:kind() == "layer" and not corner:visible())
                assert(not corner:updates_enabled())
                assert(edge:parent_id() == nil)
                assert(corner:size().width == 24)
                corner:open()
                edge:close()
                assert(corner:visible() and not edge:visible())
            "##,
        )
        .unwrap();

    let surfaces = runtime.window_surface_configs();
    assert_eq!(surfaces.len(), 2);
    assert!(!surfaces[0].visible);
    assert!(surfaces[1].visible);
    let WindowSurfaceKind::Layer(edge) = &surfaces[0].kind else {
        panic!("first surface was not a layer surface");
    };
    assert_eq!(edge.namespace, "border-top");
    assert_eq!((edge.width, edge.height), (0, 6));
    assert_eq!(edge.margin_top, -6);
    assert_eq!(edge.layer, "overlay");
    assert_eq!(edge.keyboard_focus, "none");
    assert!(edge.anchors.top && edge.anchors.left && edge.anchors.right);
    assert!(!edge.anchors.bottom);
    // Decoration drawn outside the usable area must not claim space by default.
    assert_eq!(edge.exclusive_zone, 0);
    assert_eq!(edge.reserve, SurfaceReserve::default());
}

#[test]
fn layer_surface_settings_reject_unknown_and_shell_only_keys() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layer-errors.lua",
            br#"
                local ui = require("mold.ui")
                local window = require("mold.window")
                local root = ui.Item {}
                assert(not pcall(window.layer, { root = root, nonsense = 1 }))
                assert(not pcall(window.layer, { root = root, layer = "middle" }))
                assert(not pcall(window.layer, { root = root, height = 0 }))
                assert(not pcall(window.layer, { root = root, reserve = { top = 4 } }))
                assert(not pcall(window.layer, { width = 10 }))
            "#,
        )
        .unwrap();
    assert!(runtime.window_surface_configs().is_empty());
}

#[test]
fn shell_surface_reserve_is_native_and_typed() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "reserve.lua",
            br#"
                local mold = require("mold")
                assert(mold.surface.reserve.top == 0)
                mold.surface.reserve = { top = 12, bottom = 8 }
                assert(mold.surface.reserve.top == 12)
                assert(mold.surface.reserve.right == 0)
                assert(mold.surface.reserve.bottom == 8)
                assert(not pcall(function() mold.surface.reserve = { top = -1 } end))
                assert(not pcall(function() mold.surface.reserve = { middle = 1 } end))
                assert(not pcall(function() mold.surface.reserve = 4 end))
            "#,
        )
        .unwrap();

    assert_eq!(
        runtime.layer_surface_config().reserve,
        SurfaceReserve {
            top: 12,
            right: 0,
            bottom: 8,
            left: 0,
        }
    );
}

#[test]
fn a_layer_surface_root_is_not_a_scene_orphan() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layer-root.lua",
            br#"
                local ui = require("mold.ui")
                local window = require("mold.window")
                ui.Item {}
                window.layer { root = ui.Item {}, height = 4 }
            "#,
        )
        .unwrap();

    let surfaces = runtime.window_surface_configs();
    assert_eq!(surfaces.len(), 1);
    let roots = runtime.scene().roots();
    assert_eq!(roots.len(), 2);
    assert!(roots.contains(&surfaces[0].root));
}

#[test]
fn shell_surface_geometry_reports_only_real_changes() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "geometry.lua",
            br#"
                local mold = require("mold")
                mold.surface.margin_left = 200
            "#,
        )
        .unwrap();

    assert!(runtime.take_layer_surface_change());
    assert!(!runtime.take_layer_surface_change());

    // Lua re-runs a binding whenever anything it reads moves, so an assignment
    // that writes back the value already there must not reconfigure a surface.
    runtime
        .execute(
            "unchanged.lua",
            br#"
                local mold = require("mold")
                mold.surface.margin_left = 200
            "#,
        )
        .unwrap();
    assert!(!runtime.take_layer_surface_change());
}

#[test]
fn shell_surface_geometry_accepts_interpolated_numbers() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "animated.lua",
            br#"
                local mold = require("mold")
                mold.surface.margin_left = 120.6
                mold.surface.margin_top = -3.5
                mold.surface.width = 640.4
                assert(not pcall(function() mold.surface.margin_left = "12" end))
            "#,
        )
        .unwrap();

    let config = runtime.layer_surface_config();
    assert!(runtime.take_layer_surface_change());
    // A slide animation assigns a float every frame; rounding is what keeps the
    // margin animatable instead of raising an error mid-transition.
    assert_eq!(config.margin_left, 121);
    assert_eq!(config.margin_top, -4);
    assert_eq!(config.width, 640);
}
