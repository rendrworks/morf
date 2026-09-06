use std::time::Duration;

use crate::*;

fn ipc_string(runtime: &mut Runtime, verb: &str) -> String {
    match runtime.call_ipc(verb, &[]).unwrap().as_slice() {
        [IpcValue::String(value)] => value.clone(),
        [IpcValue::Color(color)] => color.to_pastel().to_rgb_hex_string(true),
        [IpcValue::Number(value)] => value.to_string(),
        [IpcValue::Integer(value)] => value.to_string(),
        other => panic!("{verb} answered {other:?}"),
    }
}

#[test]
fn a_theme_holds_colours_and_derives_tokens_from_them() {
    // A colour-named string is a colour, a function is derived from the rest,
    // and a binding that reads a derived token follows the tokens it reads.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "theme.lua",
            br##"
                local morf = require("morf")
                local ui = require("morf.ui")
                local theme = morf.theme {
                    accent = "#3366cc",
                    family = "Inter",
                    hover = function(t) return t.accent:alpha(0.5) end,
                    label = function(t) return t.family .. " on " .. tostring(t.accent) end,
                }
                local box = ui.Rect { color = function() return theme.hover end }
                morf.ipc.hover = function() return theme.hover end
                morf.ipc.label = function() return theme.label end
                morf.ipc.family = function() return theme.family end
                morf.ipc.retint = function() theme.accent = "#ff0000" end
            "##,
        )
        .unwrap();
    assert_eq!(ipc_string(&mut runtime, "hover"), "#3366cc80");
    assert_eq!(ipc_string(&mut runtime, "label"), "Inter on #3366cc");
    assert_eq!(ipc_string(&mut runtime, "family"), "Inter");
    let node = runtime.scene().roots()[0];
    let hex = |runtime: &Runtime| {
        runtime
            .scene()
            .color_value(node, "color")
            .unwrap()
            .to_pastel()
            .to_rgb_hex_string(true)
    };
    assert_eq!(hex(&runtime), "#3366cc80");
    runtime.call_ipc("retint", &[]).unwrap();
    assert_eq!(
        hex(&runtime),
        "#ff000080",
        "the binding re-derived hover from the new accent"
    );
}

#[test]
fn a_theme_reads_its_tokens_from_a_json_file_and_follows_rewrites() {
    let directory = std::env::temp_dir().join(format!("morf-theme-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("colors.json");
    std::fs::write(
        &path,
        br##"{"special": {"background": "#101010"}, "colors": {"color1": "#aa0000", "color2": "#00aa00"}, "alpha": "100"}"##,
    )
    .unwrap();
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "wal.lua",
            format!(
                r##"
                local morf = require("morf")
                local theme = morf.theme({{ color1 = "#ffffff", color9 = "#123456" }}, {{ source = "{}" }})
                morf.ipc.one = function() return theme.color1 end
                morf.ipc.two = function() return theme.color2 end
                morf.ipc.nine = function() return theme.color9 end
                morf.ipc.background = function() return theme.background end
                morf.ipc.alpha = function() return theme.alpha end
                "##,
                path.display()
            )
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        ipc_string(&mut runtime, "one"),
        "#aa0000",
        "the file wins over the seed"
    );
    assert_eq!(
        ipc_string(&mut runtime, "two"),
        "#00aa00",
        "a nested leaf is a token"
    );
    assert_eq!(
        ipc_string(&mut runtime, "nine"),
        "#123456",
        "the seed fills what the file lacks"
    );
    assert_eq!(ipc_string(&mut runtime, "background"), "#101010");
    assert_eq!(
        ipc_string(&mut runtime, "alpha"),
        "100",
        "a string that is not a colour stays one"
    );

    // Rewritten the way a palette generator does it: a new file moved over.
    let staged = directory.join("colors.json.new");
    std::fs::write(&staged, br##"{"colors": {"color1": "#0000aa"}}"##).unwrap();
    std::fs::rename(&staged, &path).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        runtime.poll_services();
        if ipc_string(&mut runtime, "one") == "#0000aa" || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        ipc_string(&mut runtime, "one"),
        "#0000aa",
        "the token followed the rewrite"
    );
    assert_eq!(
        ipc_string(&mut runtime, "two"),
        "#00aa00",
        "an absent key keeps its value"
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_theme_rejects_what_it_cannot_derive_from() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute(
            "bad.lua",
            br##"
                local morf = require("morf")
                morf.theme({ accent = "#fff" }, { source = 3 })
            "##,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("theme `source` is a path"),
        "{error}"
    );
}
