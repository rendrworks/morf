use super::*;
#[test]
fn shapes_emit_path_commands() {
    let mut scene = Scene::new();
    let shape = scene.create(Element::Shape);
    scene.assign(shape, "path", "M0 0 L16 0 L8 16 Z").unwrap();
    scene.assign(shape, "width", 16.0).unwrap();
    scene.assign(shape, "height", 16.0).unwrap();
    scene.assign(shape, "stroke_width", 2.0).unwrap();
    let layout = Layout::compute(
        &scene,
        shape,
        Size {
            width: 16.0,
            height: 16.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert!(matches!(
        &list.commands[0],
        DrawCommand::Path { path, stroke_width, .. }
            if path == "M0 0 L16 0 L8 16 Z" && *stroke_width == 2.0
    ));
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

    assert!(!engine.render(&scene, &layout, 120).unwrap().is_empty());
    assert!(engine.render(&scene, &layout, 120).unwrap().is_empty());
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
    tracker.diff(&list, 120);

    assert_eq!(
        tracker.diff(&list, 150),
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
        }],
        layers: Vec::new(),
    };
    tracker.diff(&first, 120);
    let mut second = first.clone();
    if let DrawCommand::Quad { bounds, .. } = &mut second.commands[0] {
        bounds.x = 20.0;
    }

    let damage = tracker.diff(&second, 120);

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
    let instance = SdfQuadInstance::from_command(&command, 120).unwrap();
    assert_eq!(instance.bounds, [15.0, 16.0, 56.0, 36.0]);
    assert_eq!(instance.shape, [5.0, 4.0, 40.0, 20.0]);
    assert_eq!(instance.effects[..3], [2.0, 6.0, 2.0]);
    assert_eq!(instance.border[..2], [1.0, 0.0]);

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
    let instance = SdfQuadInstance::from_command(&inner, 120).unwrap();
    assert_eq!(instance.shadow[2], 1.0);
}
