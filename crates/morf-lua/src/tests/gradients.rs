use std::time::Duration;

use morf_scene::{Color, ColorSpace, Gradient, GradientKind};

use crate::*;

fn ipc_string(runtime: &mut Runtime, verb: &str) -> String {
    match runtime.call_ipc(verb, &[]).unwrap().as_slice() {
        [IpcValue::String(value)] => value.clone(),
        [IpcValue::Integer(value)] => value.to_string(),
        other => panic!("{verb} answered {other:?}"),
    }
}

#[test]
fn a_gradient_is_one_property_holding_its_stops() {
    // Written as one table of stops, read back with every default filled in,
    // and moved stop by stop when the property has a behavior.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "gradient.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local accent = morf.color "#3366cc"
                local box = ui.Rect {
                    width = 100,
                    height = 40,
                    gradient = {
                        angle = 90,
                        space = "oklch",
                        stops = { accent, { "#ffffff", 0.25 }, "transparent" },
                    },
                    behavior = { gradient = { duration = 100, easing = "linear", space = "srgb" } },
                }
                morf.ipc.count = function() return #box.gradient.stops end
                morf.ipc.second = function()
                    local stop = box.gradient.stops[2]
                    return tostring(stop.color) .. "@" .. stop.position
                end
                morf.ipc.kind = function() return box.gradient.kind end
                morf.ipc.retarget = function()
                    box.gradient = {
                        angle = 90,
                        space = "oklch",
                        stops = { "#000000", { "#000000", 0.75 }, "transparent" },
                    }
                end
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    let read = |runtime: &Runtime| {
        Gradient::parse(runtime.scene().current(node, "gradient").unwrap())
            .unwrap()
            .unwrap()
    };
    let gradient = read(&runtime);
    assert_eq!(gradient.kind, GradientKind::Linear);
    assert_eq!(gradient.angle, 90.0);
    assert_eq!(gradient.space, ColorSpace::Oklch);
    let positions: Vec<f64> = gradient.stops.iter().map(|stop| stop.position).collect();
    assert_eq!(positions, vec![0.0, 0.25, 1.0]);
    assert_eq!(gradient.stops[0].color, Color::parse("#3366cc").unwrap());
    assert_eq!(gradient.stops[2].color.alpha, 0.0);
    assert_eq!(ipc_string(&mut runtime, "count"), "3");
    assert_eq!(ipc_string(&mut runtime, "second"), "#ffffff@0.25");
    assert_eq!(ipc_string(&mut runtime, "kind"), "linear");

    runtime.call_ipc("retarget", &[]).unwrap();
    runtime.tick_animations(Duration::from_millis(50)).unwrap();
    let halfway = read(&runtime);
    assert_eq!(halfway.stops[1].position, 0.5);
    assert!(
        (halfway.stops[1].color.red - 0.5).abs() < 0.01,
        "the middle stop is halfway from white to black: {:?}",
        halfway.stops[1].color
    );
}

#[test]
fn a_bad_gradient_is_an_error_where_it_is_written() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local ui = require("morf.ui")
                ui.Rect { gradient = { stops = { "#ffffff" } } }
            "##,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a gradient needs at least two stops"),
        "{error}"
    );
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local ui = require("morf.ui")
                ui.Sdf { gradient = { kind = "swirl", stops = { "#fff", "#000" } } }
            "##,
        )
        .unwrap_err();
    assert!(error.to_string().contains("swirl"), "{error}");
}
