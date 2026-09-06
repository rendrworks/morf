use std::time::Duration;

use morf_scene::{Color, Value};

use crate::*;

fn ipc_string(runtime: &mut Runtime, verb: &str) -> String {
    match runtime.call_ipc(verb, &[]).unwrap().as_slice() {
        [IpcValue::String(value)] => value.clone(),
        [IpcValue::Boolean(value)] => value.to_string(),
        [IpcValue::Integer(value)] => value.to_string(),
        [IpcValue::Number(value)] => value.to_string(),
        [IpcValue::Nil] => "nil".to_owned(),
        [IpcValue::Color(color)] => color.to_pastel().to_rgb_hex_string(true),
        other => panic!("{verb} answered {other:?}"),
    }
}

#[test]
fn preferences_are_fields_a_binding_follows() {
    // Whatever the desktop says, the fields are there with the right shape,
    // and a change reaches every binding that read one.
    let screen = Screen {
        scale: 2,
        ..Screen::default()
    };
    let mut runtime = Runtime::for_screen(Limits::default(), screen);
    runtime
        .execute(
            "prefers.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local prefers = morf.prefers
                assert(type(prefers.color_scheme) == "string")
                assert(type(prefers.contrast) == "string")
                assert(type(prefers.reduced_motion) == "boolean")
                assert(prefers.accent_color == nil or prefers.accent_color.r ~= nil)
                local box = ui.Rect {
                    color = function()
                        return prefers.color_scheme == "dark" and "#000000" or "#ffffff"
                    end,
                }
                morf.ipc.scheme = function() return prefers.color_scheme end
                morf.ipc.scale = function() return prefers.scale end
                morf.ipc.accent = function() return prefers.accent_color end
            "##,
        )
        .unwrap();
    assert_eq!(ipc_string(&mut runtime, "scale"), "2");
    let node = runtime.scene().roots()[0];
    runtime
        .set_preference("color_scheme", IpcValue::String("dark".to_owned()))
        .unwrap();
    assert_eq!(ipc_string(&mut runtime, "scheme"), "dark");
    assert_eq!(
        runtime.scene().color_value(node, "color").unwrap(),
        Color::rgba8(0, 0, 0, 255)
    );
    runtime
        .set_preference("color_scheme", IpcValue::String("light".to_owned()))
        .unwrap();
    assert_eq!(
        runtime.scene().color_value(node, "color").unwrap(),
        Color::rgba8(255, 255, 255, 255)
    );
    runtime
        .set_preference("accent_color", IpcValue::Color(Color::rgba8(1, 2, 3, 255)))
        .unwrap();
    assert_eq!(ipc_string(&mut runtime, "accent"), "#010203");
    assert_eq!(
        runtime.set_preference("font", IpcValue::Nil).unwrap_err(),
        "`font` is not a preference"
    );
}

#[test]
fn reduced_motion_lands_every_motion_on_its_target_at_once() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "motion.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local box = ui.Rect {
                    width = 0,
                    x = 0,
                    behavior = {
                        width = { duration = 400, easing = "linear" },
                        x = { kind = "spring", stiffness = 120, damping = 14 },
                    },
                }
                morf.ipc.go = function()
                    box.width = 100
                    box.x = 50
                end
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    runtime
        .set_preference("reduced_motion", IpcValue::Boolean(true))
        .unwrap();
    runtime.call_ipc("go", &[]).unwrap();
    let frame = runtime.tick_animations(Duration::from_millis(1)).unwrap();
    assert_eq!(
        runtime.scene().current(node, "width").unwrap(),
        &Value::Number(100.0)
    );
    assert_eq!(
        runtime.scene().current(node, "x").unwrap(),
        &Value::Number(50.0)
    );
    assert!(!frame.active, "nothing is left moving");

    // Motion is back the moment the preference is.
    runtime
        .set_preference("reduced_motion", IpcValue::Boolean(false))
        .unwrap();
    runtime.scene_mut().assign(node, "width", 0.0).unwrap();
    runtime.tick_animations(Duration::from_millis(200)).unwrap();
    assert_eq!(
        runtime.scene().current(node, "width").unwrap(),
        &Value::Number(50.0)
    );
}
