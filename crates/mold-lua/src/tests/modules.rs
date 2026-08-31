use crate::*;
use std::fs;

use super::*;

#[test]
fn downstream_modules_are_not_embedded() {
    let mut runtime = Runtime::default();
    let error = runtime
        .execute("downstream.lua", b"require('consumer.widgets.button')")
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("module `consumer.widgets.button` is not available")
    );
}

#[test]
fn runtimepath_loads_user_modules_without_rust_registration() {
    let root = std::env::temp_dir().join(format!("mold-runtime-{}", std::process::id()));
    let module = root.join("lua/user/widget.lua");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"return { answer = 42 }").unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    runtime
        .execute(
            &shell.to_string_lossy(),
            b"local widget = require('user.widget'); assert(widget.answer == 42)",
        )
        .unwrap();

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn require_caches_user_modules_in_package_loaded() {
    let root = std::env::temp_dir().join(format!("mold-require-{}", std::process::id()));
    let module = root.join("lua/user/once.lua");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(
        &module,
        b"module_runs = (module_runs or 0) + 1; return { runs = module_runs }",
    )
    .unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    runtime
        .execute(
            &shell.to_string_lossy(),
            br#"
                local first = require("user.once")
                local second = require("user.once")
                assert(first == second)
                assert(second.runs == 1)
                assert(package.loaded["user.once"] == first)
            "#,
        )
        .unwrap();

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ipc_registry_calls_named_bounded_handlers() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "ipc.lua",
            br#"
                mold.ipc["launcher.toggle"] = function(name, count)
                    return "hello " .. name, count + 1, true
                end
            "#,
        )
        .unwrap();

    assert_eq!(runtime.ipc_verbs(), ["launcher.toggle"]);
    assert_eq!(
        runtime
            .call_ipc(
                "launcher.toggle",
                &[IpcValue::String("mold".into()), IpcValue::Integer(2)],
            )
            .unwrap(),
        [
            IpcValue::String("hello mold".into()),
            IpcValue::Integer(3),
            IpcValue::Boolean(true),
        ]
    );
    assert!(runtime.call_ipc("missing", &[]).is_err());
}

#[test]
fn ipc_handlers_are_fuel_bounded() {
    let mut runtime = Runtime::new(Limits {
        effect_fuel: 256,
        ..Limits::default()
    });
    runtime
        .execute(
            "ipc-fuel.lua",
            b"mold.ipc.loop = function() while true do end end",
        )
        .unwrap();

    let error = runtime.call_ipc("loop", &[]).unwrap_err().to_string();
    assert!(error.contains("IPC handler fuel exhausted"), "{error}");
}

#[test]
fn list_handlers_preserve_order_and_isolate_failures() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "handlers.lua",
            br#"
                local calls = ""
                mold.idle.subscribe(1000, function()
                  calls = calls .. "a"
                  error("broken")
                end)
                mold.idle.subscribe(1000, function()
                  calls = calls .. "b"
                end)
                mold.ipc["calls"] = function() return calls end
            "#,
        )
        .unwrap();

    assert!(runtime.dispatch_idle(1000, true));
    assert_eq!(
        runtime.call_ipc("calls", &[]).unwrap(),
        [IpcValue::String("ab".into())]
    );
    assert!(runtime.take_logs()[0].contains("broken"));
}

#[test]
fn reloadable_signals_carry_state_into_a_new_runtime() {
    let source = br#"
        local visible = mold.reloadable("launcher.visible", false)
        mold.ipc["state.set"] = function(value) visible:set(value) end
        mold.ipc["state.get"] = function() return visible:get() end
    "#;
    let mut first = Runtime::default();
    first.execute("reloadable.lua", source).unwrap();
    first
        .call_ipc("state.set", &[IpcValue::Boolean(true)])
        .unwrap();

    let mut second = Runtime::default();
    second.restore_reloadable_state(first.reloadable_state());
    second.execute("reloadable.lua", source).unwrap();

    assert_eq!(
        second.call_ipc("state.get", &[]).unwrap(),
        [IpcValue::Boolean(true)]
    );
}

#[test]
fn lua_reload_requests_are_coalesced_and_consumed() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "reload-request.lua",
            br#"
                local core = require("mold.core")
                core.reload(false)
                core.reload(true)
            "#,
        )
        .unwrap();

    assert_eq!(runtime.take_reload_request(), Some(true));
    assert_eq!(runtime.take_reload_request(), None);
}

#[test]
fn persistent_properties_reload_as_one_typed_scope() {
    let source = br#"
        local state = mold.persistent("launcher", { visible = false, page = 1 })
        mold.ipc["state.set"] = function()
            state.visible = true
            state.page = 4
        end
        mold.ipc["state.get"] = function()
            return state.visible, state.page, state.loaded, state.reloaded
        end
    "#;
    let mut first = Runtime::default();
    first.execute("persistent.lua", source).unwrap();
    assert_eq!(
        first.call_ipc("state.get", &[]).unwrap(),
        [
            IpcValue::Boolean(false),
            IpcValue::Integer(1),
            IpcValue::Boolean(true),
            IpcValue::Boolean(false),
        ]
    );
    first.call_ipc("state.set", &[]).unwrap();

    let mut second = Runtime::default();
    second.restore_reloadable_state(first.reloadable_state());
    second.execute("persistent.lua", source).unwrap();
    assert_eq!(
        second.call_ipc("state.get", &[]).unwrap(),
        [
            IpcValue::Boolean(true),
            IpcValue::Integer(4),
            IpcValue::Boolean(true),
            IpcValue::Boolean(true),
        ]
    );
}

#[test]
fn reload_scopes_isolate_repeated_local_ids() {
    let source = br#"
        local left = mold.scope("screen.left")
        local right = mold.scope("screen.right")
        local left_open = left:reloadable("open", false)
        local right_open = right:reloadable("open", true)
        local state = left:persistent("panel", { page = 2 })
        mold.ipc["scope.get"] = function()
            return left_open:get(), right_open:get(), state.page,
                left:id("open"), right:id("open")
        end
    "#;
    let mut runtime = Runtime::default();
    runtime.execute("scopes.lua", source).unwrap();
    assert_eq!(
        runtime.call_ipc("scope.get", &[]).unwrap(),
        [
            IpcValue::Boolean(false),
            IpcValue::Boolean(true),
            IpcValue::Integer(2),
            IpcValue::String("screen.left.open".into()),
            IpcValue::String("screen.right.open".into()),
        ]
    );
    assert_eq!(
        runtime
            .reloadable_state()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        [
            "screen.left.open".to_owned(),
            "screen.left.panel.page".to_owned(),
            "screen.right.open".to_owned(),
        ]
    );
}

#[test]
fn idle_callbacks_receive_compositor_state() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "idle.lua",
            br#"
                local idle = mold.signal("idle", false)
                mold.idle.subscribe(30000, function(value) idle:set(value) end)
                mold.ipc["idle.get"] = function() return idle:get() end
            "#,
        )
        .unwrap();

    assert_eq!(runtime.idle_timeouts(), [30_000]);
    assert!(runtime.dispatch_idle(30_000, true));
    assert_eq!(
        runtime.call_ipc("idle.get", &[]).unwrap(),
        [IpcValue::Boolean(true)]
    );
}

#[test]
fn output_power_requests_are_bounded_and_ordered() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "power.lua",
            br#"
                mold.output_power.set("off")
                mold.output_power.set("on")
            "#,
        )
        .unwrap();

    assert_eq!(runtime.take_output_power_requests(), [false, true]);
    assert!(runtime.take_output_power_requests().is_empty());
}

#[test]
fn clipboard_bridges_publications_and_selections() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "clipboard.lua",
            br#"
                local current = mold.signal("clipboard", "")
                mold.clipboard.subscribe(function(text) current:set(text or "none") end)
                mold.clipboard.set("copied")
                mold.ipc["clipboard.get"] = function() return current:get() end
            "#,
        )
        .unwrap();

    assert_eq!(runtime.take_clipboard_requests(), ["copied"]);
    assert!(runtime.dispatch_clipboard(Some("pasted".to_owned())));
    assert_eq!(
        runtime.call_ipc("clipboard.get", &[]).unwrap(),
        [IpcValue::String("pasted".to_owned())]
    );
    assert!(runtime.dispatch_clipboard(None));
    assert_eq!(
        runtime.call_ipc("clipboard.get", &[]).unwrap(),
        [IpcValue::String("none".to_owned())]
    );
}

#[test]
fn screencopy_bridges_bounded_requests_and_pixels() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "screencopy.lua",
            br#"
                local result = mold.signal("capture", "pending")
                mold.screencopy.capture(true, function(frame, err)
                    if err then
                        result:set(err)
                    else
                        result:set(frame.format .. ":" .. frame.width .. ":" ..
                            #frame.pixels .. ":" .. string.byte(frame.pixels, 1))
                    end
                end)
                local second = mold.signal("second", "pending")
                mold.screencopy.capture(false, function(_, err) second:set(err) end)
                mold.ipc["capture.get"] = function() return result:get() end
                mold.ipc["second.get"] = function() return second:get() end
            "#,
        )
        .unwrap();

    assert_eq!(
        runtime.take_screencopy_requests(),
        [
            ScreencopyRequest {
                id: 0,
                include_cursor: true,
            },
            ScreencopyRequest {
                id: 1,
                include_cursor: false,
            },
        ]
    );
    assert!(runtime.dispatch_screencopy(1, Err("second failed".to_owned())));
    assert!(runtime.dispatch_screencopy(
        0,
        Ok(Screencopy {
            width: 2,
            height: 1,
            stride: 8,
            format: "argb8888".to_owned(),
            y_invert: false,
            pixels: vec![7; 8],
        })
    ));
    assert_eq!(
        runtime.call_ipc("capture.get", &[]).unwrap(),
        [IpcValue::String("argb8888:2:8:7".to_owned())]
    );
    assert_eq!(
        runtime.call_ipc("second.get", &[]).unwrap(),
        [IpcValue::String("second failed".to_owned())]
    );
}

#[test]
fn virtual_keyboard_requests_preserve_protocol_order() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "keyboard.lua",
            br#"
                mold.virtual_keyboard.modifiers(1, 2, 4, 0)
                mold.virtual_keyboard.key(30, true)
                mold.virtual_keyboard.key(30, false)
            "#,
        )
        .unwrap();

    assert_eq!(
        runtime.take_virtual_keyboard_requests(),
        [
            VirtualKeyboardRequest::Modifiers {
                depressed: 1,
                latched: 2,
                locked: 4,
                group: 0,
            },
            VirtualKeyboardRequest::Key {
                keycode: 30,
                pressed: true,
            },
            VirtualKeyboardRequest::Key {
                keycode: 30,
                pressed: false,
            },
        ]
    );
}
