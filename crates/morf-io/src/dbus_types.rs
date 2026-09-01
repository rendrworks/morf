//! Generic D-Bus method and property client.
//!
//! # File descriptors
//!
//! They do not cross into a configuration, in either direction. That is a
//! decision, not an omission.
//!
//! A D-Bus message can carry an fd, and handing one to a sandboxed Lua VM hands
//! it whatever that descriptor is attached to — a file outside the config
//! directory, a socket to another service, a device node. The sandbox exists to
//! make the set of things a configuration can reach an enumerable list, and an
//! fd arriving over the bus is a hole straight through it. Both directions are
//! refused by name, in `dbus_encode` and `dbus_decode`, so the refusal reads as
//! deliberate rather than as a gap nobody got round to filling.
//!
//! If a configuration ever genuinely needs one, the answer is a specific named
//! capability that takes the fd on its behalf — never a general one.

use crate::dbus_decode::basic_value;
use crate::dbus_encode::dbus_argument_value;
use crate::dbus_encode::decode_message_value;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use zbus::blocking::{Connection as DbusConnection, Proxy as ZbusProxy};

use crate::dbus_decode::DbusSignal;
use zbus::zvariant::{DynamicDeserialize, DynamicType, OwnedValue, StructureBuilder, Value};

/// Message bus used by a generic D-Bus proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bus {
    Session,
    System,
}

/// How long a call waits for its reply before giving up.
///
/// zbus defaults to twenty-five seconds, which is the right answer for a
/// program that can afford to wait and the wrong one for this: a configuration
/// calls out from a Lua handler, and a Lua handler runs on the thread that
/// paints. A service that hangs would take the shell with it for twenty-five
/// seconds — no repaints, no input — and there is no service worth that.
///
/// A second is far longer than any healthy reply on a session bus and short
/// enough that a bad one costs a stutter rather than a hang.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_millis(1000);

/// Typed generic D-Bus method and property client.
#[derive(Clone, Debug)]
pub struct DbusProxy {
    proxy: ZbusProxy<'static>,
    bus: Bus,
    destination: String,
    path: String,
    interface: String,
}

/// Bounded value transferable through the Lua D-Bus facade.
#[derive(Clone, Debug, PartialEq)]
pub enum DbusValue {
    Nil,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Number(f64),
    String(String),
    List(Vec<DbusValue>),
    Map(BTreeMap<String, DbusValue>),
    Typed {
        signature: String,
        value: Box<DbusValue>,
    },
}

impl DbusProxy {
    /// Connects a proxy to one bus object and interface.
    ///
    /// Calls are bounded by [`DEFAULT_CALL_TIMEOUT`]; use
    /// [`Self::connect_with_timeout`] to choose another.
    pub fn connect(
        bus: Bus,
        destination: impl Into<String>,
        path: impl Into<String>,
        interface: impl Into<String>,
    ) -> zbus::Result<Self> {
        Self::connect_with_timeout(bus, destination, path, interface, DEFAULT_CALL_TIMEOUT)
    }

    /// Connects a proxy whose calls give up after `timeout`.
    ///
    /// The bound is on the connection rather than the call, which is where zbus
    /// puts it — so it is per proxy, and a configuration that genuinely needs to
    /// wait on one slow service can say so without slowing every other call it
    /// makes.
    pub fn connect_with_timeout(
        bus: Bus,
        destination: impl Into<String>,
        path: impl Into<String>,
        interface: impl Into<String>,
        timeout: Duration,
    ) -> zbus::Result<Self> {
        let connection = match bus {
            Bus::Session => zbus::blocking::connection::Builder::session()?,
            Bus::System => zbus::blocking::connection::Builder::system()?,
        }
        .method_timeout(timeout)
        .build()?;
        let destination = destination.into();
        let path = path.into();
        let interface = interface.into();
        let proxy = ZbusProxy::new_owned(
            connection,
            destination.clone(),
            path.clone(),
            interface.clone(),
        )?;
        Ok(Self {
            proxy,
            bus,
            destination,
            path,
            interface,
        })
    }

    /// How long this proxy's calls wait before giving up.
    ///
    /// Readable so the bound can be asserted on. A timeout is only observable
    /// by waiting for it, and a test that waits for a real one has to find a
    /// peer willing to accept a call and never answer — which is a harder thing
    /// to arrange than the bug is to prevent.
    pub fn call_timeout(&self) -> Option<Duration> {
        self.proxy.connection().method_timeout()
    }

    /// Returns the connection's unique bus name.
    pub fn unique_name(&self) -> Option<String> {
        self.proxy
            .connection()
            .unique_name()
            .map(ToString::to_string)
    }

    /// Calls one method and deserializes its reply body.
    pub fn call<B, R>(&self, method: &str, body: &B) -> zbus::Result<R>
    where
        B: Serialize + DynamicType,
        R: for<'de> DynamicDeserialize<'de>,
    {
        self.proxy.call(method, body)
    }

    /// Reads one remote property.
    pub fn get_property<T>(&self, property: &str) -> zbus::Result<T>
    where
        T: TryFrom<OwnedValue>,
        T::Error: Into<zbus::Error>,
    {
        self.proxy.get_property(property)
    }

    /// Writes one remote property.
    pub fn set_property<'value, T>(&self, property: &str, value: T) -> zbus::Result<()>
    where
        T: 'value + Into<Value<'value>>,
    {
        Ok(self.proxy.set_property(property, value)?)
    }

    /// Returns the remote object's introspection XML.
    pub fn introspect(&self) -> zbus::Result<String> {
        Ok(self.proxy.introspect()?)
    }

    /// Reads one property for an interpreter-facing facade.
    pub fn get_value(&self, property: &str) -> Result<DbusValue, String> {
        let value: OwnedValue = self
            .proxy
            .get_property(property)
            .map_err(|error| error.to_string())?;
        basic_value(&value)
    }

    /// Calls a no-argument method returning a supported value.
    pub fn call_value(&self, method: &str) -> Result<DbusValue, String> {
        let message = self
            .proxy
            .call_method(method, &())
            .map_err(|error| error.to_string())?;
        decode_message_value(&message)
    }

    /// Calls a method with one scalar or a list of positional scalar arguments.
    pub fn call_value_with(&self, method: &str, value: &DbusValue) -> Result<DbusValue, String> {
        let message = match value {
            DbusValue::Nil => self.proxy.call_method(method, &()),
            DbusValue::Bool(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Integer(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Unsigned(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::Number(value) => self.proxy.call_method(method, &(*value,)),
            DbusValue::String(value) => self.proxy.call_method(method, &(value.as_str(),)),
            DbusValue::Typed { .. } => {
                let body = StructureBuilder::new()
                    .append_field(dbus_argument_value(value)?)
                    .build()
                    .map_err(|error| error.to_string())?;
                self.proxy.call_method(method, &body)
            }
            DbusValue::List(values) if values.is_empty() => self.proxy.call_method(method, &()),
            DbusValue::List(values) => {
                let mut body = StructureBuilder::new();
                for value in values {
                    body = body.append_field(dbus_argument_value(value)?);
                }
                let body = body.build().map_err(|error| error.to_string())?;
                self.proxy.call_method(method, &body)
            }
            DbusValue::Map(_) => {
                return Err("D-Bus maps need an explicit signature".to_owned());
            }
        }
        .map_err(|error| error.to_string())?;
        decode_message_value(&message)
    }

    /// Writes one property for an interpreter-facing facade.
    pub fn set_value(&self, property: &str, value: &DbusValue) -> Result<(), String> {
        let result = match value {
            DbusValue::Nil => return Err("D-Bus properties cannot be nil".to_owned()),
            DbusValue::Bool(value) => self.set_property(property, *value),
            DbusValue::Integer(value) => self.set_property(property, *value),
            DbusValue::Unsigned(value) => self.set_property(property, *value),
            DbusValue::Number(value) => self.set_property(property, *value),
            DbusValue::String(value) => self.set_property(property, value.as_str()),
            DbusValue::Typed { .. } => {
                let value = dbus_argument_value(value)?;
                self.set_property(property, value)
            }
            // A list or a map needs its wire type stated, because there is no
            // way to infer one: an empty list could be an array of anything,
            // and a map of numbers could be `a{sd}` or `a{si}`. Reading needs
            // no such guess — the reply carries its own signature — which is
            // why this was asymmetric, and why the answer is to ask rather than
            // to refuse.
            DbusValue::List(_) | DbusValue::Map(_) => {
                return Err(
                    "a compound D-Bus property needs its signature stated: pass `{ signature = \"as\", value = ... }` rather than a bare table"
                        .to_owned(),
                );
            }
        };
        result.map_err(|error| error.to_string())
    }

    /// Subscribes to one signal on a dedicated bus connection.
    pub fn subscribe(&self, signal: impl Into<String>) -> zbus::Result<DbusSignal> {
        // One connection per bus, shared by every subscription on it.
        //
        // This used to open its own: a socket, an authentication handshake and
        // a bus registration for each signal a configuration watched. A shell
        // that follows battery, network, player and tray paid four of each for
        // work one connection does. The match rule is what separates them, and
        // the bus has always been able to carry as many as it is asked to.
        //
        // The rule is registered by `for_match_rule` and deregistered when the
        // iterator drops, so a configuration that stops listening stops costing
        // the bus anything — which hand-rolled routing would have had to
        // remember to do.
        let connection = shared_connection(self.bus)?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(self.destination.as_str())?
            .path(self.path.as_str())?
            .interface(self.interface.as_str())?
            .member(signal.into())?
            .build();
        let iterator = zbus::blocking::MessageIterator::for_match_rule(
            rule,
            &connection,
            // Deep enough that a burst is not dropped between polls, shallow
            // enough that a subscription nobody reads cannot grow without
            // bound.
            Some(64),
        )?;
        let (tx, events) = mpsc::channel();
        let join = thread::spawn(move || {
            for message in iterator {
                // An error here is the connection going away, not one bad
                // message: ending the thread lets the receiver see the
                // disconnect rather than spinning on a stream that will not
                // recover.
                let Ok(message) = message else { break };
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        Ok(DbusSignal {
            events,
            connection: Some(connection),
            join: Some(join),
        })
    }
}

/// The one connection this process holds to `bus`, opened on first use.
///
/// Shared rather than pooled: a connection is a socket, a handshake and a name
/// on the bus, and there is no reason for a process to hold more than one of
/// each. Subscriptions are separated by their match rules, not by their
/// sockets.
fn shared_connection(bus: Bus) -> zbus::Result<DbusConnection> {
    static SESSION: OnceLock<Mutex<Option<DbusConnection>>> = OnceLock::new();
    static SYSTEM: OnceLock<Mutex<Option<DbusConnection>>> = OnceLock::new();
    let slot = match bus {
        Bus::Session => &SESSION,
        Bus::System => &SYSTEM,
    }
    .get_or_init(|| Mutex::new(None));
    // A poisoned lock means a previous caller panicked while connecting, which
    // says nothing about whether connecting works now.
    let mut held = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(connection) = held.as_ref() {
        return Ok(connection.clone());
    }
    let connection = match bus {
        Bus::Session => DbusConnection::session()?,
        Bus::System => DbusConnection::system()?,
    };
    *held = Some(connection.clone());
    Ok(connection)
}
