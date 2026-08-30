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

#[test]
fn one_anchor_conversion_serves_creation_and_reconfiguration() {
    assert_eq!(
        layer_anchor_mask(LayerAnchors {
            top: true,
            right: true,
            bottom: false,
            left: true,
        }),
        Anchor::TOP | Anchor::RIGHT | Anchor::LEFT
    );
    assert_eq!(
        layer_anchor_mask(LayerAnchors {
            top: false,
            right: false,
            bottom: true,
            left: false,
        }),
        Anchor::BOTTOM
    );
    assert_eq!(
        layer_anchor_mask(LayerAnchors {
            top: false,
            right: false,
            bottom: false,
            left: false,
        }),
        Anchor::empty()
    );
    assert_eq!(
        layer_interactivity(KeyboardFocus::Exclusive),
        WlrKeyboardInteractivity::Exclusive
    );
    assert_eq!(
        layer_interactivity(KeyboardFocus::None),
        WlrKeyboardInteractivity::None
    );
}

#[test]
fn reposition_tokens_count_per_popup_and_record_the_echo() {
    let mut repositions = HashMap::new();

    assert_eq!(next_reposition_token(&mut repositions, 4), 1);
    assert_eq!(next_reposition_token(&mut repositions, 4), 2);
    // A second popup counts on its own, so an echo identifies one request.
    assert_eq!(next_reposition_token(&mut repositions, 9), 1);
    assert_eq!(repositions[&4].acknowledged, None);

    record_reposition_ack(&mut repositions, 4, 2);
    assert_eq!(repositions[&4].acknowledged, Some(2));
    assert_eq!(repositions[&9].acknowledged, None);

    // Zero stays unused: a wrapped counter must not look like a fresh one.
    repositions.get_mut(&4).unwrap().sent = u32::MAX;
    assert_eq!(next_reposition_token(&mut repositions, 4), 1);
}
