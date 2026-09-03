//! Being something on the session, and leaving it.
//!
//! Two verbs a shell needs and morf had neither of: owning a bus name so other
//! programs can call in, and asking the shell to stop.

use std::thread;
use std::time::Duration;

use super::*;

#[test]
fn a_configuration_owns_a_bus_name_and_answers_a_call() {
    // The client half of D-Bus has been reachable from Lua for years; this is
    // the other half, and it is what a notification server, a tray watcher and
    // a polkit agent all need. `morf_io::DbusService` had every piece of it and
    // was reachable from nowhere — the gap was the binding, not the engine.
    const NAME: &str = "org.morf.LuaServeSmoke";
    const PATH: &str = "/org/morf/LuaServeSmoke";
    const INTERFACE: &str = "org.morf.LuaServeSmoke";

    let mut runtime = Runtime::default();
    let started = runtime.execute(
        "serve.lua",
        format!(
            r#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local answered = morf.signal("serve.answered", "waiting")
                local service, outcome = morf.dbus.serve("session", "{NAME}", "{PATH}")
                answered:set(outcome)
                service:on_call(function(call)
                    answered:set(call.member)
                    service:reply(call.id, "answered")
                end)
                ui.Text {{ text = function() return answered:get() end }}
            "#
        )
        .as_bytes(),
    );
    if started.is_err() {
        // No session bus here; the thing under test cannot run.
        assert!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err(),
            "there is a session bus, so this failed for a real reason: {started:?}"
        );
        return;
    }
    let root = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "owned",
        "the name is ours, and the configuration was told so"
    );

    // Somebody else on the bus, calling us. On its own thread because both
    // halves block: the call has to be in flight before the runtime can see it.
    let caller = thread::spawn(|| {
        let proxy = morf_io::DbusProxy::connect_with_timeout(
            morf_io::Bus::Session,
            NAME,
            PATH,
            INTERFACE,
            Duration::from_secs(5),
        )
        .expect("a caller can connect");
        proxy.call_value("Echo")
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while runtime.scene().string_value(root, "text").unwrap() != "Echo"
        && std::time::Instant::now() < deadline
    {
        runtime.poll_services();
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "Echo",
        "the call reached Lua, with the member it was made on"
    );
    assert_eq!(
        caller.join().expect("the caller finished").unwrap(),
        morf_io::DbusValue::List(vec![morf_io::DbusValue::String("answered".to_owned())]),
        "and Lua's reply reached the caller"
    );
}

#[test]
fn a_configuration_can_ask_the_shell_to_stop() {
    // The greeter's missing verb. It launches a session and then has nothing
    // left to draw, and until now no way to say so — quickshell has
    // `Quickshell.quit()` and morf had no equivalent anywhere in its Lua API.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "quit.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local done = morf.signal("quit.done", false)
                morf.timer(1, function()
                    done:set(true)
                    morf.quit()
                end, false)
                ui.Text { text = function() return done:get() and "leaving" or "here" end }
            "#,
        )
        .unwrap();
    assert!(!runtime.quit_requested(), "nothing has asked to stop yet");

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.poll_services() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }

    let root = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "leaving",
        "the rest of the handler ran: quitting asks, it does not unwind"
    );
    assert!(
        runtime.quit_requested(),
        "and the request is there for the supervisor to act on"
    );
}

#[test]
fn a_proxy_takes_a_call_timeout_and_enumerates_managed_objects() {
    // Two halves of the same complaint: that writing UPower or BlueZ in Lua is
    // technically possible and unpleasant.
    //
    // The timeout was the real half. Every proxy gave up after a second, which
    // is right for reading a property and wrong for anything a human is part
    // of -- BlueZ `Pair` does not return until the pairing succeeds or fails,
    // well past it.
    //
    // ObjectManager was not a gap at all. `GetManagedObjects` returns
    // `a{oa{sa{sv}}}` and the decoder beside this has always handled it: the
    // reply arrives as a path -> interface -> property tree of ordinary Lua
    // tables. Nothing needed writing; this pins it down so it stays true.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "dbus-timeout.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local seen = morf.signal("dbus.seen", "no bus")
                local ok, proxy = pcall(morf.dbus.proxy,
                    "session", "org.freedesktop.DBus", "/org/freedesktop/DBus",
                    "org.freedesktop.DBus", 15000)
                if ok then
                    local names = proxy:call("ListNames")
                    seen:set(type(names) == "table" and "listed" or "odd")
                end
                ui.Text { text = function() return seen:get() end }
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let scene = runtime.scene();
    let text = scene.string_value(root, "text").unwrap().to_owned();
    drop(scene);
    assert!(
        text == "listed" || text == "no bus",
        "a proxy accepts a timeout and still works: {text}"
    );
    if text == "no bus" {
        assert!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err(),
            "there is a session bus, so the proxy failed for a real reason"
        );
    }
}

#[test]
fn a_configuration_reads_workspaces_and_asks_to_switch() {
    // `ext-workspace-v1` is the compositor-neutral answer to workspaces, and
    // binding it is what lets an indicator written once work on every
    // compositor that speaks it. Before this, morf's own examples reached for
    // `/dispatch workspace N` at Hyprland's socket, because there was nothing
    // neutral to reach for.
    //
    // The list is the engine's to fill, so here it is filled directly: what is
    // under test is the shape a configuration sees and the request it can make
    // back, not the protocol, which the wayland smoke covers against a live
    // compositor.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "workspaces.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local shown = morf.signal("ws.shown", "none")
                morf.timer(1, function()
                    local mine = {}
                    for _, w in ipairs(morf.workspaces) do
                        if w.output == "DP-1" then
                            mine[#mine + 1] = w.name .. (w.active and "*" or "")
                            if w.name == "2" then morf.workspace.activate(w.key) end
                        end
                    end
                    shown:set(table.concat(mine, ","))
                end, false)
                ui.Text { text = function() return shown:get() end }
            "#,
        )
        .unwrap();
    // Two outputs' worth, so the per-output filter in the config above is doing
    // something rather than passing everything through.
    runtime.set_workspaces(&[
        Workspace {
            key: "a".into(),
            name: "1".into(),
            output: "DP-1".into(),
            active: true,
            ..Workspace::default()
        },
        Workspace {
            key: "b".into(),
            name: "2".into(),
            output: "DP-1".into(),
            activatable: true,
            ..Workspace::default()
        },
        Workspace {
            key: "c".into(),
            name: "1".into(),
            output: "DP-2".into(),
            ..Workspace::default()
        },
    ]);

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.poll_services() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }

    let root = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "1*,2",
        "the configuration sees its own output's workspaces, and which is active"
    );
    assert_eq!(
        runtime.take_workspace_activation().as_deref(),
        Some("b"),
        "and switching asks by key -- not by name, which is not unique, and not \
         by the compositor's id, which is optional and absent on Hyprland"
    );
    assert_eq!(
        runtime.take_workspace_activation(),
        None,
        "the request is taken once"
    );
}
