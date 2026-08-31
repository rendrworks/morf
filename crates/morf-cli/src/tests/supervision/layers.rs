use crate::surface_layers::LayerUpdate;
use crate::surface_layers::RESERVE_LAYER_BASE;
use crate::surface_layers::layer_update;
use crate::surface_layers::reserve_bar_config;
use crate::surface_layers::window_layer_id;
use crate::surface_layers::window_surface_id;
use crate::surfaces::primary_surface_root;
use crate::surfaces::runtime_bar_config;
use morf_lua::Runtime;
use morf_lua::WindowSurfaceKind;
use morf_wayland::KeyboardFocus;
use morf_wayland::PRIMARY_LAYER;
use morf_wayland::ShellLayer;

use morf_lua::LayerSurfaceConfig;

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

    assert_eq!(config.namespace, "morf-reserve-right");
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
                    local ui = require("morf.ui")
                    local window = require("morf.window")
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
                    local ui = require("morf.ui")
                    local window = require("morf.window")
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
                    local morf = require("morf")
                    local ui = require("morf.ui")
                    ui.Item {}
                    morf.surface.namespace = "shell"
                    morf.surface.height = 40
                    morf.surface.exclusive_zone = 40
                    morf.surface.reserve = { top = 10, left = 6 }
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

#[test]
fn layer_geometry_changes_without_recreating_the_surface() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "layer-update.lua",
            br#"
                    local ui = require("morf.ui")
                    local window = require("morf.window")
                    ui.Item {}
                    window.layer {
                      root = ui.Item {},
                      namespace = "ribbon",
                      width = 400,
                      height = 200,
                      margin_left = 0,
                      layer = "overlay",
                    }
                "#,
        )
        .unwrap();
    let surfaces = runtime.window_surface_configs();
    let WindowSurfaceKind::Layer(config) = &surfaces[0].kind else {
        panic!("configured surface was not a layer surface");
    };

    // A surface with no predecessor has to be created.
    assert_eq!(layer_update(None, config), LayerUpdate::Recreate);
    assert_eq!(layer_update(Some(config), config), LayerUpdate::None);

    // Everything wlr-layer-shell accepts on a mapped surface stays mapped.
    for moved in [
        LayerSurfaceConfig {
            margin_left: -400,
            ..config.clone()
        },
        LayerSurfaceConfig {
            width: 640,
            ..config.clone()
        },
        LayerSurfaceConfig {
            exclusive_zone: 24,
            ..config.clone()
        },
        LayerSurfaceConfig {
            keyboard_focus: "exclusive".to_owned(),
            ..config.clone()
        },
        LayerSurfaceConfig {
            anchors: morf_lua::SurfaceAnchors {
                top: true,
                right: false,
                bottom: false,
                left: true,
            },
            ..config.clone()
        },
    ] {
        assert_eq!(layer_update(Some(config), &moved), LayerUpdate::Geometry);
    }

    // Namespace and layer are fixed when the surface is created.
    assert_eq!(
        layer_update(
            Some(config),
            &LayerSurfaceConfig {
                namespace: "ribbon-popup".to_owned(),
                ..config.clone()
            }
        ),
        LayerUpdate::Recreate
    );
    assert_eq!(
        layer_update(
            Some(config),
            &LayerSurfaceConfig {
                layer: "top".to_owned(),
                ..config.clone()
            }
        ),
        LayerUpdate::Recreate
    );
}
