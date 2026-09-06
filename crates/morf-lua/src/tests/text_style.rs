use morf_scene::{Color, TextDecoration, Value};

use crate::*;

#[test]
fn text_takes_a_line_height_a_slant_a_width_and_a_decoration() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "text.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local refused = morf.signal("refused", false)
                local label = ui.Text {
                    text = "Password",
                    line_height = "24px",
                    letter_spacing = 1.5,
                    word_spacing = 2,
                    font_style = "italic",
                    font_stretch = "semi_condensed",
                    decoration = function()
                        return refused:get() and { line = "under", color = "#ff0000", thickness = 2 } or {}
                    end,
                }
                morf.ipc.refuse = function() refused:set(true) end
                morf.ipc.line = function() return label.decoration.line end
            "##,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().current(node, "line_height").unwrap(),
        &Value::String("24px".to_owned())
    );
    assert_eq!(
        runtime.scene().string_value(node, "font_style").unwrap(),
        "italic"
    );
    assert_eq!(
        TextDecoration::parse(runtime.scene().current(node, "decoration").unwrap()).unwrap(),
        None,
        "nothing refused yet"
    );
    runtime.call_ipc("refuse", &[]).unwrap();
    let logs: Vec<String> = runtime
        .take_logs()
        .into_iter()
        .map(|entry| format!("{entry:?}"))
        .collect();
    assert!(logs.is_empty(), "{logs:?}");
    let decoration = TextDecoration::parse(runtime.scene().current(node, "decoration").unwrap())
        .unwrap()
        .expect("refused, so underlined");
    assert_eq!(decoration.color, Some(Color::rgba8(255, 0, 0, 255)));
    assert_eq!(decoration.thickness, Some(2.0));
    let line = runtime.call_ipc("line", &[]).unwrap();
    assert_eq!(line, vec![IpcValue::String("under".to_owned())]);
}

#[test]
fn a_wrong_text_style_is_refused_where_it_is_written() {
    let mut runtime = Runtime::default();
    for (source, expected) in [
        (
            r#"ui.Text { text = "x", font_style = "slanted" }"#,
            "`slanted` is not normal, italic or oblique",
        ),
        (
            r#"ui.Text { text = "x", line_height = "tall" }"#,
            "a multiple of the font size or a `px` size",
        ),
        (
            r#"ui.Text { text = "x", decoration = { line = "beside" } }"#,
            "decoration line `beside` is not under, over or through",
        ),
        (
            r#"ui.Text { text = "x", font_stretch = "wide" }"#,
            "`wide` is not a width",
        ),
    ] {
        let error = runtime
            .execute(
                "bad.lua",
                format!("local ui = require(\"morf.ui\")\n{source}").as_bytes(),
            )
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{source}: {error}");
    }
}
