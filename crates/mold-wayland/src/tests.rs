use super::*;

#[test]
fn physical_size_rounds_fractional_scale_upward() {
    assert_eq!(physical_size((101, 31), 150), (127, 39));
}

#[test]
fn output_transforms_have_stable_public_names() {
    assert_eq!(
        output_transform_name(wl_output::Transform::Normal),
        "normal"
    );
    assert_eq!(output_transform_name(wl_output::Transform::_90), "90");
    assert_eq!(
        output_transform_name(wl_output::Transform::Flipped270),
        "flipped_270"
    );
}

#[test]
fn popup_defaults_preserve_general_constraint_policy() {
    let popup = PopupConfig::default();
    assert_eq!(popup.anchor_edge, PopupAnchor::BottomLeft);
    assert_eq!(popup.gravity, PopupGravity::BottomRight);
    assert!(popup.constraints.slide_x);
    assert!(popup.constraints.slide_y);
    assert!(popup.constraints.flip_x);
    assert!(popup.constraints.flip_y);
    assert!(!popup.constraints.resize_x);
    assert!(!popup.constraints.resize_y);
}

#[test]
fn default_virtual_keymap_round_trips() {
    let keymap = default_keymap().unwrap();
    let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    assert!(
        xkbcommon::xkb::Keymap::new_from_string(
            &context,
            keymap,
            xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1,
            xkbcommon::xkb::COMPILE_NO_FLAGS,
        )
        .is_some()
    );
}

#[test]
fn layer_roles_are_distinct_per_surface_identifier() {
    assert_eq!(PRIMARY_LAYER, 0);
    assert_ne!(SurfaceRole::Layer(0), SurfaceRole::Layer(1));
    assert_ne!(SurfaceRole::Layer(1), SurfaceRole::Popup(1));
    let events = [
        LayerEvent::Configure {
            id: 3,
            width: 8,
            height: 4,
        },
        LayerEvent::Scale {
            id: 3,
            scale_120: 180,
        },
        LayerEvent::Frame { id: 3, time_ms: 1 },
        LayerEvent::Closed { id: 3 },
    ];
    assert!(events.iter().all(|event| match event {
        LayerEvent::Configure { id, .. }
        | LayerEvent::Scale { id, .. }
        | LayerEvent::Frame { id, .. }
        | LayerEvent::Closed { id } => *id == 3,
        _ => false,
    }));
}
