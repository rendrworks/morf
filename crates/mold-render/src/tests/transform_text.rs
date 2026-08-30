use super::*;
#[test]
fn draw_list_composes_ancestor_transforms() {
    let mut scene = Scene::new();
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    scene.assign(parent, "width", 100.0).unwrap();
    scene.assign(parent, "height", 100.0).unwrap();
    scene.assign(parent, "rotation", 90.0).unwrap();
    scene.assign(child, "x", 10.0).unwrap();
    scene.assign(child, "width", 20.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.assign(child, "scale", 2.0).unwrap();
    scene.reparent(child, Some(parent)).unwrap();
    let layout = Layout::compute(
        &scene,
        parent,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Quad { transform, .. } = &list.commands[0] else {
        panic!("child did not emit a quad");
    };
    let transformed = transform.bounds(layout.geometry(child).unwrap());
    assert!((transformed.x - 85.0).abs() < 0.000_001);
    assert!(transformed.y.abs() < 0.000_001);
    assert!((transformed.width - 20.0).abs() < 0.000_001);
    assert!((transformed.height - 40.0).abs() < 0.000_001);
}

#[test]
fn draw_list_keeps_non_uniform_origin_aware_transform() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let rect = scene.create(Element::Rect);
    scene.assign(rect, "x", 10.0).unwrap();
    scene.assign(rect, "y", 20.0).unwrap();
    scene.assign(rect, "width", 100.0).unwrap();
    scene.assign(rect, "height", 40.0).unwrap();
    scene.assign(rect, "scale_x", 2.0).unwrap();
    scene.assign(rect, "scale_y", 0.5).unwrap();
    scene.assign(rect, "transform_origin_x", 0.0).unwrap();
    scene.assign(rect, "transform_origin_y", 0.0).unwrap();
    scene.assign(rect, "translate_x", 7.0).unwrap();
    scene.assign(rect, "translate_y", -3.0).unwrap();
    scene.reparent(rect, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 100.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Quad {
        bounds, transform, ..
    } = list.commands[0]
    else {
        panic!("rectangle did not emit a quad");
    };

    assert_eq!(
        transform.bounds(bounds),
        Geometry {
            x: 17.0,
            y: 17.0,
            width: 200.0,
            height: 20.0,
        }
    );
}

#[test]
fn draw_list_intersects_nested_ancestor_clips() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let viewport = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    scene.assign(root, "width", 100.0).unwrap();
    scene.assign(root, "height", 100.0).unwrap();
    scene.assign(root, "clip", true).unwrap();
    scene.assign(viewport, "x", 25.0).unwrap();
    scene.assign(viewport, "width", 50.0).unwrap();
    scene.assign(viewport, "height", 100.0).unwrap();
    scene.assign(viewport, "clip", true).unwrap();
    scene.assign(child, "x", -25.0).unwrap();
    scene.assign(child, "width", 100.0).unwrap();
    scene.assign(child, "height", 100.0).unwrap();
    scene.reparent(viewport, Some(root)).unwrap();
    scene.reparent(child, Some(viewport)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    assert_eq!(
        list.commands[0].clip(),
        Some(Geometry {
            x: 25.0,
            y: 0.0,
            width: 50.0,
            height: 100.0,
        })
    );
    assert_eq!(list.commands[0].bounds(), list.commands[0].clip().unwrap());
}

#[test]
fn text_commands_preserve_wrap_and_alignment() {
    let mut scene = Scene::new();
    let text = scene.create(Element::Text);
    scene.assign(text, "width", 200.0).unwrap();
    scene.assign(text, "height", 80.0).unwrap();
    scene.assign(text, "wrap", true).unwrap();
    scene.assign(text, "elide", "right").unwrap();
    scene.assign(text, "font_weight", 700.0).unwrap();
    scene
        .assign(text, "font_source", "file:///tmp/test.ttf")
        .unwrap();
    scene
        .assign(text, "horizontal_alignment", "center")
        .unwrap();
    scene.assign(text, "vertical_alignment", "bottom").unwrap();
    let layout = Layout::compute(
        &scene,
        text,
        Size {
            width: 200.0,
            height: 80.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Text {
        wrap,
        elide,
        font_weight,
        ref font_source,
        horizontal_alignment,
        vertical_alignment,
        ..
    } = list.commands[0]
    else {
        panic!("text did not emit a text command");
    };
    assert!(wrap);
    assert_eq!(elide, TextElide::Right);
    assert_eq!(font_weight, 700.0);
    assert_eq!(font_source, "file:///tmp/test.ttf");
    assert_eq!(horizontal_alignment, TextAlignment::Center);
    assert_eq!(vertical_alignment, VerticalAlignment::Bottom);
}

#[test]
fn rectangles_emit_normalized_gradient_instances() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene.assign(rect, "width", 100.0).unwrap();
    scene.assign(rect, "height", 50.0).unwrap();
    scene.assign(rect, "opacity", 0.5).unwrap();
    scene.assign(rect, "gradient_type", "linear").unwrap();
    scene
        .assign(rect, "gradient_start_color", "#ff0000")
        .unwrap();
    scene.assign(rect, "gradient_end_color", "#0000ff").unwrap();
    scene.assign(rect, "gradient_end_y", 1.0).unwrap();
    scene.assign(rect, "radius", 4.0).unwrap();
    scene.assign(rect, "top_right_radius", 12.0).unwrap();
    let layout = Layout::compute(
        &scene,
        rect,
        Size {
            width: 100.0,
            height: 50.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Quad {
        gradient, radii, ..
    } = &list.commands[0]
    else {
        panic!("rectangle did not emit a quad");
    };
    assert_eq!(*radii, [4.0, 12.0, 4.0, 4.0]);
    assert_eq!(
        gradient,
        &Gradient::Linear {
            start_color: Color {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
            end_color: Color {
                red: 0.0,
                green: 0.0,
                blue: 1.0,
                alpha: 1.0,
            },
            start: [0.0, 0.0],
            end: [1.0, 1.0],
        }
    );
    assert_eq!(list.layers.len(), 1);
    assert_eq!(list.layers[0].opacity, 0.5);
    assert_eq!(list.layers[0].blur, 0.0);
    assert_eq!(list.layers[0].shadow_color.alpha, 0.0);
    let instance = SdfQuadInstance::from_command(&list.commands[0], 120).unwrap();
    assert_eq!(instance.gradient_data[0], 1.0);
    assert_eq!(instance.gradient_points, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(instance.radii, [4.0, 12.0, 4.0, 4.0]);
}

#[test]
fn images_and_icons_emit_texture_commands() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let image = scene.create(Element::Image);
    let icon = scene.create(Element::Icon);
    scene
        .assign(image, "source", "/tmp/wallpaper.webp")
        .unwrap();
    scene.assign(image, "width", 40.0).unwrap();
    scene.assign(image, "height", 20.0).unwrap();
    scene
        .assign(image, "fill_mode", "preserve_aspect_fit")
        .unwrap();
    scene.assign(icon, "name", "network-wireless").unwrap();
    scene.assign(icon, "theme", "Adwaita").unwrap();
    scene.assign(icon, "width", 16.0).unwrap();
    scene.assign(icon, "height", 16.0).unwrap();
    scene.reparent(image, Some(root)).unwrap();
    scene.reparent(icon, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert!(matches!(
        &list.commands[0],
        DrawCommand::Texture {
            source,
            icon_theme: None,
            fill_mode: ImageFillMode::PreserveAspectFit,
            ..
        } if source == "/tmp/wallpaper.webp"
    ));
    assert!(matches!(
        &list.commands[1],
        DrawCommand::Texture { source, icon_theme: Some(theme), .. }
            if source == "network-wireless" && theme == "Adwaita"
    ));
}

#[test]
fn distance_field_edge_style_reaches_the_draw_list() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let icon = scene.create(Element::Image);
    scene.assign(root, "width", 64.0).unwrap();
    scene.assign(root, "height", 64.0).unwrap();
    scene.assign(icon, "width", 32.0).unwrap();
    scene.assign(icon, "height", 32.0).unwrap();
    scene.assign(icon, "source", "star.svg").unwrap();
    scene.assign(icon, "distance_field", true).unwrap();
    scene.assign(icon, "distance_field_weight", 0.38).unwrap();
    scene.assign(icon, "distance_field_softness", 1.5).unwrap();
    scene
        .assign(icon, "distance_field_outline_width", 2.0)
        .unwrap();
    scene
        .assign(icon, "distance_field_outline_color", "#ff0000")
        .unwrap();
    scene.reparent(icon, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 64.0,
            height: 64.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Texture {
        distance_field_style,
        ..
    } = &list.commands[0]
    else {
        panic!("image did not emit a texture command");
    };
    assert_eq!(distance_field_style.weight, 0.38);
    assert_eq!(distance_field_style.softness, 1.5);
    assert_eq!(distance_field_style.outline_width, 2.0);
    assert_eq!(
        distance_field_style.outline_color,
        Color::rgba8(255, 0, 0, 255)
    );
}

#[test]
fn animating_the_field_edge_repaints_without_touching_the_cached_field() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let icon = scene.create(Element::Icon);
    scene.assign(root, "width", 64.0).unwrap();
    scene.assign(root, "height", 64.0).unwrap();
    scene.assign(icon, "width", 32.0).unwrap();
    scene.assign(icon, "height", 32.0).unwrap();
    scene.assign(icon, "name", "battery").unwrap();
    scene.assign(icon, "distance_field", true).unwrap();
    scene.reparent(icon, Some(root)).unwrap();
    let size = Size {
        width: 64.0,
        height: 64.0,
    };
    let layout = Layout::compute(&scene, root, size, &mut NoText).unwrap();
    let before = DrawList::from_scene(&scene, &layout).unwrap();

    scene.assign(icon, "distance_field_weight", 0.42).unwrap();
    let layout = Layout::compute(&scene, root, size, &mut NoText).unwrap();
    let after = DrawList::from_scene(&scene, &layout).unwrap();

    let source = |list: &DrawList| {
        let DrawCommand::Texture {
            source,
            distance_field_spread,
            ..
        } = &list.commands[0]
        else {
            panic!("icon did not emit a texture command");
        };
        (source.clone(), *distance_field_spread)
    };
    // A new edge is a new draw command, but the cache key that decides whether
    // the CPU distance transform reruns is untouched.
    assert_ne!(before.commands[0], after.commands[0]);
    assert_eq!(source(&before), source(&after));
}
