use mold_io::{Bus, DbusProxy};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proxy = DbusProxy::connect(
        Bus::Session,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let id: String = proxy.call("GetId", &())?;
    let xml = proxy.introspect()?;
    if id.is_empty() || !xml.contains("org.freedesktop.DBus") {
        return Err("D-Bus reply was incomplete".into());
    }
    let signals = proxy.subscribe("NameOwnerChanged")?;
    let name = format!("org.mold.Smoke{}", std::process::id());
    let _: u32 = proxy.call("RequestName", &(name.as_str(), 0_u32))?;
    if signals.next(Duration::from_secs(2)).is_none() {
        return Err("D-Bus signal was not received".into());
    }
    let _: u32 = proxy.call("ReleaseName", &(name.as_str(),))?;
    println!("D-Bus {} introspection passed", id);
    Ok(())
}
