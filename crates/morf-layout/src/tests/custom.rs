use super::*;

/// The bar layout every shell draws by hand: first child at the left
/// edge, last at the right, the one between centred.
struct Align;

impl CustomLayout for Align {
    fn measure(
        &mut self,
        _: NodeHandle,
        available: Size,
        children: &[Size],
    ) -> Result<Size, String> {
        Ok(Size {
            width: if available.width.is_finite() {
                available.width
            } else {
                children.iter().map(|size| size.width).sum()
            },
            height: children.iter().map(|size| size.height).fold(0.0, f64::max),
        })
    }

    fn place(
        &mut self,
        _: NodeHandle,
        bounds: Size,
        children: &[Size],
    ) -> Result<Vec<Geometry>, String> {
        let mut out = Vec::new();
        for (index, size) in children.iter().enumerate() {
            let x = match index {
                0 => 0.0,
                index if index == children.len() - 1 => bounds.width - size.width,
                _ => (bounds.width - size.width) / 2.0,
            };
            out.push(Geometry {
                x,
                y: (bounds.height - size.height) / 2.0,
                width: size.width,
                height: size.height,
            });
        }
        Ok(out)
    }
}

fn sized(scene: &mut Scene, element: Element, width: f64, height: f64) -> NodeHandle {
    let node = scene.create(element);
    scene.assign(node, "width", width).unwrap();
    scene.assign(node, "height", height).unwrap();
    node
}

#[test]
fn a_custom_container_is_measured_and_placed_by_its_host() {
    let mut scene = Scene::new();
    let bar = sized(&mut scene, Element::Custom, 300.0, 40.0);
    let left = sized(&mut scene, Element::Rect, 50.0, 20.0);
    let middle = sized(&mut scene, Element::Rect, 100.0, 40.0);
    let right = sized(&mut scene, Element::Rect, 30.0, 10.0);
    // A child's own children still go by the ordinary rules.
    let inner = sized(&mut scene, Element::Rect, 10.0, 10.0);
    scene
        .assign(
            inner,
            "anchors",
            Value::Map(BTreeMap::from([("right".to_owned(), Value::Bool(true))])),
        )
        .unwrap();
    scene.reparent(inner, Some(right)).unwrap();
    for child in [left, middle, right] {
        scene.reparent(child, Some(bar)).unwrap();
    }
    let available = Size {
        width: 300.0,
        height: 40.0,
    };

    let layout = Layout::compute_with(&scene, bar, available, &mut FixedText, &mut Align).unwrap();

    assert_eq!(layout.geometry(left).unwrap().x, 0.0);
    assert_eq!(layout.geometry(left).unwrap().y, 10.0);
    assert_eq!(layout.geometry(middle).unwrap().x, 100.0);
    assert_eq!(layout.geometry(right).unwrap().x, 270.0);
    assert_eq!(layout.geometry(inner).unwrap().x, 290.0);

    // Without a host, the container is an error that says so.
    let error = Layout::compute(&scene, bar, available, &mut FixedText).unwrap_err();
    assert!(error.to_string().contains("needs a host"), "{error}");
}
