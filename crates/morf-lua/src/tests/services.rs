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

#[test]
fn a_configuration_acts_on_another_window() {
    // `ext-foreign-toplevel-list` is enumeration and nothing else -- no state,
    // no requests -- so a shell built on it alone can draw a task list and not
    // a task bar: every entry is a label nobody can click. The control half
    // comes from `wlr-foreign-toplevel-management`, and this is what a
    // configuration sees of it.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "toplevels.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local shown = morf.signal("tl.shown", "none")
                morf.timer(1, function()
                    local lines = {}
                    for _, w in ipairs(morf.windows) do
                        lines[#lines + 1] = w.app_id ..
                            (w.activated and "*" or "") ..
                            (w.controllable and "" or "?")
                        if w.app_id == "editor" then
                            morf.toplevel.activate(w.identifier)
                            morf.toplevel.set_maximized(w.identifier)
                            morf.toplevel.set_minimized(w.identifier, false)
                        end
                    end
                    shown:set(table.concat(lines, ","))
                end, false)
                ui.Text { text = function() return shown:get() end }
            "#,
        )
        .unwrap();
    runtime.set_windows(&[
        Toplevel {
            identifier: "one".into(),
            title: "notes".into(),
            app_id: "editor".into(),
            activated: true,
            controllable: true,
            ..Toplevel::default()
        },
        // A window the control protocol never matched: its state is unknown
        // rather than false, which is what `controllable` is for.
        Toplevel {
            identifier: "two".into(),
            title: "mail".into(),
            app_id: "mailer".into(),
            ..Toplevel::default()
        },
    ]);

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.poll_services() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }

    let root = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "editor*,mailer?",
        "focus and controllability both reach the configuration"
    );
    let requests = runtime.take_toplevel_requests();
    let described = requests
        .iter()
        .map(|request| {
            format!(
                "{}:{}={}",
                request.identifier, request.action, request.value
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        described,
        [
            "one:activate=true",
            // The setters default to on, so `set_maximized(id)` reads the way
            // it looks, and the explicit `false` still comes through.
            "one:set_maximized=true",
            "one:set_minimized=false",
        ],
        "and the requests come back in order, addressed by identifier"
    );
    assert!(runtime.take_toplevel_requests().is_empty(), "taken once");
}

/// A confdir with one service whose auth step asks for a password and checks
/// it against `hunter2` through `pam_exec` -- a real module, prompting through
/// the real conversation, without needing an account on the machine.
fn pam_service_dir(name: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("morf-lua-pam-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let check = dir.join("check.sh");
    std::fs::write(&check, "#!/bin/sh\ntest \"$(cat)\" = hunter2\n").unwrap();
    std::fs::set_permissions(&check, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.join(name),
        format!(
            "auth required pam_exec.so expose_authtok quiet {}\naccount required pam_permit.so\n",
            check.display()
        ),
    )
    .unwrap();
    dir
}

fn poll_until(runtime: &mut Runtime, root: morf_scene::NodeHandle, wanted: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        runtime.poll_services();
        let text = runtime
            .scene()
            .string_value(root, "text")
            .unwrap()
            .to_owned();
        if text.contains(wanted) || std::time::Instant::now() > deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_configuration_holds_a_pam_conversation() {
    // The fingerprint case, end to end from Lua: the module asks, the
    // configuration is shown the question, answers it, and is told the
    // verdict. Before this the engine answered every prompt itself from a
    // password handed in up front, so nothing a person had to take part in
    // could log anybody in.
    let dir = pam_service_dir("ask");
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pam-session.lua",
            format!(
                r#"
                    local morf = require("morf")
                    local ui = require("morf.ui")
                    local seen = morf.signal("pam.seen", "")
                    local session = morf.pam.session("ask", "nobody", "{}")
                    session:on_message(function(m)
                        seen:set(seen:get() .. m.kind .. ";")
                        if m.kind == "prompt" then
                            assert(m.echo == false, "a password prompt says it is secret")
                            assert(session:respond("hunter2"), "the prompt took the answer")
                            assert(not session:respond("again"), "and only one answer")
                        elseif m.kind == "finished" then
                            seen:set(seen:get() .. (m.ok and "ok" or ("err:" .. tostring(m.error))))
                        end
                    end)
                    ui.Text {{ text = function() return seen:get() end }}
                "#,
                dir.display()
            )
            .as_bytes(),
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = poll_until(&mut runtime, root, "finished");
    assert_eq!(
        text, "prompt;finished;ok",
        "the question reached Lua, the answer reached the module, and the verdict came back \
         (if this says libpam could not be loaded, re-enter the dev shell: the flake provides pam)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_configuration_can_give_up_on_a_pam_conversation() {
    // Nobody is going to touch the sensor. The configuration says so, and the
    // transaction ends with a verdict that says it was cancelled rather than
    // that "the conversation failed".
    let dir = pam_service_dir("cancel");
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pam-cancel.lua",
            format!(
                r#"
                    local morf = require("morf")
                    local ui = require("morf.ui")
                    local seen = morf.signal("pam.seen", "")
                    local session = morf.pam.session("cancel", "nobody", "{}")
                    session:on_message(function(m)
                        if m.kind == "prompt" then
                            session:cancel()
                            seen:set("cancelled;")
                        elseif m.kind == "finished" then
                            seen:set(seen:get() .. "finished:" .. tostring(m.code))
                        end
                    end)
                    ui.Text {{ text = function() return seen:get() end }}
                "#,
                dir.display()
            )
            .as_bytes(),
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    let text = poll_until(&mut runtime, root, "finished");
    assert_eq!(
        text, "cancelled;finished:-1",
        "and the code names the cancel"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
