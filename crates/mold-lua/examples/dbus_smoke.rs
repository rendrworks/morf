use mold_lua::Runtime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::default();
    runtime.execute(
        "dbus-smoke.lua",
        br#"
            local mold = require("mold")
            local proxy = mold.dbus.proxy(
                "session",
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus"
            )
            local id = proxy:call("GetId")
            assert(type(id) == "string" and #id > 0)
            local owner = proxy:call_with("GetNameOwner", "org.freedesktop.DBus")
            assert(type(owner) == "string" and #owner > 0)
            assert(string.find(proxy:introspect(), "org.freedesktop.DBus", 1, true))
            local battery = require("patin.services.upower").new()
            assert(type(battery:percentage()) == "number")
            local network = require("patin.services.network").new()
            assert(type(network:state()) == "number")
        "#,
    )?;
    println!("Lua D-Bus, UPower, and NetworkManager passed");
    Ok(())
}
