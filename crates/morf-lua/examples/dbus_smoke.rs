use std::thread;
use std::time::Duration;

use morf_lua::{IpcValue, Runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::default();
    runtime.execute(
        "dbus-smoke.lua",
        br#"
            local morf = require("morf")
            local proxy = morf.dbus.proxy(
                "session",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus"
            )
            local id = proxy:call("GetId")
            assert(type(id) == "string" and #id > 0)
            local owner = proxy:call_with("GetNameOwner", "org.freedesktop.DBus")
            assert(type(owner) == "string" and #owner > 0)
            local signals = 0
            proxy:subscribe("NameOwnerChanged", function(value)
              if type(value) == "table" and #value == 3 then signals = signals + 1 end
            end)
            morf.ipc["test.dbus-signals"] = function() return signals end
            morf.dbus.proxy(
              "session",
              "org.freedesktop.DBus",
              "/org/freedesktop/DBus",
              "org.freedesktop.DBus"
            )
            assert(string.find(proxy:introspect(), "org.freedesktop.DBus", 1, true))
        "#,
    )?;
    let mut signals = 0;
    for _ in 0..50 {
        runtime.poll_services();
        if let [IpcValue::Integer(count)] = runtime.call_ipc("test.dbus-signals", &[])?[..] {
            signals = count;
        }
        if signals > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(signals > 0, "D-Bus signal callback did not run");
    println!("Lua D-Bus call and signal handling passed");
    Ok(())
}
