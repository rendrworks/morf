use super::*;

/// Builds a field with the given layers, and lays it out at 200x200.
pub(super) fn field_scene(layers: &[(Element, &[(&str, Value)])]) -> (Scene, NodeHandle, Layout) {
    let mut scene = Scene::new();
    let root = scene.create(Element::Sdf);
    scene.assign(root, "width", 200.0).unwrap();
    scene.assign(root, "height", 200.0).unwrap();
    for (element, properties) in layers {
        let child = scene.create(*element);
        for (name, value) in *properties {
            scene.assign(child, name, value.clone()).unwrap();
        }
        scene.reparent(child, Some(root)).unwrap();
    }
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    (scene, root, layout)
}

#[test]
fn a_field_absorbs_the_rects_beneath_it_and_leaves_everything_else_alone() {
    // The rule that makes fields the foundation rather than a separate kind of
    // drawing: anything with a shape of its own becomes a layer, and is not
    // then drawn a second time as itself. A `Text` has no field, so it paints
    // over the composition exactly as it always did.
    let (scene, _, layout) = field_scene(&[
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(40.0)),
                ("height", Value::Number(40.0)),
                ("shape", Value::String("star".into())),
            ],
        ),
        (
            Element::Rect,
            &[
                ("x", Value::Number(20.0)),
                ("width", Value::Number(60.0)),
                ("height", Value::Number(30.0)),
                ("radius", Value::Number(9.0)),
            ],
        ),
        (
            Element::Text,
            &[
                ("width", Value::Number(50.0)),
                ("height", Value::Number(12.0)),
            ],
        ),
    ]);
    let list = DrawList::from_scene(&scene, &layout).unwrap();

    let DrawCommand::Field { layers, .. } = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap()
    else {
        unreachable!("found a field")
    };
    assert_eq!(layers.len(), 2, "the rect became a layer");
    assert_eq!(layers[0].shape, SdfShapeKind::Star);
    assert_eq!(layers[1].shape, SdfShapeKind::Box);
    assert_eq!(
        layers[1].radii, [9.0; 4],
        "the rect keeps its corner radius"
    );

    assert!(
        !list
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Quad { .. })),
        "an absorbed rect must not also paint as a rect"
    );
    assert!(
        list.commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Text { .. })),
        "text has no field, so it still paints"
    );
}

#[test]
fn a_field_reaches_through_the_positioners_that_laid_its_shapes_out() {
    // The point of descending: a row of rects positioned by the ordinary layout
    // engine arrives as a row of fields to fuse, without anything in the
    // configuration having been written for the field.
    let mut scene = Scene::new();
    let root = scene.create(Element::Sdf);
    scene.assign(root, "width", 300.0).unwrap();
    scene.assign(root, "height", 80.0).unwrap();
    scene.assign(root, "blend", 18.0).unwrap();
    let row = scene.create(Element::Row);
    scene.assign(row, "spacing", 12.0).unwrap();
    scene.reparent(row, Some(root)).unwrap();
    for _ in 0..3 {
        let cell = scene.create(Element::Rect);
        scene.assign(cell, "width", 60.0).unwrap();
        scene.assign(cell, "height", 60.0).unwrap();
        scene.assign(cell, "radius", 14.0).unwrap();
        scene.reparent(cell, Some(row)).unwrap();
    }
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 300.0,
            height: 80.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();

    let DrawCommand::Field { layers, .. } = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap()
    else {
        unreachable!("found a field")
    };
    assert_eq!(layers.len(), 3);
    // The row's spacing is in the layer positions, so the field composes what
    // layout produced rather than a second set of coordinates.
    assert_eq!(layers[0].bounds.x, 0.0);
    assert_eq!(layers[1].bounds.x, 72.0);
    assert_eq!(layers[2].bounds.x, 144.0);
    // And the field's own blend reached every one of them, so they fuse.
    for layer in layers {
        assert_eq!(layer.operation, SdfOperation::SmoothUnion);
        assert_eq!(layer.blend, 18.0);
    }
}

#[test]
fn a_field_without_a_blend_composes_the_same_shapes_with_hard_edges() {
    // The container decides whether its contents fuse. Nothing about the rects
    // changes between the two cases.
    let (scene, _, layout) = field_scene(&[
        (
            Element::Rect,
            &[
                ("width", Value::Number(40.0)),
                ("height", Value::Number(40.0)),
            ],
        ),
        (
            Element::Rect,
            &[
                ("x", Value::Number(50.0)),
                ("width", Value::Number(40.0)),
                ("height", Value::Number(40.0)),
            ],
        ),
    ]);
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Field { layers, .. } = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap()
    else {
        unreachable!("found a field")
    };
    for layer in layers {
        assert_eq!(layer.operation, SdfOperation::Union);
        assert_eq!(layer.blend, 0.0);
    }
}

#[test]
fn a_nested_field_keeps_its_own_composition() {
    // A field inside a field has its own fill and its own blend. Folding its
    // layers into the parent would silently discard both, so the walk stops
    // and the inner field paints itself.
    let mut scene = Scene::new();
    let root = scene.create(Element::Sdf);
    scene.assign(root, "width", 200.0).unwrap();
    scene.assign(root, "height", 200.0).unwrap();
    let outer = scene.create(Element::Rect);
    scene.assign(outer, "width", 50.0).unwrap();
    scene.assign(outer, "height", 50.0).unwrap();
    scene.reparent(outer, Some(root)).unwrap();
    let inner = scene.create(Element::Sdf);
    scene.assign(inner, "width", 80.0).unwrap();
    scene.assign(inner, "height", 80.0).unwrap();
    scene.reparent(inner, Some(root)).unwrap();
    let deep = scene.create(Element::Rect);
    scene.assign(deep, "width", 30.0).unwrap();
    scene.assign(deep, "height", 30.0).unwrap();
    scene.reparent(deep, Some(inner)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();

    let fields: Vec<_> = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::Field { layers, .. } => Some(layers.len()),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec![1, 1], "one layer each, not two and none");
}

#[test]
fn a_field_with_no_layers_paints_nothing() {
    // Every operator starts from "infinitely far outside". A composition with
    // nothing in it therefore has no zero crossing at all, and emitting it
    // would fill the node's whole rectangle.
    let (scene, _, layout) = field_scene(&[]);
    let list = DrawList::from_scene(&scene, &layout).unwrap();

    assert!(
        !list
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Field { .. }))
    );
}

#[test]
fn an_invisible_layer_leaves_the_composition_rather_than_emptying_it() {
    // Hiding a subtracted layer must remove the subtraction, not leave an empty
    // field behind that would punch the hole everywhere instead of nowhere.
    let (mut scene, root, _) = field_scene(&[
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(100.0)),
                ("height", Value::Number(100.0)),
            ],
        ),
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(40.0)),
                ("height", Value::Number(40.0)),
                ("operation", Value::String("subtract".into())),
            ],
        ),
    ]);
    let hole = scene.children(root).unwrap()[1];
    scene.assign(hole, "visible", false).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();

    let DrawCommand::Field { layers, .. } = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap()
    else {
        unreachable!("found a field")
    };
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].operation, SdfOperation::Union);
}

#[test]
fn an_unknown_shape_or_operation_is_an_error_rather_than_a_default() {
    // A typo in a shape name must surface where it was written instead of
    // quietly drawing a circle.
    let (scene, _, layout) = field_scene(&[(
        Element::SdfShape,
        &[
            ("width", Value::Number(10.0)),
            ("height", Value::Number(10.0)),
            ("shape", Value::String("trapezoid".into())),
        ],
    )]);
    let error = DrawList::from_scene(&scene, &layout).unwrap_err();
    assert!(format!("{error}").contains("trapezoid"), "{error}");

    let (scene, _, layout) = field_scene(&[(
        Element::SdfShape,
        &[
            ("width", Value::Number(10.0)),
            ("height", Value::Number(10.0)),
            ("operation", Value::String("blend".into())),
        ],
    )]);
    let error = DrawList::from_scene(&scene, &layout).unwrap_err();
    assert!(format!("{error}").contains("blend"), "{error}");
}

#[test]
fn a_field_drives_the_morph_of_every_layer_that_does_not_speak_for_itself() {
    // A compound shape is several layers that have to move together. Driving
    // them from the container makes the whole compound one animatable number,
    // rather than N numbers a configuration has to keep in step by hand — which
    // is how a config acquires a frame runtime.
    let (scene, root, _) = field_scene(&[
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(40.0)),
                ("height", Value::Number(40.0)),
                ("shape", Value::String("circle".into())),
                ("morph_to", Value::String("capsule".into())),
            ],
        ),
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(20.0)),
                ("height", Value::Number(20.0)),
                ("shape", Value::String("ring".into())),
                ("morph_to", Value::String("box".into())),
            ],
        ),
        (
            Element::SdfShape,
            &[
                ("width", Value::Number(10.0)),
                ("height", Value::Number(10.0)),
                ("shape", Value::String("star".into())),
                // This one opts out and holds its own position.
                ("morph_progress", Value::Number(0.25)),
            ],
        ),
    ]);
    let mut scene = scene;
    scene.assign(root, "morph_progress", 0.75).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 200.0,
        },
        &mut NoText,
    )
    .unwrap();
    let list = DrawList::from_scene(&scene, &layout).unwrap();
    let DrawCommand::Field { layers, .. } = list
        .commands
        .iter()
        .find(|command| matches!(command, DrawCommand::Field { .. }))
        .unwrap()
    else {
        unreachable!("found a field")
    };

    assert_eq!(layers[0].morph, 0.75, "follows the field");
    assert_eq!(layers[1].morph, 0.75, "follows the field");
    assert_eq!(layers[2].morph, 0.25, "keeps its own");
}
