use morf_scene::Color;

use crate::*;

fn ipc_string(runtime: &mut Runtime, verb: &str) -> String {
    match runtime.call_ipc(verb, &[]).unwrap().as_slice() {
        [IpcValue::String(value)] => value.clone(),
        [IpcValue::Color(color)] => color.to_pastel().to_rgb_hex_string(true),
        other => panic!("{verb} answered {other:?}"),
    }
}

fn ipc_number(runtime: &mut Runtime, verb: &str) -> f64 {
    match runtime.call_ipc(verb, &[]).unwrap().as_slice() {
        [IpcValue::Number(value)] => *value,
        [IpcValue::Integer(value)] => *value as f64,
        other => panic!("{verb} answered {other:?}"),
    }
}

#[test]
fn a_colour_is_a_value_a_property_takes_and_gives_back() {
    // Written as a value, read back as the same value: fields, string
    // form, equality, and the round trip through a signal and a state.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "color.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local accent = morf.color "#3366cc"
                local tint = morf.signal("tint", accent:alpha(0.5))
                local model = morf.state { paper = morf.color.oklch(0.95, 0.02, 80) }
                local box = ui.Rect {
                    color = accent,
                    border_color = function() return tint:get() end,
                }
                morf.ipc.read = function() return tostring(box.color) end
                morf.ipc.alpha = function() return box.border_color.a end
                morf.ipc.hue = function() return math.floor(box.color.h + 0.5) end
                morf.ipc.paper = function() return model.paper end
                morf.ipc.same = function()
                    return box.color == morf.color.rgb(0x33, 0x66, 0xcc) and "same" or "different"
                end
                morf.ipc.retint = function() tint:set(morf.color "rebeccapurple") end
                morf.ipc.name = function() return accent:nearest_name() end
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().color_value(node, "color").unwrap(),
        Color::parse("#3366cc").unwrap()
    );
    assert_eq!(ipc_string(&mut runtime, "read"), "#3366cc");
    assert!((ipc_number(&mut runtime, "alpha") - 0.5).abs() < 1e-6);
    assert_eq!(ipc_number(&mut runtime, "hue"), 220.0);
    assert_eq!(ipc_string(&mut runtime, "same"), "same");
    assert_eq!(ipc_string(&mut runtime, "name"), "royalblue");
    let paper = runtime.call_ipc("paper", &[]).unwrap();
    assert!(matches!(paper.as_slice(), [IpcValue::Color(_)]));
    runtime.call_ipc("retint", &[]).unwrap();
    assert_eq!(
        runtime.scene().color_value(node, "border_color").unwrap(),
        Color::parse("rebeccapurple").unwrap()
    );
}

#[test]
fn every_pastel_operation_has_a_lua_counterpart() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "ops.lua",
            br##"
                local morf = require("morf")
                local c = morf.color "hsl(200, 60%, 40%)"
                local answers = {}
                local function put(name, value) answers[#answers + 1] = name .. "=" .. tostring(value) end
                put("hex", c:hex())
                put("rgb", c:rgb_string())
                put("hsl", c:hsl_string())
                put("oklch", c:distance(morf.color(c:oklch_string())) < 1)
                put("lighter", c:lighten(0.2))
                put("rotated", c:rotate(180) == c:complement())
                put("gray", c:gray():hsl().s)
                put("mixed", c:mix("white", 0.5, "srgb"))
                put("over", morf.color("#ff000080"):composite("white"))
                put("blind", c:blind("deuteranopia") ~= c)
                put("with", c:with { l = 0.9, space = "hsl" }:hsl().l)
                put("light", c:is_light())
                put("contrast", string.format("%.2f", c:contrast("white")))
                put("text", c:text_color())
                put("distance", c:distance("white", "cie76") > 0)
                put("scale", morf.color.scale { "black", "white" }:sample(0.5, "srgb"))
                put("samples", #morf.color.scale { "black", "white" }:samples(5))
                put("distinct", #morf.color.distinct(4, { fixed = { "red" }, order = true, iterations = 3000 }))
                put("random", morf.color.random("gray"):hsl().s)
                put("named", morf.color.named("Tomato"))
                put("names", morf.color.names().tomato == morf.color.named("tomato"))
                put("ansi", morf.color.ansi8(9):hex() .. ":" .. c:ansi8())
                put("paint", c:paint("x", { bold = true, mode = "8bit" }) ~= "x")
                put("table", morf.color { r = 255, g = 0, b = 0 })
                put("hwb", morf.color "hwb(0 0% 0%)")
                put("slash", morf.color "rgb(255 0 0 / 50%)")
                put("cmyk", morf.color.cmyk(0, 1, 1, 0))
                put("invert", morf.color("#ff0000"):invert())
                put("gray8", morf.color.gray(0.5):rgb8().r)
                morf.ipc.answers = function() return table.concat(answers, " ") end
            "##,
        )
        .unwrap();
    let answers = ipc_string(&mut runtime, "answers");
    let expect = |pair: &str| assert!(answers.contains(pair), "{answers} lacks {pair}");
    expect("hex=#297aa3");
    expect("rgb=rgb(41, 122, 163)");
    expect("hsl=hsl(200, 59.8%, 40.0%)");
    expect("oklch=true");
    expect("lighter=#5cadd6");
    expect("rotated=true");
    expect("gray=0.0");
    expect("mixed=#94bdd1");
    expect("over=#ff7e7e");
    expect("blind=true");
    expect("with=0.9");
    expect("light=false");
    expect("contrast=4.77");
    expect("text=#ffffff");
    expect("distance=true");
    expect("scale=#808080");
    expect("samples=5");
    expect("distinct=4");
    expect("random=0.0");
    expect("named=#ff6347");
    expect("names=true");
    expect("ansi=#ff0000:");
    expect("paint=true");
    expect("table=#ff0000");
    expect("hwb=#ff0000");
    expect("slash=#ff000080");
    expect("cmyk=#ff0000");
    expect("invert=#00ffff");
    expect("gray8=128");
}

#[test]
fn a_bad_colour_is_an_error_where_it_is_written() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local morf = require("morf")
                local c = morf.color "#\xc3\xa9\xc3\xa9"
            "##,
        )
        .unwrap_err();
    assert!(error.to_string().contains("is not a colour"), "{error}");
    let error = runtime
        .execute(
            "bad-space.lua",
            br##"
                local morf = require("morf")
                morf.color("red"):mix("blue", 0.5, "cielab")
            "##,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("unknown mixing space"),
        "{error}"
    );
}

#[test]
fn a_colour_behavior_names_its_space_and_direction() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "space.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                ui.Rect {
                    color = "red",
                    behavior = {
                        color = { duration = 1000, space = "oklch", hue = "longer" },
                    },
                }
            "##,
        )
        .unwrap();
    let error = runtime
        .execute(
            "bad-space.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                ui.Rect { color = "red", behavior = { color = { duration = 1, space = "hsl" } } }
            "##,
        )
        .unwrap_err();
    assert!(error.to_string().contains("space must be"), "{error}");
}

#[test]
fn linear_light_is_the_shader_side_of_a_colour() {
    // What a shader is handed: the sRGB curve taken off, so a data block
    // carries the numbers the GPU multiplies rather than the ones a
    // stylesheet writes.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "linear.lua",
            br##"
                local morf = require("morf")
                local linear = morf.color("#808080"):linear()
                morf.ipc.gray = function() return linear.r end
                morf.ipc.alpha = function() return morf.color("#ff000080"):linear().a end
            "##,
        )
        .unwrap();
    assert!((ipc_number(&mut runtime, "gray") - 0.2158).abs() < 0.001);
    assert!((ipc_number(&mut runtime, "alpha") - 0.5).abs() < 0.01);
}
