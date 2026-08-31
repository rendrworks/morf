use morf_io::{Bus, DbusProxy, DbusValue};
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
    let name = format!("org.morf.Smoke{}", std::process::id());
    let reply = proxy.call_value_with(
        "RequestName",
        &DbusValue::List(vec![
            DbusValue::String(name.clone()),
            DbusValue::Typed {
                signature: "u".to_owned(),
                value: Box::new(DbusValue::Unsigned(0)),
            },
        ]),
    )?;
    if !matches!(reply, DbusValue::Unsigned(1 | 4)) {
        return Err("dynamic D-Bus arguments returned an invalid result".into());
    }
    if signals.next(Duration::from_secs(2)).is_none() {
        return Err("D-Bus signal was not received".into());
    }
    let _: u32 = proxy.call("ReleaseName", &(name.as_str(),))?;
    println!("D-Bus {} introspection passed", id);
    Ok(())
}
