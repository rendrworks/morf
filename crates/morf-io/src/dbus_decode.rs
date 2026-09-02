use crate::dbus_encode::decode_message_value;
use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use zbus::zvariant::{Array, Dict, OwnedValue, Structure, Value};

use crate::dbus_types::DbusValue;

pub(crate) fn dynamic_value(value: &Value<'_>) -> Result<DbusValue, String> {
    Ok(match value {
        Value::U8(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::Bool(value) => DbusValue::Bool(*value),
        Value::I16(value) => DbusValue::Integer(i64::from(*value)),
        Value::U16(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::I32(value) => DbusValue::Integer(i64::from(*value)),
        Value::U32(value) => DbusValue::Unsigned(u64::from(*value)),
        Value::I64(value) => DbusValue::Integer(*value),
        Value::U64(value) => DbusValue::Unsigned(*value),
        Value::F64(value) => DbusValue::Number(*value),
        Value::Str(value) => DbusValue::String(value.to_string()),
        Value::Signature(value) => DbusValue::String(value.to_string()),
        Value::ObjectPath(value) => DbusValue::String(value.to_string()),
        Value::Value(value) => dynamic_value(value)?,
        Value::Array(value) => array_value(value)?,
        Value::Dict(value) => dict_value(value)?,
        Value::Structure(value) => structure_value(value)?,
        #[cfg(unix)]
        Value::Fd(_) => return Err("D-Bus file descriptors cannot cross into Lua".to_owned()),
        #[allow(unreachable_patterns)]
        _ => return Err("D-Bus value is not supported".to_owned()),
    })
}

pub(crate) fn structure_value(value: &Structure<'_>) -> Result<DbusValue, String> {
    value
        .fields()
        .iter()
        .map(dynamic_value)
        .collect::<Result<Vec<_>, _>>()
        .map(DbusValue::List)
}

pub(crate) fn array_value(value: &Array<'_>) -> Result<DbusValue, String> {
    value
        .inner()
        .iter()
        .map(dynamic_value)
        .collect::<Result<Vec<_>, _>>()
        .map(DbusValue::List)
}

fn dict_value(value: &Dict<'_, '_>) -> Result<DbusValue, String> {
    let mut map = BTreeMap::new();
    for (key, value) in value.iter() {
        let key = match dynamic_value(key)? {
            DbusValue::String(key) => key,
            _ => return Err("D-Bus dictionary keys must be strings".to_owned()),
        };
        map.insert(key, dynamic_value(value)?);
    }
    Ok(DbusValue::Map(map))
}

/// Blocking receiver for a filtered D-Bus signal stream.
pub struct DbusSignal {
    pub(crate) events: mpsc::Receiver<zbus::Message>,
    /// The reader this subscription is a route on.
    ///
    /// Held so that dropping the subscription can remove the route and tell the
    /// bus to stop sending what only it wanted.
    pub(crate) router: Option<Arc<crate::dbus_types::SignalRouter>>,
    pub(crate) id: u64,
}

impl DbusSignal {
    /// The unique bus name of the connection this subscription rides on.
    ///
    /// Exposed so that "these share a connection" is a thing a test can assert
    /// rather than a thing a comment claims: one connection has one unique
    /// name, and four have four.
    pub fn connection_name(&self) -> Option<String> {
        self.router
            .as_ref()
            .and_then(|router| router.connection_name())
    }

    /// Waits for the next signal message.
    pub fn next(&self, timeout: Duration) -> Option<zbus::Message> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Waits for and decodes the next scalar signal body.
    pub fn next_value(&self, timeout: Duration) -> Option<Result<DbusValue, String>> {
        self.next(timeout)
            .map(|message| decode_message_value(&message))
    }
}

impl Drop for DbusSignal {
    fn drop(&mut self) {
        // The route goes; the connection and its reader stay. They belong to
        // the process, not to this subscription, and closing them would take
        // every other subscription down with it.
        if let Some(router) = self.router.take() {
            router.unsubscribe(self.id);
        }
    }
}

/// Decodes one property value.
///
/// This used to probe `TryFrom` for each scalar type in turn and give up at the
/// end of the list, which quietly excluded the two scalars the list forgot:
/// zvariant's conversions match one exact variant each, so an object path or a
/// signature matched nothing and came back "not a supported scalar" — even
/// though `dynamic_value` beside it has always handled both. Every property
/// carrying a `o` or `g` was unreadable for no reason anyone chose.
pub(crate) fn basic_value(value: &OwnedValue) -> Result<DbusValue, String> {
    dynamic_value(value)
}
