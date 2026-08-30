use std::collections::BTreeMap;

use super::*;
#[test]
fn draw_list_preserves_tree_paint_order() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let first = scene.create(Element::Rect);
    let second = scene.create(Element::Text);
    scene.assign(first, "width", 20.0).unwrap();
    scene.assign(first, "height", 10.0).unwrap();
    scene.reparent(first, Some(root)).unwrap();
    scene.reparent(second, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 20.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert_eq!(list.commands[0].node(), first);
    assert_eq!(list.commands[1].node(), second);
}

#[test]
fn clip_rect_overlays_its_border_after_children() {
    let mut scene = Scene::new();
    let root = scene.create(Element::ClipRect);
    let child = scene.create(Element::Rect);
    scene.assign(root, "radius", 8.0).unwrap();
    scene.assign(root, "border_width", 2.0).unwrap();
    scene.assign(root, "border_color", "#ffffffff").unwrap();
    scene.assign(child, "width", 20.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.reparent(child, Some(root)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 40.0,
            height: 30.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert_eq!(list.commands.len(), 3);
    assert_eq!(list.commands[0].node(), root);
    assert_eq!(list.commands[1].node(), child);
    assert_eq!(list.commands[2].node(), root);
    let DrawCommand::Quad {
        border_width: background_border,
        ..
    } = list.commands[0]
    else {
        panic!("clip background did not emit a quad");
    };
    let DrawCommand::Quad {
        color,
        border_width,
        ..
    } = list.commands[2]
    else {
        panic!("clip border did not emit a quad");
    };
    assert_eq!(background_border, 0.0);
    assert_eq!(color.alpha, 0.0);
    assert_eq!(border_width, 2.0);
    assert_eq!(list.layers[0].mask.unwrap().radii, [8.0; 4]);
    assert_eq!(list.layers[1].parent, Some(0));
    assert_eq!(list.layers[1].mask.unwrap().radii, [6.0; 4]);
    assert_eq!(list.layers[1].bounds.width, 36.0);
    assert_eq!(list.layers[1].bounds.height, 26.0);
}

#[test]
fn color_overlay_propagates_through_a_subtree() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    let overlay = Color::rgba8(255, 0, 0, 128);
    scene.assign(root, "color_overlay", "#ff000080").unwrap();
    scene.assign(child, "width", 20.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.reparent(child, Some(root)).unwrap();
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
    let DrawCommand::Quad { color_overlay, .. } = &list.commands[0] else {
        panic!("child did not emit a quad");
    };
    assert_eq!(*color_overlay, overlay);
    assert_eq!(
        SdfQuadInstance::from_command(&list.commands[0], 120)
            .unwrap()
            .color_overlay,
        color_array(overlay)
    );
}

#[test]
fn nested_opacity_emits_composable_subtree_layers() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let group = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    scene.assign(root, "opacity", 0.5).unwrap();
    scene.assign(group, "opacity", 0.25).unwrap();
    scene.assign(child, "width", 20.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.reparent(group, Some(root)).unwrap();
    scene.reparent(child, Some(group)).unwrap();
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

    assert_eq!(list.layers.len(), 2);
    assert_eq!(list.layers[0].commands, 0..1);
    assert_eq!(list.layers[0].parent, None);
    assert_eq!(list.layers[0].opacity, 0.5);
    assert_eq!(list.layers[0].blur, 0.0);
    assert_eq!(list.layers[0].shadow_color.alpha, 0.0);
    assert_eq!(list.layers[1].commands, 0..1);
    assert_eq!(list.layers[1].parent, Some(0));
    assert_eq!(list.layers[1].opacity, 0.25);
    assert_eq!(list.layers[1].blur, 0.0);
    assert_eq!(list.layers[1].shadow_color.alpha, 0.0);
    let DrawCommand::Quad { color, .. } = list.commands[0] else {
        panic!("child did not emit a quad");
    };
    assert_eq!(color.alpha, 1.0);
}

#[test]
fn layer_enabled_map_forces_an_offscreen_subtree() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene
        .assign(
            rect,
            "layer",
            Value::Map(BTreeMap::from([
                ("enabled".to_owned(), Value::Bool(true)),
                ("blur".to_owned(), Value::Number(8.0)),
                (
                    "shadow_color".to_owned(),
                    Value::Color(Color::rgba8(8, 16, 24, 128)),
                ),
                ("shadow_blur".to_owned(), Value::Number(10.0)),
                ("shadow_offset_x".to_owned(), Value::Number(12.0)),
                ("shadow_offset_y".to_owned(), Value::Number(8.0)),
            ])),
        )
        .unwrap();
    scene.assign(rect, "width", 20.0).unwrap();
    scene.assign(rect, "height", 10.0).unwrap();
    let layout = Layout::compute(
        &scene,
        rect,
        Size {
            width: 20.0,
            height: 10.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert_eq!(list.layers.len(), 1);
    assert_eq!(list.layers[0].commands, 0..1);
    assert_eq!(list.layers[0].opacity, 1.0);
    assert_eq!(list.layers[0].blur, 8.0);
    assert_eq!(list.layers[0].shadow_color, Color::rgba8(8, 16, 24, 128));
    assert_eq!(list.layers[0].shadow_blur, 10.0);
    assert_eq!(list.layers[0].shadow_offset, [12.0, 8.0]);
    assert_eq!(list.layers[0].mask, None);
    assert_eq!(
        list.layers[0].bounds,
        Geometry {
            x: -16.0,
            y: -16.0,
            width: 68.0,
            height: 54.0,
        }
    );
    let DrawCommand::Quad { blur, .. } = list.commands[0] else {
        panic!("rectangle did not emit a quad");
    };
    assert_eq!(blur, 0.0);
}

#[test]
fn rounded_clip_emits_a_transformed_layer_mask() {
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    scene.assign(rect, "x", 10.0).unwrap();
    scene.assign(rect, "y", 20.0).unwrap();
    scene.assign(rect, "width", 40.0).unwrap();
    scene.assign(rect, "height", 30.0).unwrap();
    scene.assign(rect, "radius", 7.0).unwrap();
    scene.assign(rect, "clip", true).unwrap();
    scene.assign(rect, "rotation", 15.0).unwrap();
    let layout = Layout::compute(
        &scene,
        rect,
        Size {
            width: 80.0,
            height: 80.0,
        },
        &mut NoText,
    )
    .unwrap();

    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let mask = list.layers[0]
        .mask
        .expect("rounded clip did not emit a mask");

    assert_eq!(mask.bounds, layout.geometry(rect).unwrap());
    assert_eq!(mask.radii, [7.0; 4]);
    assert_ne!(mask.transform, Transform2D::IDENTITY);
}
