use super::*;

fn sized(scene: &mut Scene, element: Element, width: f64, height: f64) -> NodeHandle {
    let node = scene.create(element);
    scene.assign(node, "width", width).unwrap();
    scene.assign(node, "height", height).unwrap();
    node
}

fn attached(scene: &mut Scene, node: NodeHandle, entries: &[(&str, Value)]) {
    let map = entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    scene.assign(node, "layout", Value::Map(map)).unwrap();
}

#[test]
fn a_row_aligns_its_children_across_its_axis() {
    let mut scene = Scene::new();
    let row = sized(&mut scene, Element::Row, 100.0, 40.0);
    scene.assign(row, "alignment", "center").unwrap();
    let short = sized(&mut scene, Element::Rect, 10.0, 20.0);
    let tall = sized(&mut scene, Element::Rect, 10.0, 40.0);
    let ended = sized(&mut scene, Element::Rect, 10.0, 10.0);
    attached(
        &mut scene,
        ended,
        &[("alignment", Value::String("end".into()))],
    );
    let stretched = sized(&mut scene, Element::Rect, 10.0, 10.0);
    attached(
        &mut scene,
        stretched,
        &[("alignment", Value::String("stretch".into()))],
    );
    for child in [short, tall, ended, stretched] {
        scene.reparent(child, Some(row)).unwrap();
    }

    let layout = Layout::compute(
        &scene,
        row,
        Size {
            width: 100.0,
            height: 40.0,
        },
        &mut FixedText,
    )
    .unwrap();

    assert_eq!(layout.geometry(short).unwrap().y, 10.0);
    assert_eq!(layout.geometry(tall).unwrap().y, 0.0);
    assert_eq!(layout.geometry(ended).unwrap().y, 30.0);
    let stretched = layout.geometry(stretched).unwrap();
    assert_eq!((stretched.y, stretched.height), (0.0, 40.0));
}

#[test]
fn a_row_justifies_its_children_along_its_axis_with_the_old_and_new_words() {
    // `gap` and `spacing` are one thing, `align` and `alignment` are one
    // thing, and `justify` distributes what is left over.
    let mut scene = Scene::new();
    let row = sized(&mut scene, Element::Row, 100.0, 20.0);
    scene.assign(row, "spacing", 10.0).unwrap();
    scene.assign(row, "justify", "space_between").unwrap();
    let a = sized(&mut scene, Element::Rect, 10.0, 10.0);
    let b = sized(&mut scene, Element::Rect, 10.0, 10.0);
    let c = sized(&mut scene, Element::Rect, 10.0, 10.0);
    for child in [a, b, c] {
        scene.reparent(child, Some(row)).unwrap();
    }
    let layout = compute_row(&scene, row);
    assert_eq!(layout.geometry(a).unwrap().x, 0.0);
    assert_eq!(layout.geometry(b).unwrap().x, 45.0);
    assert_eq!(layout.geometry(c).unwrap().x, 90.0);

    scene.assign(row, "justify", "center").unwrap();
    scene.assign(row, "gap", 5.0).unwrap();
    let layout = compute_row(&scene, row);
    // 40 used with 5-gaps, 60 free, half of it before the first child.
    assert_eq!(layout.geometry(a).unwrap().x, 30.0);
    assert_eq!(layout.geometry(c).unwrap().x, 60.0);

    scene.assign(row, "justify", "sideways").unwrap();
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
    assert!(error.to_string().contains("sideways"), "{error}");
}

fn compute_row(scene: &Scene, row: NodeHandle) -> Layout {
    Layout::compute(
        scene,
        row,
        Size {
            width: 100.0,
            height: 20.0,
        },
        &mut FixedText,
    )
    .unwrap()
}

#[test]
fn wrapped_text_is_measured_at_the_width_its_parent_gives_it() {
    // A Text with no width of its own, filling an Inset: measured once
    // unconstrained it is one line, and its Column is one line tall. On
    // screen it wraps. The second pass measures it at the width it got.
    let mut scene = Scene::new();
    let column = sized(&mut scene, Element::Column, 100.0, 0.0);
    let inset = scene.create(Element::Inset);
    scene.assign(inset, "width", 100.0).unwrap();
    let text = scene.create(Element::Text);
    scene.assign(text, "text", "x".repeat(40)).unwrap(); // 200 wide at size 10
    scene.assign(text, "font_size", 10.0).unwrap();
    scene.assign(text, "wrap", true).unwrap();
    scene.reparent(text, Some(inset)).unwrap();
    scene.reparent(inset, Some(column)).unwrap();

    let layout = Layout::compute(
        &scene,
        column,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut WrapText,
    )
    .unwrap();

    let text = layout.geometry(text).unwrap();
    assert_eq!((text.width, text.height), (100.0, 20.0));
    assert_eq!(layout.geometry(inset).unwrap().height, 20.0);
}

#[test]
fn z_puts_a_child_over_its_later_siblings_for_hit_testing() {
    let mut scene = Scene::new();
    let root = sized(&mut scene, Element::Item, 100.0, 100.0);
    let under = sized(&mut scene, Element::MouseArea, 100.0, 100.0);
    scene.assign(under, "z", 1.0).unwrap();
    let over = sized(&mut scene, Element::MouseArea, 100.0, 100.0);
    scene.reparent(under, Some(root)).unwrap();
    scene.reparent(over, Some(root)).unwrap();

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

    // `under` comes first in the tree, but its `z` lifts it above `over`.
    assert_eq!(
        layout.hit_test(&scene, 5.0, 5.0).unwrap().unwrap().node,
        under
    );
    assert_eq!(
        scene.paint_order(root).unwrap().as_ref(),
        &[over, under][..]
    );
}

#[test]
fn text_options_that_would_do_nothing_are_refused() {
    let mut scene = Scene::new();
    let text = scene.create(Element::Text);
    scene.assign(text, "text", "hello").unwrap();
    scene.assign(text, "max_lines", 2.0).unwrap();
    let size = Size {
        width: 100.0,
        height: 100.0,
    };
    let error = Layout::compute(&scene, text, size, &mut FixedText).unwrap_err();
    assert!(error.to_string().contains("max_lines"), "{error}");

    scene.assign(text, "max_lines", 0.0).unwrap();
    scene.assign(text, "wrap", true).unwrap();
    scene.assign(text, "elide", "right").unwrap();
    let error = Layout::compute(&scene, text, size, &mut FixedText).unwrap_err();
    assert!(error.to_string().contains("elide"), "{error}");
}
