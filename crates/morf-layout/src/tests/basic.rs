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
    scene.assign(row, "spacing", 5.0).unwrap();
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
