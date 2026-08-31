use crate::*;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;

use super::*;

#[test]
fn xkb_facade_builds_osk_layout_tables() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "xkb.lua",
            br#"
                local keymap = morf.xkb.compile { layout = "us" }
                assert(string.find(keymap.source, "xkb_keymap", 1, true))
                local found = false
                for _, key in ipairs(keymap.keys) do
                    if key.name == "AC01" then
                        assert(key.evdev_code == 30)
                        assert(key.layouts[1][1][1].text == "a")
                        assert(key.layouts[1][2][1].text == "A")
                        found = true
                    end
                end
                assert(found)
            "#,
        )
        .unwrap();
}

#[test]
fn input_method_bridges_context_and_edits() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "input-method.lua",
            br#"
                local active = morf.signal("input.active", false)
                morf.input_method.subscribe(function(value) active:set(value) end)
                morf.input_method.preedit("hel", 3, 3)
                morf.input_method.commit("hello")
                morf.input_method.delete(1, 2)
                morf.ipc["input.active"] = function() return active:get() end
            "#,
        )
        .unwrap();

    assert!(runtime.take_input_method_enable_request());
    assert_eq!(
        runtime.take_input_method_requests(),
        [
            InputMethodRequest::Preedit {
                text: "hel".to_owned(),
                begin: 3,
                end: 3,
            },
            InputMethodRequest::Commit("hello".to_owned()),
            InputMethodRequest::Delete {
                before: 1,
                after: 2,
            },
        ]
    );
    assert!(runtime.dispatch_input_method(true, Some("hello".to_owned()), 5, 5, 1));
    assert_eq!(
        runtime.call_ipc("input.active", &[]).unwrap(),
        [IpcValue::Boolean(true)]
    );
}

#[test]
fn text_input_bridges_state_and_edit_batches() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "text-input.lua",
            br#"
                local committed = morf.signal("text.committed", "")
                morf.text_input.subscribe(function(_, _, _, _, text)
                    if text then committed:set(text) end
                end)
                morf.text_input.surrounding("draft", 5, 5)
                morf.text_input.content_type(3, 0)
                morf.text_input.cursor_rect(10, 20, 2, 18)
                morf.ipc["text.get"] = function() return committed:get() end
            "#,
        )
        .unwrap();

    assert!(runtime.take_text_input_enable_request());
    assert_eq!(runtime.take_text_input_requests().len(), 3);
    assert!(runtime.dispatch_text_input(true, None, 0, 0, Some("done".to_owned()), 0, 0, 1,));
    assert_eq!(
        runtime.call_ipc("text.get", &[]).unwrap(),
        [IpcValue::String("done".to_owned())]
    );
}

#[test]
fn variants_builds_the_current_screen_instance() {
    let mut runtime = Runtime::for_screen(
        Limits::default(),
        Screen {
            id: 12,
            name: "DP-1".to_owned(),
            make: "Example".to_owned(),
            model: "Panel".to_owned(),
            description: Some("Example Panel".to_owned()),
            position: Some((10, 20)),
            width: Some(1920),
            height: Some(1080),
            physical_size: Some((600, 340)),
            scale: 2,
            transform: "normal".to_owned(),
        },
    );
    runtime
        .execute(
            "variants.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local instances = morf.variants(morf.screens, function(screen)
                    assert(screen.id == 12)
                    assert(screen.make == "Example")
                    assert(screen.model == "Panel")
                    assert(screen.description == "Example Panel")
                    assert(screen.x == 10 and screen.y == 20)
                    assert(screen.physical_width_mm == 600)
                    assert(screen.physical_height_mm == 340)
                    assert(screen.device_pixel_ratio == 2)
                    assert(screen.physical_pixel_density > 160)
                    assert(screen.logical_pixel_density > 80)
                    assert(screen.orientation == "landscape")
                    assert(screen.primary_orientation == "landscape")
                    assert(screen.transform == "normal")
                    return ui.Text { text = screen.name, width = screen.width }
                end)
                assert(#instances == 1)
            "#,
        )
        .unwrap();
    let node = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().string_value(node, "text").unwrap(), "DP-1");
    assert_eq!(runtime.scene().number(node, "width").unwrap(), 1920.0);
}

#[test]
fn variants_builds_every_model_instance() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "variants-all.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local instances = morf.variants({ "a", "b", "c" }, function(value)
                    return ui.Text { text = value }
                end)
                assert(#instances == 3)
            "#,
        )
        .unwrap();
    let scene = runtime.scene();
    assert_eq!(scene.roots().len(), 3);
    assert_eq!(scene.string_value(scene.roots()[2], "text").unwrap(), "c");
}

#[test]
fn lua_greetd_client_handles_authentication_prompts() {
    let path = std::env::temp_dir().join(format!("morf-greetd-{}.sock", std::process::id()));
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut length = [0_u8; 4];
        stream.read_exact(&mut length).unwrap();
        let mut request = vec![0_u8; u32::from_ne_bytes(length) as usize];
        stream.read_exact(&mut request).unwrap();
        assert!(
            String::from_utf8(request)
                .unwrap()
                .contains("create_session")
        );
        let response =
            br#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#;
        stream
            .write_all(&(response.len() as u32).to_ne_bytes())
            .unwrap();
        stream.write_all(response).unwrap();
    });
    let mut runtime = Runtime::default();
    let source = format!(
        r#"
            local client = morf.greetd.connect({:?})
            local response = client:create_session("morf")
            assert(response.type == "auth_message")
            assert(response.auth_message_type == "secret")
            assert(response.auth_message == "Password:")
        "#,
        path.to_string_lossy()
    );

    runtime.execute("greetd.lua", source.as_bytes()).unwrap();
    server.join().unwrap();
    fs::remove_file(path).unwrap();
}

#[test]
fn reports_syntax_errors_with_the_source_name() {
    let mut runtime = Runtime::default();
    let error = runtime.execute("broken.lua", b"local =").unwrap_err();
    assert!(matches!(error, Error::Load(_)));
    assert!(error.to_string().contains("broken.lua"));
}

#[test]
fn stops_an_infinite_loop_on_fuel_exhaustion() {
    let limits = Limits {
        fuel: 2_000,
        slice_fuel: 128,
        ..Limits::default()
    };
    let mut runtime = Runtime::new(limits);
    let error = runtime
        .execute("loop.lua", b"while true do end")
        .unwrap_err();
    assert_eq!(error, Error::FuelExhausted { budget: 2_000 });
}

#[test]
fn lua_signal_change_reruns_exactly_one_effect() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "reactive.lua",
            br#"
                local morf = require("morf")
                local source = morf.signal("source", 1)
                local other = morf.signal("other", 2)
                local source_runs = 0
                local other_runs = 0
                assert(morf.effect("source effect", function()
                    source:get()
                    source_runs = source_runs + 1
                end))
                assert(morf.effect("other effect", function()
                    other:get()
                    other_runs = other_runs + 1
                end))
                source_runs = 0
                other_runs = 0
                local ok, err = source:set(7)
                assert(ok, err)
                assert(source_runs == 1)
                assert(other_runs == 0)
            "#,
        )
        .unwrap();
}

#[test]
fn binding_dependencies_ignore_settled_signals() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "dependencies.lua",
            br#"
                local morf = require("morf")
                local source = morf.signal("source", 1)
                assert(morf.effect("source binding", function()
                    source:get()
                end))
            "#,
        )
        .unwrap();

    assert!(runtime.binding_dependencies().is_empty());
}

#[test]
fn binding_dependencies_flag_animated_property_reads() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "animated-dependency.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local width = morf.signal("width", 0)
                local source = ui.Rect {
                    behavior = {
                        width = { duration = 200, easing = "linear" },
                    },
                    width = function() return width:get() end,
                }
                ui.Item {
                    height = function() return source.width end,
                    x = function() return source.width_target end,
                }
                local ok, err = width:set(100)
                assert(ok, err)
            "#,
        )
        .unwrap();

    let diagnostics = runtime.binding_dependencies();
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains(".height <- "));
    assert!(diagnostics[0].contains("current animation values do not trigger Lua"));
    let runs = runtime.effect_runs();

    runtime.tick_animations(Duration::from_millis(100)).unwrap();

    assert_eq!(runtime.effect_runs(), runs);
    let item = runtime.scene().roots()[1];
    assert_eq!(runtime.scene().number(item, "height").unwrap(), 0.0);
    assert_eq!(runtime.scene().number(item, "x").unwrap(), 100.0);
}

#[test]
fn lua_binding_loop_names_the_property_chain() {
    let mut runtime = Runtime::new(Limits {
        effect_fuel: 10_000,
        frame_fuel: 100_000,
        ..Limits::default()
    });
    runtime
        .execute(
            "loop.lua",
            br#"
                local morf = require("morf")
                local left = morf.signal("left", 0)
                local right = morf.signal("right", 0)
                assert(morf.effect("left binding", function()
                    left:set(right:get() + 1)
                end))
                local ok, err = morf.effect("right binding", function()
                    right:set(left:get() + 1)
                end)
                assert(not ok, "loop unexpectedly succeeded")
                assert(string.find(err, "left binding", 1, true), err)
                assert(string.find(err, "right binding", 1, true), err)
                assert(string.find(err, "left", 1, true), err)
                assert(string.find(err, "right", 1, true), err)
            "#,
        )
        .unwrap();
}
