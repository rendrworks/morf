#[test]
fn layer_identifiers_separate_engine_surfaces_from_configured_ones() {
    assert_eq!(window_layer_id(0), 1);
    assert_eq!(window_surface_id(window_layer_id(41)), Some(41));
    assert_eq!(window_surface_id(PRIMARY_LAYER), None);
    for index in 0..4 {
        assert_eq!(window_surface_id(RESERVE_LAYER_BASE + index), None);
    }
}

#[test]
fn each_reserver_anchors_one_edge_and_claims_its_thickness() {
    let config = reserve_bar_config("right", 24, "eDP-1");

    assert_eq!(config.namespace, "mold-reserve-right");
    assert_eq!(config.exclusive_zone, 24);
    assert_eq!(config.output.as_deref(), Some("eDP-1"));
    assert_eq!(config.layer, ShellLayer::Bottom);
    assert_eq!(config.keyboard_focus, KeyboardFocus::None);
    assert!(config.anchors.right);
    assert!(!config.anchors.top && !config.anchors.bottom && !config.anchors.left);
    // Layer shell rejects a zero extent on an axis anchored to neither edge.
    assert!(config.width > 0 && config.height > 0);
}

#[test]
fn configured_layer_surfaces_join_the_window_roots() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layer-roots.lua",
            br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    local primary = ui.Item {}
                    window.layer { root = ui.Item {}, height = 6 }
                "#,
        )
        .unwrap();

    let primary = primary_surface_root(&runtime).unwrap();
    let surfaces = runtime.window_surface_configs();
    assert_eq!(surfaces.len(), 1);
    assert_ne!(surfaces[0].root, primary);
    assert_eq!(runtime.scene().roots()[0], primary);
}

#[test]
fn a_configured_layer_surface_becomes_its_own_bar_config() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layer-config.lua",
            br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    ui.Item {}
                    window.layer {
                      root = ui.Item {},
                      namespace = "border-bottom",
                      width = 0,
                      height = 8,
                      anchors = { bottom = true, left = true, right = true },
                      margin_bottom = -8,
                      layer = "overlay",
                      keyboard_focus = "none",
                    }
                "#,
        )
        .unwrap();

    let surfaces = runtime.window_surface_configs();
    let WindowSurfaceKind::Layer(config) = &surfaces[0].kind else {
        panic!("configured surface was not a layer surface");
    };
    let bar = runtime_bar_config(config, "DP-2").unwrap();

    assert_eq!(bar.namespace, "border-bottom");
    assert_eq!((bar.width, bar.height), (0, 8));
    assert_eq!(bar.margin_bottom, -8);
    assert_eq!(bar.exclusive_zone, 0);
    assert_eq!(bar.layer, ShellLayer::Overlay);
    assert_eq!(bar.keyboard_focus, KeyboardFocus::None);
    assert_eq!(bar.output.as_deref(), Some("DP-2"));
    assert!(bar.anchors.bottom && bar.anchors.left && bar.anchors.right);
    assert!(!bar.anchors.top);
}

#[test]
fn the_shell_surface_still_configures_the_primary_layer() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "shell-config.lua",
            br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    ui.Item {}
                    mold.surface.namespace = "shell"
                    mold.surface.height = 40
                    mold.surface.exclusive_zone = 40
                    mold.surface.reserve = { top = 10, left = 6 }
                "#,
        )
        .unwrap();

    let config = runtime.layer_surface_config();
    let bar = runtime_bar_config(&config, "eDP-1").unwrap();
    assert_eq!(bar.namespace, "shell");
    assert_eq!(bar.exclusive_zone, 40);
    assert_eq!(
        config
            .reserve
            .edges()
            .into_iter()
            .filter(|(_, thickness)| *thickness > 0)
            .collect::<Vec<_>>(),
        [("top", 10), ("left", 6)]
    );
}
