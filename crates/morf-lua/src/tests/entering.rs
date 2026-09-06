use std::time::Duration;

use morf_scene::Value;

use crate::*;

#[test]
fn enter_is_where_a_node_starts_and_its_behaviors_carry_it_from_there() {
    // The declared values are where the node settles; `enter` is where its
    // first frame starts. A property with a behavior travels between the two,
    // and one without simply arrives.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "enter.lua",
            br##"
                local ui = require("morf.ui")
                ui.Rect {
                    opacity = 1,
                    x = 100,
                    width = 40,
                    enter = { opacity = 0, x = 0, width = 10 },
                    behavior = {
                        opacity = { duration = 100, easing = "linear" },
                        x = { kind = "smoothed", velocity = 1000 },
                    },
                }
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    let current =
        |runtime: &Runtime, property: &str| runtime.scene().number(node, property).unwrap();
    assert_eq!(current(&runtime, "opacity"), 0.0, "starts where enter says");
    assert_eq!(current(&runtime, "x"), 0.0);
    assert_eq!(
        current(&runtime, "width"),
        40.0,
        "no behavior, so the declared value is already there"
    );
    assert_eq!(
        runtime.scene().target(node, "opacity").unwrap(),
        &Value::Number(1.0),
        "and is going where it was declared"
    );
    runtime.tick_animations(Duration::from_millis(50)).unwrap();
    assert!((current(&runtime, "opacity") - 0.5).abs() < 0.01);
    assert!((current(&runtime, "x") - 50.0).abs() < 0.01);
    runtime.tick_animations(Duration::from_millis(100)).unwrap();
    assert_eq!(current(&runtime, "opacity"), 1.0);
    assert_eq!(current(&runtime, "x"), 100.0);
}

#[test]
fn enter_names_a_property_the_node_has_and_a_cursor_is_a_shape() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local ui = require("morf.ui")
                ui.Rect { enter = { glow = 0 } }
            "##,
        )
        .unwrap_err();
    assert!(error.to_string().contains("glow"), "{error}");
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local ui = require("morf.ui")
                ui.MouseArea { cursor = "hand" }
            "##,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("`hand` is not a cursor shape"),
        "{error}"
    );
    runtime
        .execute(
            "good.lua",
            br##"
                local ui = require("morf.ui")
                ui.MouseArea { cursor = "pointer" }
            "##,
        )
        .unwrap();
    let node = *runtime.scene().roots().last().unwrap();
    assert_eq!(
        runtime.scene().string_value(node, "cursor").unwrap(),
        "pointer"
    );
}
