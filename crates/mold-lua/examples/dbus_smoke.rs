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
            assert(string.find(proxy:introspect(), "org.freedesktop.DBus", 1, true))
        "#,
    )?;
    println!("Lua D-Bus call and introspection passed");
    Ok(())
}
