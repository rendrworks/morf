use std::collections::BTreeMap;
use std::time::Duration;

use crate::*;

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn list(items: Vec<Value>) -> Value {
    Value::List(items)
}

#[test]
fn a_bare_list_of_colours_is_spread_evenly() {
    let gradient = Gradient::parse(&map(&[(
        "stops",
        list(vec!["#ff0000".into(), "#00ff00".into(), "#0000ff".into()]),
    )]))
    .unwrap()
    .unwrap();
    assert_eq!(gradient.kind, GradientKind::Linear);
    assert_eq!(
        gradient.angle, 180.0,
        "a linear gradient runs downwards by default"
    );
    assert_eq!(gradient.space, ColorSpace::Oklab);
    let positions: Vec<f64> = gradient.stops.iter().map(|stop| stop.position).collect();
    assert_eq!(positions, vec![0.0, 0.5, 1.0]);
    assert_eq!(gradient.stops[1].color, Color::rgba8(0, 255, 0, 255));
}

#[test]
fn a_missing_position_sits_between_its_placed_neighbours() {
    // Written positions stay; the unplaced stop lands halfway between the
    // two around it, and a stop behind an earlier one is pulled up to it.
    let gradient = Gradient::parse(&map(&[
        ("kind", "conic".into()),
        ("at", list(vec![0.25.into(), 0.75.into()])),
        (
            "stops",
            list(vec![
                list(vec!["#000000".into(), 0.2.into()]),
                "#ffffff".into(),
                map(&[("color", "#ff0000".into()), ("position", 0.8.into())]),
                list(vec!["#00ff00".into(), 0.1.into()]),
            ]),
        ),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(gradient.kind, GradientKind::Conic);
    assert_eq!(gradient.angle, 0.0);
    assert_eq!(gradient.at, [0.25, 0.75]);
    let positions: Vec<f64> = gradient.stops.iter().map(|stop| stop.position).collect();
    assert_eq!(positions, vec![0.2, 0.5, 0.8, 0.8]);
}

#[test]
fn a_gradient_says_what_is_wrong_with_it() {
    let error = |value: Value| Gradient::parse(&value).unwrap_err();
    assert_eq!(
        error(map(&[("stops", list(vec!["#fff".into()]))])),
        "a gradient needs at least two stops"
    );
    assert_eq!(
        error(map(&[
            ("kind", "swirl".into()),
            ("stops", list(vec!["#fff".into(), "#000".into()]))
        ])),
        "gradient kind `swirl` is not linear, radial or conic"
    );
    assert_eq!(
        error(map(&[("stops", list(vec!["#fff".into(), "nope".into()]))])),
        "`nope` is not a colour"
    );
    assert_eq!(
        error(map(&[
            ("angel", 3.0.into()),
            ("stops", list(vec!["#fff".into(), "#000".into()]))
        ])),
        "a gradient has no `angel`"
    );
    assert_eq!(
        error(map(&[(
            "stops",
            list((0..17).map(|_| Value::from("#fff")).collect())
        )])),
        "a gradient takes at most 16 stops"
    );
    assert_eq!(Gradient::parse(&Value::Nil).unwrap(), None);
    assert_eq!(Gradient::parse(&map(&[])).unwrap(), None);
}

#[test]
fn a_property_stores_the_canonical_form_and_animates_stop_by_stop() {
    // What a configuration wrote is read back with every default filled in,
    // and a behavior on the property moves each stop's colour and position.
    let mut scene = Scene::new();
    let rect = scene.create(Element::Rect);
    let written = map(&[("stops", list(vec!["#000000".into(), "#ffffff".into()]))]);
    scene.assign(rect, "gradient", written).unwrap();
    let stored = scene.current(rect, "gradient").unwrap().clone();
    let gradient = Gradient::parse(&stored).unwrap().unwrap();
    assert_eq!(gradient.to_value(), stored, "the stored value is canonical");
    let Value::Map(entries) = &stored else {
        panic!("a gradient is stored as a map")
    };
    assert_eq!(entries["angle"], Value::Number(180.0));
    assert_eq!(entries["space"], Value::String("oklab".to_owned()));

    scene
        .set_behavior(
            rect,
            "gradient",
            Some(Behavior {
                duration: Duration::from_millis(100),
                easing: Easing::Linear,
                color_space: ColorSpace::Srgb,
                ..Behavior::default()
            }),
        )
        .unwrap();
    scene
        .assign(
            rect,
            "gradient",
            map(&[(
                "stops",
                list(vec![
                    list(vec!["#000000".into(), 0.5.into()]),
                    "#000000".into(),
                ]),
            )]),
        )
        .unwrap();
    scene.tick_animations(Duration::from_millis(50)).unwrap();
    let halfway = Gradient::parse(scene.current(rect, "gradient").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(halfway.stops[0].position, 0.25);
    assert!(
        (halfway.stops[1].color.red - 0.5).abs() < 0.01,
        "the second stop is halfway to black: {:?}",
        halfway.stops[1].color
    );

    let wrong = scene
        .assign(rect, "gradient", map(&[("kind", "linear".into())]))
        .unwrap_err();
    assert_eq!(
        wrong.to_string(),
        "invalid Rect property `gradient`: a gradient needs a list of stops"
    );
}
