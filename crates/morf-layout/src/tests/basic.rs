use super::*;
#[test]
fn image_implicit_size_uses_source_dimensions() {
    let mut scene = Scene::new();
    let image = scene.create(Element::Image);
    scene.assign(image, "source", "/tmp/image.png").unwrap();

    let layout = Layout::compute(
        &scene,
        image,
        Size {
            width: 64.0,
            height: 32.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(
        layout.implicit_size(image),
        Some(Size {
            width: 64.0,
            height: 32.0
        })
    );
}

#[test]
fn clip_rect_keeps_content_inside_its_border() {
    let mut scene = Scene::new();
    let root = scene.create(Element::ClipRect);
    let child = scene.create(Element::Rect);
    scene.assign(root, "border_width", 3.0).unwrap();
    scene.assign(child, "implicit_width", 20.0).unwrap();
    scene.assign(child, "implicit_height", 10.0).unwrap();
    scene
        .assign(
            child,
            "anchors",
            Value::Map(BTreeMap::from([("fill".to_owned(), Value::Bool(true))])),
        )
        .unwrap();
    scene.reparent(child, Some(root)).unwrap();

    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 50.0,
            height: 30.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(
        layout.implicit_size(root),
        Some(Size {
            width: 26.0,
            height: 16.0,
        })
    );
    assert_eq!(
        layout.geometry(child),
        Some(Geometry {
            x: 3.0,
            y: 3.0,
            width: 44.0,
            height: 24.0,
        })
    );
}

#[test]
fn implicit_sizes_resolve_bottom_up_before_rows_distribute() {
    let mut scene = Scene::new();
    let row = scene.create(Element::Row);
    scene.assign(row, "gap", 5.0).unwrap();
    let first = scene.create(Element::Text);
    scene.assign(first, "text", "aa").unwrap();
    scene.assign(first, "font_size", 10.0).unwrap();
    let second = scene.create(Element::Rect);
    scene.assign(second, "width", 20.0).unwrap();
    scene.assign(second, "height", 8.0).unwrap();
    scene.reparent(first, Some(row)).unwrap();
    scene.reparent(second, Some(row)).unwrap();

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

    assert_eq!(layout.implicit_size(row).unwrap().width, 35.0);
    assert_eq!(layout.geometry(first).unwrap().x, 0.0);
    assert_eq!(layout.geometry(second).unwrap().x, 15.0);
}

#[test]
fn text_font_weight_reaches_the_measurer() {
    let mut scene = Scene::new();
    let text = scene.create(Element::Text);
    scene.assign(text, "font_weight", 650.0).unwrap();
    let mut measurer = WeightText(0.0);

    Layout::compute(&scene, text, Size::default(), &mut measurer).unwrap();

    assert_eq!(measurer.0, 650.0);
}

#[test]
fn inset_sizes_and_positions_its_single_child() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Inset);
    let child = scene.create(Element::Rect);
    scene.reparent(child, Some(root)).unwrap();
    scene.assign(root, "margin", 4.0).unwrap();
    scene.assign(root, "extra_margin", 2.0).unwrap();
    scene.assign(root, "left_margin", 10.0).unwrap();
    scene.assign(child, "implicit_width", 40.0).unwrap();
    scene.assign(child, "implicit_height", 20.0).unwrap();

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

    assert_eq!(
        layout.implicit_size(root),
        Some(Size {
            width: 58.0,
            height: 32.0
        })
    );
    assert_eq!(
        layout.geometry(child),
        Some(Geometry {
            x: 12.0,
            y: 6.0,
            width: 82.0,
            height: 38.0
        })
    );
}

#[test]
fn inset_can_preserve_child_size_and_distribute_space() {
    let mut scene = Scene::new();
    let root = scene.create(Element::Inset);
    let child = scene.create(Element::Item);
    scene.reparent(child, Some(root)).unwrap();
    scene.assign(root, "left_margin", 2.0).unwrap();
    scene.assign(root, "right_margin", 1.0).unwrap();
    scene.assign(root, "resize_child", false).unwrap();
    scene.assign(child, "implicit_width", 40.0).unwrap();
    scene.assign(child, "implicit_height", 20.0).unwrap();

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

    assert_eq!(
        layout.geometry(child),
        Some(Geometry {
            x: 40.0,
            y: 15.0,
            width: 40.0,
            height: 20.0
        })
    );
}

#[test]
fn a_backdrop_region_is_only_the_nodes_that_asked_for_one() {
    // The blur region and the input region answer different questions about
    // the same tree, and a node very often wants one without the other: a
    // frosted panel is usually not interactive, and a button usually does not
    // want its own blur. So this walk is separate from `input_geometry`, and
    // this test is what says the two do not drift into each other.
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let glass = scene.create(Element::Rect);
    let plain = scene.create(Element::Rect);
    for node in [glass, plain] {
        scene.assign(node, "width", 120.0).unwrap();
        scene.assign(node, "height", 40.0).unwrap();
        scene.reparent(node, Some(root)).unwrap();
    }
    scene.assign(glass, "x", 10.0).unwrap();
    scene.assign(glass, "y", 20.0).unwrap();
    scene.assign(glass, "radius", 8.0).unwrap();
    scene.assign(glass, "backdrop_blur", true).unwrap();

    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap();

    let regions = layout.backdrop_geometry(&scene).unwrap();
    assert_eq!(regions.len(), 1, "only the node that asked: {regions:?}");
    let (geometry, radii) = regions[0];
    assert_eq!((geometry.x, geometry.y), (10.0, 20.0));
    assert_eq!((geometry.width, geometry.height), (120.0, 40.0));
    assert_eq!(
        radii, [8.0; 4],
        "the corners travel with it, so the region the compositor blurs has the \
         same rounding as the glass drawn over it",
    );
}

#[test]
fn a_hidden_node_asks_for_no_backdrop() {
    // A region is double-buffered surface state that outlives the frame that
    // set it, so a panel that hides itself and leaves its blur behind would
    // leave a rounded rectangle of blurred desktop with nothing on it.
    let mut scene = Scene::new();
    let root = scene.create(Element::Item);
    let glass = scene.create(Element::Rect);
    scene.assign(glass, "width", 120.0).unwrap();
    scene.assign(glass, "height", 40.0).unwrap();
    scene.assign(glass, "backdrop_blur", true).unwrap();
    scene.assign(glass, "visible", false).unwrap();
    scene.reparent(glass, Some(root)).unwrap();

    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 200.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert!(layout.backdrop_geometry(&scene).unwrap().is_empty());
}
