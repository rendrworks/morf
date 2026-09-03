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

fn compute(scene: &Scene, root: NodeHandle, width: f64, height: f64) -> Layout {
    Layout::compute(scene, root, Size { width, height }, &mut FixedText).unwrap()
}

#[test]
fn a_flex_row_grows_its_fillers_around_fixed_children_with_gaps() {
    let mut scene = Scene::new();
    let flex = sized(&mut scene, Element::Flex, 300.0, 40.0);
    scene.assign(flex, "gap", 10.0).unwrap();
    scene.assign(flex, "align", "center").unwrap();
    let fixed = sized(&mut scene, Element::Rect, 50.0, 20.0);
    let grows = sized(&mut scene, Element::Rect, 0.0, 20.0);
    attached(&mut scene, grows, &[("grow", Value::Number(1.0))]);
    let grows_twice = sized(&mut scene, Element::Rect, 0.0, 40.0);
    attached(&mut scene, grows_twice, &[("grow", Value::Number(2.0))]);
    for child in [fixed, grows, grows_twice] {
        scene.reparent(child, Some(flex)).unwrap();
    }

    let layout = compute(&scene, flex, 300.0, 40.0);

    // 300 - 50 - two gaps of 10 = 230 to grow into, one third and two thirds.
    let fixed = layout.geometry(fixed).unwrap();
    assert_eq!((fixed.x, fixed.y, fixed.width), (0.0, 10.0, 50.0));
    let grows = layout.geometry(grows).unwrap();
    assert!((grows.width - 230.0 / 3.0).abs() < 0.01, "{grows:?}");
    assert_eq!(grows.x, 60.0);
    let twice = layout.geometry(grows_twice).unwrap();
    assert!((twice.width - 460.0 / 3.0).abs() < 0.01, "{twice:?}");
    assert_eq!(twice.y, 0.0);
}

#[test]
fn a_flex_row_shrinks_by_size_and_honours_a_percent_minimum() {
    let mut scene = Scene::new();
    let flex = sized(&mut scene, Element::Flex, 100.0, 10.0);
    let a = sized(&mut scene, Element::Rect, 60.0, 10.0);
    // A flex item will not shrink below its content on its own; say so.
    attached(&mut scene, a, &[("minimum_width", Value::Number(0.0))]);
    let b = sized(&mut scene, Element::Rect, 60.0, 10.0);
    attached(&mut scene, b, &[("shrink", Value::Number(0.0))]);
    let c = sized(&mut scene, Element::Rect, 60.0, 10.0);
    attached(
        &mut scene,
        c,
        &[("minimum_width", Value::String("50%".into()))],
    );
    for child in [a, b, c] {
        scene.reparent(child, Some(flex)).unwrap();
    }

    let layout = compute(&scene, flex, 100.0, 10.0);

    // b refuses to shrink and c stops at half the row; a takes the rest.
    assert_eq!(layout.geometry(b).unwrap().width, 60.0);
    assert_eq!(layout.geometry(c).unwrap().width, 50.0);
    assert_eq!(layout.geometry(a).unwrap().width, 0.0);
}

#[test]
fn a_flex_column_wraps_text_at_the_width_it_gives_it_and_is_that_tall() {
    // The Flex has no height of its own: it is as tall as its content, and
    // its content is text shaped at the Flex's width.
    let mut scene = Scene::new();
    let root = sized(&mut scene, Element::Item, 100.0, 300.0);
    let flex = scene.create(Element::Flex);
    scene.assign(flex, "direction", "column").unwrap();
    scene.assign(flex, "width", 100.0).unwrap();
    let text = scene.create(Element::Text);
    scene.assign(text, "text", "x".repeat(40)).unwrap();
    scene.assign(text, "font_size", 10.0).unwrap();
    scene.assign(text, "wrap", true).unwrap();
    scene.reparent(text, Some(flex)).unwrap();
    scene.reparent(flex, Some(root)).unwrap();

    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: 100.0,
            height: 300.0,
        },
        &mut WrapText,
    )
    .unwrap();

    let text = layout.geometry(text).unwrap();
    assert_eq!((text.width, text.height), (100.0, 20.0));
    assert_eq!(layout.geometry(flex).unwrap().height, 20.0);
}

#[test]
fn a_grid_with_tracks_places_children_in_fractions_and_spans() {
    let mut scene = Scene::new();
    let grid = sized(&mut scene, Element::Grid, 200.0, 100.0);
    scene
        .assign(
            grid,
            "template_columns",
            Value::List(vec![
                Value::String("1fr".into()),
                Value::String("3fr".into()),
            ]),
        )
        .unwrap();
    scene
        .assign(
            grid,
            "template_rows",
            Value::List(vec![Value::String("repeat(2, 1fr)".into())]),
        )
        .unwrap();
    scene.assign(grid, "column_spacing", 20.0).unwrap();
    let wide = scene.create(Element::Rect);
    attached(
        &mut scene,
        wide,
        &[
            ("column", Value::Number(1.0)),
            ("column_span", Value::Number(2.0)),
            ("row", Value::Number(1.0)),
        ],
    );
    let small = scene.create(Element::Rect);
    attached(
        &mut scene,
        small,
        &[("column", Value::Number(2.0)), ("row", Value::Number(2.0))],
    );
    // A leaf keeps its own children by the ordinary rules.
    let label = sized(&mut scene, Element::Rect, 10.0, 10.0);
    scene
        .assign(
            label,
            "anchors",
            Value::Map(BTreeMap::from([("right".to_owned(), Value::Bool(true))])),
        )
        .unwrap();
    scene.reparent(label, Some(small)).unwrap();
    for child in [wide, small] {
        scene.reparent(child, Some(grid)).unwrap();
    }

    let layout = compute(&scene, grid, 200.0, 100.0);

    // Tracks: (200 - 20) split 1:3 = 45 and 135; rows 50 each.
    let wide = layout.geometry(wide).unwrap();
    assert_eq!(
        (wide.x, wide.y, wide.width, wide.height),
        (0.0, 0.0, 200.0, 50.0)
    );
    let small = layout.geometry(small).unwrap();
    assert_eq!(
        (small.x, small.y, small.width, small.height),
        (65.0, 50.0, 135.0, 50.0)
    );
    let label = layout.geometry(label).unwrap();
    assert_eq!(label.x, 65.0 + 135.0 - 10.0);
}

#[test]
fn a_flex_child_may_not_anchor_and_a_bad_word_is_named() {
    let mut scene = Scene::new();
    let flex = sized(&mut scene, Element::Flex, 100.0, 100.0);
    scene.assign(flex, "justify", "sideways").unwrap();
    let child = sized(&mut scene, Element::Rect, 10.0, 10.0);
    scene.reparent(child, Some(flex)).unwrap();

    let error = Layout::compute(
        &scene,
        flex,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap_err();
    assert!(error.to_string().contains("sideways"), "{error}");

    scene.assign(flex, "justify", "start").unwrap();
    scene
        .assign(
            child,
            "anchors",
            Value::Map(BTreeMap::from([("left".to_owned(), Value::Bool(true))])),
        )
        .unwrap();
    let error = Layout::compute(
        &scene,
        flex,
        Size {
            width: 100.0,
            height: 100.0,
        },
        &mut FixedText,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("anchors inside a Flex"),
        "{error}"
    );
}
