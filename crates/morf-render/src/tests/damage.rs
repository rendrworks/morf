use super::*;

/// Diffs one frame and makes it the baseline, the way `RenderEngine` does.
fn diff_frame(tracker: &mut DamageTracker, mut list: DrawList, scale_120: u32) -> Vec<DamageRect> {
    let damage = tracker.diff(&list, scale_120);
    tracker.retain(&mut list);
    damage
}
#[test]
fn unchanged_frames_submit_no_gpu_work() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Rect);
    scene.assign(root, "width", 20.0).unwrap();
    scene.assign(root, "height", 10.0).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 20.0,
            height: 10.0,
        },
        &mut NoText,
    )
    .unwrap();
    let mut engine = RenderEngine::new(RecordingBackend::default());

    // The damage is offered to the host before anything is presented, because
    // a commit carries whatever damage was declared before it.
    let mut declared = Vec::new();
    assert!(
        !engine
            .render(&scene, &layout, 120, |damage| declared
                .extend_from_slice(damage))
            .unwrap()
            .is_empty()
    );
    assert!(
        !declared.is_empty(),
        "the host saw the first frame's damage"
    );
    declared.clear();
    assert!(
        engine
            .render(&scene, &layout, 120, |damage| declared
                .extend_from_slice(damage))
            .unwrap()
            .is_empty()
    );
    assert!(
        declared.is_empty(),
        "an unchanged frame declares nothing, so nothing is recomposited"
    );
    assert_eq!(engine.backend_mut().frames, 1);
}

#[test]
fn fractional_scale_rounds_damage_outward() {
    let geometry = Geometry {
        x: 1.0,
        y: 2.0,
        width: 3.0,
        height: 4.0,
    };

    assert_eq!(
        physical_damage(geometry, 180),
        Some(DamageRect {
            x: 1,
            y: 3,
            width: 5,
            height: 6,
        })
    );
}

#[test]
fn scale_change_redamages_an_unchanged_frame() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Rect);
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 20.0,
            height: 10.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let mut tracker = DamageTracker::default();
    diff_frame(&mut tracker, list.clone(), 120);

    assert_eq!(
        diff_frame(&mut tracker, list.clone(), 150),
        vec![DamageRect {
            x: 0,
            y: 0,
            width: 25,
            height: 13,
        }]
    );
}

#[test]
fn changed_command_damages_old_and_new_bounds() {
    let node = {
        let mut scene = Scene::new();
        scene.create(Element::Rect)
    };
    let mut tracker = DamageTracker::default();
    let first = DrawList {
        commands: vec![DrawCommand::Quad {
            node,
            bounds: Geometry {
                width: 10.0,
                height: 10.0,
                ..Geometry::default()
            },
            transform: Transform2D::IDENTITY,
            clip: None,
            color: Color::rgba8(0, 0, 0, 255),
            color_overlay: Color::rgba8(0, 0, 0, 0),
            gradient: Gradient::None,
            radii: [0.0; 4],
            border_width: 0.0,
            antialiasing: true,
            border_pixel_aligned: false,
            border_color: Color::rgba8(0, 0, 0, 0),
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_spread: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_inner: false,
            shader: None,
        }],
        layers: Vec::new(),
    };
    diff_frame(&mut tracker, first.clone(), 120);
    let mut second = first.clone();
    if let DrawCommand::Quad { bounds, .. } = &mut second.commands[0] {
        bounds.x = 20.0;
    }

    let damage = diff_frame(&mut tracker, second.clone(), 120);

    assert_eq!(damage.len(), 2);
}

#[test]
fn blur_and_shadow_expand_damage_and_gpu_bounds() {
    let node = {
        let mut scene = Scene::new();
        scene.create(Element::Rect)
    };
    let command = DrawCommand::Quad {
        node,
        bounds: Geometry {
            x: 20.0,
            y: 20.0,
            width: 40.0,
            height: 20.0,
        },
        transform: Transform2D::IDENTITY,
        clip: None,
        color: Color::rgba8(255, 255, 255, 255),
        color_overlay: Color::rgba8(0, 0, 0, 0),
        gradient: Gradient::None,
        radii: [4.0; 4],
        border_width: 0.6,
        antialiasing: false,
        border_pixel_aligned: true,
        border_color: Color::rgba8(0, 0, 0, 0),
        blur: 2.0,
        shadow_color: Color::rgba8(0, 0, 0, 128),
        shadow_blur: 6.0,
        shadow_spread: 2.0,
        shadow_offset_x: 3.0,
        shadow_offset_y: 4.0,
        shadow_inner: false,
        shader: None,
    };

    assert_eq!(
        command.bounds(),
        Geometry {
            x: 15.0,
            y: 16.0,
            width: 56.0,
            height: 36.0,
        }
    );
    // The rectangle is a one-layer field now, so its own rect is the instance
    // bounds and the effect expansion is the `area` the shader walks — the
    // quad no longer carries the expanded rectangle with the shape offset
    // inside it.
    let mut layers = Vec::new();
    let mut materials = Vec::new();
    let instance =
        SdfFieldInstance::from_command(&command, 120, &mut layers, &mut materials).unwrap();
    assert_eq!(instance.bounds, [20.0, 20.0, 40.0, 20.0]);
    assert_eq!(instance.area, [-5.0, -4.0, 51.0, 32.0]);
    assert_eq!(instance.style[..2], [1.0, 2.0]);
    assert_eq!(materials[0].effects[1..3], [6.0, 2.0]);
    assert_eq!(layers[0].rect, [20.0, 10.0, 20.0, 10.0]);

    let mut inner = command;
    if let DrawCommand::Quad {
        blur, shadow_inner, ..
    } = &mut inner
    {
        *blur = 0.0;
        *shadow_inner = true;
    }
    assert_eq!(
        inner.bounds(),
        Geometry {
            x: 20.0,
            y: 20.0,
            width: 40.0,
            height: 20.0,
        }
    );
    let mut layers = Vec::new();
    let mut materials = Vec::new();
    SdfFieldInstance::from_command(&inner, 120, &mut layers, &mut materials).unwrap();
    assert_eq!(materials[0].shadow[2], 1.0);
}

#[test]
fn a_clip_rect_repaints_when_only_its_fill_changes() {
    // A ClipRect emits two commands under one node: the fill, and the border it
    // overlays. Keyed on the node alone the two collapse into one — a HashMap
    // keeps the last — so the fill is never compared against anything and a
    // change to it damages nothing at all.
    let mut scene = Scene::new();
    let node = scene.create(Element::ClipRect);
    scene.assign(node, "width", 40.0).unwrap();
    scene.assign(node, "height", 40.0).unwrap();
    scene.assign(node, "border_width", 2.0).unwrap();
    scene.assign(node, "color", "#ff0000").unwrap();

    let draw = |scene: &Scene| {
        let layout = Layout::compute(
            scene,
            scene.roots()[0],
            Size {
                width: 40.0,
                height: 40.0,
            },
            &mut NoText,
        )
        .unwrap();
        DrawList::from_scene(scene, &layout).unwrap()
    };

    let mut tracker = DamageTracker::default();
    let first = draw(&scene);
    assert!(
        !diff_frame(&mut tracker, first, 120).is_empty(),
        "the first frame damages everything"
    );
    assert!(
        diff_frame(&mut tracker, draw(&scene), 120).is_empty(),
        "an unchanged frame damages nothing"
    );

    scene.assign(node, "color", "#00ff00").unwrap();
    assert!(
        !diff_frame(&mut tracker, draw(&scene), 120).is_empty(),
        "changing only the fill has to repaint it"
    );
}
