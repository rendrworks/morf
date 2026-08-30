use super::*;
#[test]
fn transform_watcher_tracks_layout_and_ancestor_changes() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Item);
    scene.reparent(parent, Some(root)).unwrap();
    scene.reparent(child, Some(parent)).unwrap();
    let mut tracker = TransformTracker::default();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 50.0,
        },
        &mut FixedText,
    )
    .unwrap();
    tracker.update(&layout);
    let mut watcher = TransformWatcher::new(root, child, Some(root));
    assert!(!watcher.observe(&scene, &tracker).unwrap());

    scene.assign(parent, "rotation", 30.0).unwrap();
    assert!(watcher.observe(&scene, &tracker).unwrap());
    assert!(!watcher.observe(&scene, &tracker).unwrap());

    scene.assign(child, "x", 12.0).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 50.0,
        },
        &mut FixedText,
    )
    .unwrap();
    tracker.update(&layout);
    assert!(watcher.observe(&scene, &tracker).unwrap());
}

#[test]
fn transform_tracker_maps_node_local_geometry() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Item);
    scene.assign(parent, "x", 10.0).unwrap();
    scene.assign(child, "x", 5.0).unwrap();
    scene.assign(child, "y", 3.0).unwrap();
    scene.assign(child, "width", 20.0).unwrap();
    scene.assign(child, "height", 10.0).unwrap();
    scene.reparent(parent, Some(root)).unwrap();
    scene.reparent(child, Some(parent)).unwrap();
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 50.0,
        },
        &mut FixedText,
    )
    .unwrap();
    let mut tracker = TransformTracker::default();
    tracker.update(&layout);

    assert_eq!(
        tracker.map_from_node(&scene, child, 2.0, 4.0).unwrap(),
        Some((17.0, 7.0))
    );
    assert_eq!(
        tracker
            .map_rect_from_node(
                &scene,
                child,
                Geometry {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 10.0,
                }
            )
            .unwrap(),
        Some(Geometry {
            x: 15.0,
            y: 3.0,
            width: 20.0,
            height: 10.0,
        })
    );
}

#[test]
fn fill_anchors_respect_margins() {
    let mut scene = Scene::new();
    let parent = scene.create(Element::Item);
    let child = scene.create(Element::Rect);
    let anchors = BTreeMap::from([
        ("fill".to_owned(), Value::Bool(true)),
        ("margins".to_owned(), Value::Number(4.0)),
    ]);
    scene.assign(child, "anchors", Value::Map(anchors)).unwrap();
    scene.reparent(child, Some(parent)).unwrap();

    let layout = Layout::compute(
        &scene,
        parent,
        Size {
            width: 80.0,
            height: 40.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(
        layout.geometry(child).unwrap(),
        Geometry {
            x: 4.0,
            y: 4.0,
            width: 72.0,
            height: 32.0,
        }
    );
}

#[test]
fn anchors_cannot_compete_with_a_positioner_axis() {
    let mut scene = Scene::new();
    let row = scene.create(Element::Row);
    let child = scene.create(Element::Item);
    scene
        .assign(
            child,
            "anchors",
            Value::Map(BTreeMap::from([("left".to_owned(), Value::Bool(true))])),
        )
        .unwrap();
    scene.reparent(child, Some(row)).unwrap();

    let error = Layout::compute(
        &scene,
        row,
        Size {
            width: 100.0,
            height: 20.0,
        },
        &mut FixedText,
    )
    .unwrap_err();

    assert_eq!(error, LayoutError::AxisConflict { axis: "horizontal" });
}
