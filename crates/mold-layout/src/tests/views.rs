use super::*;
#[test]
fn hit_test_uses_absolute_geometry_and_paint_order() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let parent = scene.create(Element::Item);
    scene.assign(parent, "x", 10.0).unwrap();
    scene.assign(parent, "y", 5.0).unwrap();
    scene.assign(parent, "width", 40.0).unwrap();
    scene.assign(parent, "height", 20.0).unwrap();
    scene.reparent(parent, Some(root)).unwrap();
    let first = scene.create(Element::MouseArea);
    let second = scene.create(Element::MouseArea);
    for area in [first, second] {
        scene.assign(area, "x", 3.0).unwrap();
        scene.assign(area, "y", 2.0).unwrap();
        scene.assign(area, "width", 20.0).unwrap();
        scene.assign(area, "height", 10.0).unwrap();
        scene.reparent(area, Some(parent)).unwrap();
    }
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 40.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.geometry(second).unwrap().x, 13.0);
    assert_eq!(layout.hit_test(&scene, 15.0, 9.0).unwrap(), Some(second));
    scene.assign(second, "enabled", false).unwrap();
    assert_eq!(layout.hit_test(&scene, 15.0, 9.0).unwrap(), Some(first));
    assert_eq!(layout.hit_test(&scene, 2.0, 2.0).unwrap(), None);
    assert_eq!(
        layout.input_geometry(&scene).unwrap(),
        vec![layout.geometry(first).unwrap()]
    );
}

#[test]
fn hit_test_inverts_rotation_and_scale() {
    let mut scene = Scene::new();
    let area = scene.create(Element::MouseArea);
    scene.assign(area, "x", 20.0).unwrap();
    scene.assign(area, "y", 20.0).unwrap();
    scene.assign(area, "width", 40.0).unwrap();
    scene.assign(area, "height", 20.0).unwrap();
    scene.assign(area, "rotation", 90.0).unwrap();
    let layout = Layout::compute(
        &scene,
        area,
        Size {
            width: 40.0,
            height: 20.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.hit_test(&scene, 20.0, -5.0).unwrap(), Some(area));
    assert_eq!(layout.hit_test(&scene, 1.0, 1.0).unwrap(), None);
    let input = layout.input_geometry(&scene).unwrap();
    assert_eq!(input.len(), 1);
    assert!((input[0].x - 10.0).abs() < 0.000_001);
    assert!((input[0].y + 10.0).abs() < 0.000_001);
    assert!((input[0].width - 20.0).abs() < 0.000_001);
    assert!((input[0].height - 40.0).abs() < 0.000_001);

    scene.assign(area, "scale", 0.5).unwrap();
    assert_eq!(layout.hit_test(&scene, 20.0, 8.0).unwrap(), Some(area));
    assert_eq!(layout.hit_test(&scene, 20.0, -5.0).unwrap(), None);
}

#[test]
fn flickable_offsets_content_inside_its_viewport() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Flickable);
    let child = scene.create(Element::Rect);
    scene.assign(root, "content_x", 25.0).unwrap();
    scene.assign(root, "content_y", 80.0).unwrap();
    scene.assign(child, "x", 40.0).unwrap();
    scene.assign(child, "y", 120.0).unwrap();
    scene.assign(child, "width", 10.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.reparent(child, Some(root)).unwrap();

    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.geometry(child).unwrap().x, 15.0);
    assert_eq!(layout.geometry(child).unwrap().y, 40.0);
}

#[test]
fn grid_places_children_in_fixed_columns() {
    let mut scene = Scene::new();
    let grid = scene.create(Element::Grid);
    scene.assign(grid, "columns", 2.0).unwrap();
    scene.assign(grid, "column_spacing", 5.0).unwrap();
    scene.assign(grid, "row_spacing", 7.0).unwrap();
    let children = (0..4)
        .map(|_| {
            let child = scene.create(Element::Rect);
            scene.assign(child, "width", 20.0).unwrap();
            scene.assign(child, "height", 10.0).unwrap();
            scene.reparent(child, Some(grid)).unwrap();
            child
        })
        .collect::<Vec<_>>();

    let layout = Layout::compute(
        &scene,
        grid,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.geometry(children[1]).unwrap().x, 25.0);
    assert_eq!(layout.geometry(children[2]).unwrap().y, 17.0);
    assert_eq!(layout.implicit_size(grid).unwrap().width, 45.0);
}

#[test]
fn row_layout_distributes_remaining_width_to_fillers() {
    let mut scene = Scene::new();
    let row = scene.create(Element::RowLayout);
    scene.assign(row, "spacing", 10.0).unwrap();
    let fixed = scene.create(Element::Rect);
    scene.assign(fixed, "width", 30.0).unwrap();
    scene.assign(fixed, "height", 10.0).unwrap();
    let fill = scene.create(Element::Rect);
    scene.assign(fill, "width", 20.0).unwrap();
    scene.assign(fill, "height", 10.0).unwrap();
    scene
        .assign(
            fill,
            "layout",
            Value::Map(BTreeMap::from([("fill_width".into(), Value::Bool(true))])),
        )
        .unwrap();
    scene.reparent(fixed, Some(row)).unwrap();
    scene.reparent(fill, Some(row)).unwrap();

    let layout = Layout::compute(
        &scene,
        row,
        Size {
            width: 100.0,
            height: 20.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.geometry(fill).unwrap().width, 60.0);
    assert_eq!(layout.geometry(fill).unwrap().x, 40.0);
}

#[test]
fn reparent_transition_preserves_position_then_flies_to_target() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let left = scene.create(Element::Item);
    let right = scene.create(Element::Item);
    let tile = scene.create(Element::Rect);
    scene.assign(left, "x", 10.0).unwrap();
    scene.assign(left, "width", 100.0).unwrap();
    scene.assign(left, "height", 100.0).unwrap();
    scene.assign(right, "x", 200.0).unwrap();
    scene.assign(right, "width", 100.0).unwrap();
    scene.assign(right, "height", 100.0).unwrap();
    scene.assign(tile, "x", 5.0).unwrap();
    scene.assign(tile, "width", 20.0).unwrap();
    scene.assign(tile, "height", 20.0).unwrap();
    scene.reparent(left, Some(root)).unwrap();
    scene.reparent(right, Some(root)).unwrap();
    scene.reparent(tile, Some(left)).unwrap();
    let available = Size {
        width: 400.0,
        height: 200.0,
    };
    let behavior = Behavior {
        duration: std::time::Duration::from_millis(200),
        easing: mold_scene::Easing::Linear,
        rotation_direction: mold_scene::RotationDirection::Numerical,
    };

    let initial = Layout::transition_reparent(
        &mut scene,
        &mut FixedText,
        ReparentTransition {
            root,
            node: tile,
            new_parent: right,
            anchors: None,
            available,
            behavior,
        },
    )
    .unwrap();

    assert_eq!(scene.parent(tile).unwrap(), Some(right));
    assert_eq!(initial.geometry(tile).unwrap().x, 15.0);
    scene
        .tick_animations(std::time::Duration::from_millis(100))
        .unwrap();
    let halfway = Layout::compute(&scene, root, available, &mut FixedText).unwrap();
    assert_eq!(halfway.geometry(tile).unwrap().x, 110.0);
    scene
        .tick_animations(std::time::Duration::from_millis(100))
        .unwrap();
    let finished = Layout::compute(&scene, root, available, &mut FixedText).unwrap();
    assert_eq!(finished.geometry(tile).unwrap().x, 205.0);
}
