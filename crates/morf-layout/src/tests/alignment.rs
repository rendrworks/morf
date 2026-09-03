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
fn a_row_layout_shares_leftover_space_by_stretch_and_shrinks_by_size() {
    let mut scene = Scene::new();
    let row = sized(&mut scene, Element::RowLayout, 100.0, 10.0);
    let fixed = sized(&mut scene, Element::Rect, 10.0, 10.0);
    let one = sized(&mut scene, Element::Rect, 10.0, 10.0);
    attached(&mut scene, one, &[("fill_width", Value::Bool(true))]);
    let three = sized(&mut scene, Element::Rect, 10.0, 10.0);
    attached(
        &mut scene,
        three,
        &[
            ("fill_width", Value::Bool(true)),
            ("stretch", Value::Number(3.0)),
        ],
    );
    for child in [fixed, one, three] {
        scene.reparent(child, Some(row)).unwrap();
    }
    let available = Size {
        width: 100.0,
        height: 10.0,
    };
    let layout = Layout::compute(&scene, row, available, &mut FixedText).unwrap();
    // 70 left over: one share to `one`, three to `three`.
    assert_eq!(layout.geometry(fixed).unwrap().width, 10.0);
    assert_eq!(layout.geometry(one).unwrap().width, 10.0 + 17.5);
    assert_eq!(layout.geometry(three).unwrap().width, 10.0 + 52.5);

    // Now too little room: 30 requested in 24, and one child refuses to
    // give anything up.
    let narrow = sized(&mut scene, Element::RowLayout, 24.0, 10.0);
    let a = sized(&mut scene, Element::Rect, 10.0, 10.0);
    let b = sized(&mut scene, Element::Rect, 10.0, 10.0);
    let c = sized(&mut scene, Element::Rect, 10.0, 10.0);
    attached(&mut scene, c, &[("shrink", Value::Number(0.0))]);
    for child in [a, b, c] {
        scene.reparent(child, Some(narrow)).unwrap();
    }
    let layout = Layout::compute(
        &scene,
        narrow,
        Size {
            width: 24.0,
            height: 10.0,
        },
        &mut FixedText,
    )
    .unwrap();
    assert_eq!(layout.geometry(a).unwrap().width, 7.0);
    assert_eq!(layout.geometry(b).unwrap().width, 7.0);
    assert_eq!(layout.geometry(c).unwrap().width, 10.0);
    assert_eq!(layout.geometry(c).unwrap().x, 14.0);
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
