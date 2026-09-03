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
