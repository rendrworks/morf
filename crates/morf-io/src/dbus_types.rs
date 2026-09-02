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
use std::sync::{Arc, Mutex, OnceLock, mpsc};
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
        // One connection per bus, and one reader thread on it, with every
        // subscription a route rather than a thread of its own.
        //
        // This began as a connection each: a socket, an authentication
        // handshake and a bus name for every signal a configuration watched. A
        // thread each was the obvious next step and is the wrong one — a thread
        // blocked in `next()` cannot be woken, so ending a subscription would
        // mean either closing the shared connection (taking every other
        // subscription with it) or waiting for a message that may never come.
        // A route can simply be removed.
        let router = router(self.bus)?;
        router.subscribe(Route {
            sender: self.destination.clone(),
            path: self.path.clone(),
            interface: self.interface.clone(),
            member: signal.into(),
        })
    }
}

/// What a subscription is listening for.
///
/// Matched exactly, on all four, because these rules are built here rather than
/// parsed from a configuration — so the general case D-Bus match syntax allows
/// cannot arise, and reimplementing it to handle rules nobody can write would
/// be reimplementing it badly.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Route {
    sender: String,
    path: String,
    interface: String,
    member: String,
}

impl Route {
    fn rule(&self) -> zbus::Result<zbus::MatchRule<'static>> {
        Ok(zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(self.sender.as_str())?
            .path(self.path.as_str())?
            .interface(self.interface.as_str())?
            .member(self.member.as_str())?
            .build()
            .to_owned())
    }

    /// Whether a message is what this route asked for.
    ///
    /// The sender is compared against the *well-known* name the subscription
    /// named, and messages arrive stamped with the sender's unique name — so
    /// this cannot compare them and does not try. The bus already applied the
    /// rule, which included the sender; what is left to separate here is which
    /// of our own routes a delivered signal belongs to.
    fn matches(&self, header: &zbus::message::Header<'_>) -> bool {
        header.path().is_some_and(|path| path.as_str() == self.path)
            && header
                .interface()
                .is_some_and(|interface| interface.as_str() == self.interface)
            && header
                .member()
                .is_some_and(|member| member.as_str() == self.member)
    }
}

/// One reader per bus, dealing signals to whoever asked for them.
pub(crate) struct SignalRouter {
    connection: DbusConnection,
    routes: Mutex<Vec<(u64, Route, mpsc::Sender<zbus::Message>)>>,
    next_id: Mutex<u64>,
}

impl SignalRouter {
    /// The unique bus name this reader's connection holds.
    pub(crate) fn connection_name(&self) -> Option<String> {
        self.connection.unique_name().map(ToString::to_string)
    }

    fn subscribe(self: &Arc<Self>, route: Route) -> zbus::Result<DbusSignal> {
        // Ask the bus to deliver these. Rules are reference counted by the bus,
        // so two subscriptions to the same signal add it twice and it survives
        // until both have gone.
        zbus::blocking::fdo::DBusProxy::new(&self.connection)?.add_match_rule(route.rule()?)?;
        let (tx, events) = mpsc::channel();
        let id = {
            let mut next = self
                .next_id
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *next = next.wrapping_add(1);
            *next
        };
        self.routes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((id, route, tx));
        Ok(DbusSignal {
            events,
            router: Some(Arc::clone(self)),
            id,
        })
    }

    /// Drops a route and tells the bus to stop sending what only it wanted.
    pub(crate) fn unsubscribe(&self, id: u64) {
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(index) = routes.iter().position(|(held, _, _)| *held == id) else {
            return;
        };
        let (_, route, _) = routes.remove(index);
        drop(routes);
        if let Ok(rule) = route.rule()
            && let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&self.connection)
        {
            let _ = proxy.remove_match_rule(rule);
        }
    }

    /// Hands one message to every route that asked for it.
    fn deliver(&self, message: &zbus::Message) {
        let header = message.header();
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A receiver that has gone is a subscription whose owner dropped it
        // without the route being removed — possible if a `DbusSignal` leaked.
        // Clearing them here keeps the list from growing forever.
        routes.retain(|(_, route, tx)| !route.matches(&header) || tx.send(message.clone()).is_ok());
    }
}

/// The one connection this process holds to `bus`, opened on first use.
///
/// Shared rather than pooled: a connection is a socket, a handshake and a name
/// on the bus, and there is no reason for a process to hold more than one of
/// each. Subscriptions are separated by their match rules, not by their
/// sockets.
fn router(bus: Bus) -> zbus::Result<Arc<SignalRouter>> {
    static SESSION: OnceLock<Mutex<Option<Arc<SignalRouter>>>> = OnceLock::new();
    static SYSTEM: OnceLock<Mutex<Option<Arc<SignalRouter>>>> = OnceLock::new();
    let slot = match bus {
        Bus::Session => &SESSION,
        Bus::System => &SYSTEM,
    }
    .get_or_init(|| Mutex::new(None));
    // A poisoned lock means a previous caller panicked while connecting, which
    // says nothing about whether connecting works now.
    let mut held = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(router) = held.as_ref() {
        return Ok(Arc::clone(router));
    }
    let connection = match bus {
        Bus::Session => DbusConnection::session()?,
        Bus::System => DbusConnection::system()?,
    };
    let router = Arc::new(SignalRouter {
        connection: connection.clone(),
        routes: Mutex::new(Vec::new()),
        next_id: Mutex::new(0),
    });
    // The one reader. It lives as long as the process, which is why nothing
    // has to be able to interrupt it: subscriptions come and go as routes, and
    // this never has to be stopped and restarted.
    let reading = Arc::clone(&router);
    thread::spawn(move || {
        for message in zbus::blocking::MessageIterator::from(connection) {
            let Ok(message) = message else { break };
            reading.deliver(&message);
        }
    });
    *held = Some(Arc::clone(&router));
    Ok(router)
}
