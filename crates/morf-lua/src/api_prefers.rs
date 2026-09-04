//! `morf.prefers`: what the person asked their desktop for.
//!
//! A state with five fields — `color_scheme`, `contrast`, `reduced_motion`,
//! `accent_color` and `scale` — read from the settings portal over D-Bus and
//! kept current from its change signal, so a binding that reads one follows
//! the desktop's setting. Without a portal the fields hold their defaults
//! and a configuration reads them the same way.

use luna::{Context, Table};
use morf_io::{Bus, DbusProxy, DbusValue};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::{api_state::build, state::*, surface_types::*, types::*};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";
const INTERFACE: &str = "org.gnome.desktop.interface";

/// Every field of `morf.prefers`, by name.
pub(crate) const PREFERENCES: [&str; 5] = [
    "color_scheme",
    "contrast",
    "reduced_motion",
    "accent_color",
    "scale",
];

/// A variant's payload, however many layers of typing wrap it.
fn plain(value: DbusValue) -> DbusValue {
    match value {
        DbusValue::Typed { value, .. } => plain(*value),
        other => other,
    }
}

fn unsigned(value: &DbusValue) -> Option<u64> {
    match value {
        DbusValue::Unsigned(value) => Some(*value),
        DbusValue::Integer(value) => u64::try_from(*value).ok(),
        DbusValue::Number(value) => Some(*value as u64),
        _ => None,
    }
}

/// What a portal setting means as a preference, or nothing when the key is
/// not one this state follows.
pub(crate) fn preference_from_setting(
    namespace: &str,
    key: &str,
    value: DbusValue,
) -> Option<(&'static str, IpcValue)> {
    let value = plain(value);
    match (namespace, key) {
        (APPEARANCE, "color-scheme") => Some((
            "color_scheme",
            IpcValue::String(
                match unsigned(&value)? {
                    1 => "dark",
                    2 => "light",
                    _ => "none",
                }
                .to_owned(),
            ),
        )),
        (APPEARANCE, "contrast") => Some((
            "contrast",
            IpcValue::String(
                if unsigned(&value)? == 1 {
                    "high"
                } else {
                    "none"
                }
                .to_owned(),
            ),
        )),
        (APPEARANCE, "accent-color") => {
            let DbusValue::List(channels) = value else {
                return None;
            };
            let channel = |index: usize| match channels.get(index)? {
                DbusValue::Number(value) => Some(*value),
                _ => None,
            };
            let (red, green, blue) = (channel(0)?, channel(1)?, channel(2)?);
            // Out of range means the desktop has no accent to offer.
            let none = [red, green, blue]
                .iter()
                .any(|value| !(0.0..=1.0).contains(value));
            Some((
                "accent_color",
                if none {
                    IpcValue::Nil
                } else {
                    IpcValue::Color(morf_scene::Color {
                        red: red as f32,
                        green: green as f32,
                        blue: blue as f32,
                        alpha: 1.0,
                    })
                },
            ))
        }
        (INTERFACE, "enable-animations") => match value {
            DbusValue::Bool(enabled) => Some(("reduced_motion", IpcValue::Boolean(!enabled))),
            _ => None,
        },
        _ => None,
    }
}

/// Asks the portal for one setting; `None` when it has nothing to say.
fn read_setting(proxy: &DbusProxy, namespace: &str, key: &str) -> Option<DbusValue> {
    proxy
        .call_value_with(
            "ReadOne",
            &DbusValue::List(vec![
                DbusValue::String(namespace.to_owned()),
                DbusValue::String(key.to_owned()),
            ]),
        )
        .ok()
}

/// The portal's settings interface, its change signal, and what it said.
struct Portal {
    proxy: DbusProxy,
    signal: morf_io::DbusSignal,
    read: Vec<(&'static str, IpcValue)>,
}

/// Connects to the settings portal and reads every preference it knows,
/// leaving the change signal subscribed. `None` when there is no portal.
fn portal_preferences() -> Option<Portal> {
    let proxy = DbusProxy::connect_with_timeout(
        Bus::Session,
        PORTAL.to_owned(),
        PORTAL_PATH.to_owned(),
        SETTINGS.to_owned(),
        Duration::from_millis(250),
    )
    .ok()?;
    let signal = proxy.subscribe("SettingChanged").ok()?;
    let mut read = Vec::new();
    for (namespace, key) in [
        (APPEARANCE, "color-scheme"),
        (APPEARANCE, "contrast"),
        (APPEARANCE, "accent-color"),
        (INTERFACE, "enable-animations"),
    ] {
        if let Some(value) = read_setting(&proxy, namespace, key)
            && let Some(preference) = preference_from_setting(namespace, key, value)
        {
            read.push(preference);
        }
    }
    Some(Portal {
        proxy,
        signal,
        read,
    })
}

pub(crate) fn install_prefers_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
    screen: Option<&Screen>,
) {
    let portal = portal_preferences();
    let seed = Table::new(&ctx);
    seed.set_field(ctx, "color_scheme", "none");
    seed.set_field(ctx, "contrast", "none");
    seed.set_field(ctx, "reduced_motion", false);
    seed.set_field(ctx, "scale", screen.map_or(1, |screen| screen.scale) as i64);
    if let Some(portal) = &portal {
        for (name, value) in &portal.read {
            seed.set_field(ctx, name, value.to_lua(ctx));
        }
    }
    let metatable = state
        .borrow()
        .state_metatable
        .clone()
        .expect("states are installed before preferences");
    let userdata = build(ctx, &state, &metatable, "prefers", None, seed)
        .expect("the preference seed is plain scalars");
    let token = userdata
        .downcast_static::<StateToken>()
        .expect("build makes a state");
    let mut fields = token.fields.borrow_mut();
    // A table cannot seed a nil field, and no accent is nil, so that one
    // signal is made by hand.
    if !fields.scalars.contains_key("accent_color") {
        let mut state = state.borrow_mut();
        let id = state
            .graph
            .as_mut()
            .expect("the graph is not running at install")
            .signal("prefers.accent_color", IpcValue::Nil);
        state.values.insert(id, IpcValue::Nil);
        state.signals.push(id);
        fields.scalars.insert("accent_color".to_owned(), id);
    }
    let id = |name: &str| fields.scalars[name];
    let prefers = Prefers {
        color_scheme: id("color_scheme"),
        contrast: id("contrast"),
        reduced_motion: id("reduced_motion"),
        accent_color: id("accent_color"),
        scale: id("scale"),
        portal: portal.map(|portal| (portal.proxy, portal.signal)),
    };
    let reduced = matches!(
        state.borrow().values.get(&prefers.reduced_motion),
        Some(IpcValue::Boolean(true))
    );
    let mut state = state.borrow_mut();
    state.prefers = Some(prefers);
    state
        .scene
        .set_motion_scale(if reduced { 0.0 } else { 1.0 });
    drop(fields);
    morf.set_field(ctx, "prefers", userdata);
}
