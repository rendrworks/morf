/// Builds the popup configurations one Lua source registers, in identifier order.
fn popup_configs(name: &'static str, source: &[u8]) -> Vec<PopupSurfaceConfig> {
    let mut runtime = Runtime::default();
    runtime.execute(name, source).unwrap();
    runtime
        .window_surface_configs()
        .into_iter()
        .filter_map(|surface| match surface.kind {
            WindowSurfaceKind::Popup(config) => Some(config),
            _ => None,
        })
        .collect()
}

fn moved_popup() -> Vec<PopupSurfaceConfig> {
    popup_configs(
        "popup-change.lua",
        br#"
                local ui = require("mold.ui")
                local window = require("mold.window")
                ui.Item {}
                window.popup {
                  root = ui.Item {},
                  width = 200,
                  height = 120,
                  anchor = { x = 4, y = 8, width = 10, height = 12 },
                  anchor_edge = "top_right",
                  gravity = "bottom_left",
                  offset_x = 3,
                  offset_y = -5,
                  constraints = { flip_x = false, resize_y = true },
                }
            "#,
    )
}

#[test]
fn moving_a_popup_is_positional_and_reparenting_it_is_not() {
    let configs = moved_popup();
    let config = &configs[0];

    // Every positioner field together still only asks for a reposition.
    let mut moved = config.clone();
    moved.anchor_x += 17;
    moved.anchor_y += 3;
    moved.anchor_width = 40;
    moved.anchor_height = 44;
    moved.width = 320;
    moved.height = 64;
    moved.anchor_edge = "bottom".to_owned();
    moved.gravity = "top_left".to_owned();
    moved.offset_x = 0;
    moved.offset_y = 0;
    moved.constraints.slide_x = false;
    assert_ne!(&moved, config);
    assert!(!popup_change_is_structural(config, &moved));

    // The parent and the grab are bound when the xdg_popup is created.
    let mut reparented = config.clone();
    reparented.parent = Some(41);
    assert!(popup_change_is_structural(config, &reparented));
    let mut grabbed = config.clone();
    grabbed.grab_focus = !config.grab_focus;
    assert!(popup_change_is_structural(config, &grabbed));
}

#[test]
fn popup_geometry_becomes_one_positioner_request() {
    let configs = moved_popup();
    let config = &configs[0];

    let request = popup_client_config(config).unwrap();

    assert_eq!(
        (
            request.anchor.x,
            request.anchor.y,
            request.anchor.width,
            request.anchor.height
        ),
        (4, 8, 10, 12)
    );
    assert_eq!((request.width, request.height), (200, 120));
    assert_eq!(request.anchor_edge, PopupAnchor::TopRight);
    assert_eq!(request.gravity, PopupGravity::BottomLeft);
    assert_eq!((request.offset_x, request.offset_y), (3, -5));
    assert!(!request.constraints.flip_x && request.constraints.flip_y);
    assert!(request.constraints.resize_y && !request.constraints.resize_x);
    assert!(!request.grab_focus);

    // An unknown edge is refused rather than silently defaulted, on the move
    // path exactly as on the create path.
    let mut sideways = config.clone();
    sideways.gravity = "sideways".to_owned();
    assert!(popup_client_config(&sideways).is_err());
}

#[test]
fn a_parentless_popup_anchors_to_the_shells_own_layer() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "popup-parents.lua",
            br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    ui.Item {}
                    local dialog = window.floating { root = ui.Item {} }
                    window.popup { root = ui.Item {}, parent = dialog }
                    window.popup { root = ui.Item {} }
                "#,
        )
        .unwrap();
    let surfaces = runtime.window_surface_configs();
    let by_id = surfaces
        .iter()
        .map(|surface| (surface.id, surface))
        .collect::<HashMap<_, _>>();
    let configs = surfaces
        .iter()
        .filter_map(|surface| match &surface.kind {
            WindowSurfaceKind::Popup(config) => Some(config),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        popup_parent_role(configs[0], &by_id).unwrap(),
        SurfaceRole::Floating(0)
    );
    assert_eq!(
        popup_parent_role(configs[1], &by_id).unwrap(),
        SurfaceRole::Layer(PRIMARY_LAYER)
    );

    let mut orphan = configs[0].clone();
    orphan.parent = Some(97);
    assert!(popup_parent_role(&orphan, &by_id).is_err());
}
