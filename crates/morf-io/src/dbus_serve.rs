//! Owning a bus name, and answering calls on it.
//!
//! The other half of D-Bus, and the first time this engine is a service rather
//! than a client. What it unlocks is everything that requires *being* something
//! on the bus: a notification server, an MPRIS player, a portal backend. Every
//! one of those is a name plus a handful of methods, and none of them is
//! possible while a process can only make calls.
//!
//! Deliberately not zbus's `ObjectServer`. That dispatches to Rust methods
//! known at compile time, and what is needed here is dispatch to a Lua handler
//! chosen at run time — so the calls are read off the connection and answered
//! by hand. That is more code, and it is the code that lets a configuration
//! decide what interface it offers.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zbus::blocking::{Connection as DbusConnection, MessageIterator};
use zbus::fdo::RequestNameFlags;
use zbus::message::Type as MessageType;

use crate::dbus_encode::dbus_argument_value;
use crate::dbus_types::{Bus, DbusValue};

/// What happened when a name was asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameOutcome {
    /// The name is ours.
    Owned,
    /// Somebody else holds it and would not give it up.
    ///
    /// Reported rather than swallowed, because the difference matters and is
    /// invisible from the outside: a notification server that quietly failed to
    /// take `org.freedesktop.Notifications` looks exactly like one that took it
    /// and is never sent anything.
    Taken,
    /// Ours is queued behind the current owner, and may arrive later.
    Queued,
}

/// One method call waiting for an answer.
#[derive(Clone, Debug)]
pub struct DbusCall {
    /// Correlates the reply with the call. Meaningless outside this process.
    pub id: u64,
    pub interface: String,
    pub member: String,
    pub path: String,
    /// Who called, as a unique bus name.
    pub sender: String,
    /// Arguments, decoded the same way a reply body is.
    pub arguments: DbusValue,
}

/// A bus name this process owns, and the calls arriving on it.
pub struct DbusService {
    connection: DbusConnection,
    name: String,
    calls: mpsc::Receiver<(u64, zbus::Message)>,
    /// Calls that have been handed out and not yet answered.
    ///
    /// Held so a reply can be addressed to the message it answers. A call that
    /// is never answered stays here, which is a leak of one message — and the
    /// alternative, dropping it, is a caller that waits for its own timeout.
    pending: HashMap<u64, zbus::Message>,
    next_id: u64,
    join: Option<thread::JoinHandle<()>>,
}

impl DbusService {
    /// Takes `name` on `bus` and starts collecting calls for `path`.
    ///
    /// `replace` asks the current owner to hand it over, and allows a later
    /// process to do the same to us — which is what makes a shell restartable
    /// without the user first killing it.
    pub fn own(
        bus: Bus,
        name: &str,
        path: &str,
        replace: bool,
    ) -> zbus::Result<(Self, NameOutcome)> {
        let connection = match bus {
            Bus::Session => zbus::blocking::connection::Builder::session()?,
            Bus::System => zbus::blocking::connection::Builder::system()?,
        }
        .build()?;
        // `AllowReplacement` either way: a shell that cannot be restarted
        // without first being killed is a shell nobody restarts.
        //
        // `DoNotQueue` when not replacing, because without it the bus puts us
        // in a queue and reports success — and a configuration would believe it
        // owned a name it does not, which is the exact confusion this reports
        // rather than swallows.
        let flags = if replace {
            RequestNameFlags::AllowReplacement | RequestNameFlags::ReplaceExisting
        } else {
            RequestNameFlags::AllowReplacement | RequestNameFlags::DoNotQueue
        };
        // No name is a service on the connection's unique name alone. That is
        // what an agent is: polkit calls back whoever registered, at the path
        // they registered, and the system bus would not grant an unprivileged
        // process a well-known name to be called on anyway.
        let outcome = if name.is_empty() {
            NameOutcome::Owned
        } else {
            match connection.request_name_with_flags(name, flags) {
                Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => NameOutcome::Owned,
                Ok(zbus::fdo::RequestNameReply::InQueue) => NameOutcome::Queued,
                Ok(_) => NameOutcome::Taken,
                Err(zbus::Error::NameTaken) => NameOutcome::Taken,
                Err(error) => return Err(error),
            }
        };

        // Method calls addressed to our object. Signals and replies are not our
        // business here, and a rule that let them through would mean deciding
        // what to do with them on every iteration.
        let rule = zbus::MatchRule::builder()
            .msg_type(MessageType::MethodCall)
            .path(path.to_owned())?
            .build();
        let iterator = MessageIterator::for_match_rule(rule, &connection, Some(64))?;
        let (tx, calls) = mpsc::channel();
        let join = thread::spawn(move || {
            let mut id = 0u64;
            for message in iterator {
                let Ok(message) = message else { break };
                id = id.wrapping_add(1);
                if tx.send((id, message)).is_err() {
                    break;
                }
            }
        });
        let name = if name.is_empty() {
            connection
                .unique_name()
                .map(|unique| unique.to_string())
                .unwrap_or_default()
        } else {
            name.to_owned()
        };
        Ok((
            Self {
                connection,
                name,
                calls,
                pending: HashMap::new(),
                next_id: 0,
                join: Some(join),
            },
            outcome,
        ))
    }

    /// The name this service holds.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The next call waiting, if one arrived within `timeout`.
    pub fn next_call(&mut self, timeout: Duration) -> Option<DbusCall> {
        let (_, message) = self.calls.recv_timeout(timeout).ok()?;
        let header = message.header();
        let call = DbusCall {
            id: {
                self.next_id = self.next_id.wrapping_add(1);
                self.next_id
            },
            interface: header
                .interface()
                .map(ToString::to_string)
                .unwrap_or_default(),
            member: header.member().map(ToString::to_string).unwrap_or_default(),
            path: header.path().map(ToString::to_string).unwrap_or_default(),
            sender: header.sender().map(ToString::to_string).unwrap_or_default(),
            arguments: arguments_of(&message),
        };
        self.pending.insert(call.id, message);
        Some(call)
    }

    /// Answers a call.
    ///
    /// `DbusValue::Nil` is a reply with no body, which is what most methods
    /// return and is not the same as no reply at all — a caller waits for its
    /// timeout on the second.
    pub fn reply(&mut self, id: u64, value: &DbusValue) -> Result<(), String> {
        let message = self.pending.remove(&id).ok_or("no such call")?;
        let header = message.header();
        match value {
            DbusValue::Nil => self.connection.reply(&header, &()),
            // A list is several out-arguments, the same convention the call
            // path uses for inputs. `GetServerInformation` answers with four
            // strings and its caller checks for four, not for one struct of
            // four -- and an element that is itself compound says so with a
            // signature, exactly as it would as an input.
            DbusValue::List(values) => {
                let body = positional_body(values)?;
                self.connection.reply(&header, &body)
            }
            // One value is one argument, and still goes through the body
            // builder: handed over bare, a `Value` serialises as a variant,
            // and a caller that asked for `u` and is given `v` rejects it.
            other => {
                let body = positional_body(std::slice::from_ref(other))?;
                self.connection.reply(&header, &body)
            }
        }
        .map_err(|error| error.to_string())
    }

    /// Answers a call with an error.
    pub fn reply_error(&mut self, id: u64, name: &str, message: &str) -> Result<(), String> {
        let call = self.pending.remove(&id).ok_or("no such call")?;
        self.connection
            .reply_error(&call.header(), name, &message)
            .map_err(|error| error.to_string())
    }

    /// Calls a method on another service, from this service's connection.
    ///
    /// Which connection a call comes from is visible to the callee as the
    /// caller's unique name, and some services keep it: polkit registers an
    /// agent under the name that asked, and calls that name back. An agent
    /// registered through a proxy -- a separate connection -- is called at
    /// an object that is not there, and the caller waits on it forever. So
    /// a registration of this object has to come from here.
    pub fn call(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        arguments: &DbusValue,
    ) -> Result<DbusValue, String> {
        let reply = match arguments {
            DbusValue::Nil => {
                self.connection
                    .call_method(Some(destination), path, Some(interface), member, &())
            }
            DbusValue::List(values) => {
                let body = positional_body(values)?;
                self.connection
                    .call_method(Some(destination), path, Some(interface), member, &body)
            }
            other => {
                let body = positional_body(std::slice::from_ref(other))?;
                self.connection
                    .call_method(Some(destination), path, Some(interface), member, &body)
            }
        }
        .map_err(|error| error.to_string())?;
        crate::dbus_encode::decode_message_value(&reply)
    }

    /// Emits a signal from this service's object.
    pub fn emit(
        &self,
        path: &str,
        interface: &str,
        member: &str,
        value: &DbusValue,
    ) -> Result<(), String> {
        match value {
            DbusValue::Nil => {
                self.connection
                    .emit_signal(None::<&str>, path, interface, member, &())
            }
            // Positional, as for a reply: `NotificationClosed` carries two
            // unsigned integers, not a pair.
            DbusValue::List(values) => {
                let body = positional_body(values)?;
                self.connection
                    .emit_signal(None::<&str>, path, interface, member, &body)
            }
            other => {
                let body = positional_body(std::slice::from_ref(other))?;
                self.connection
                    .emit_signal(None::<&str>, path, interface, member, &body)
            }
        }
        .map_err(|error| error.to_string())
    }
}

/// A call's arguments, always as a list of them.
///
/// The decoder hands back one value for the body as a whole, which is fine
/// for a reply and a trap for a handler: one argument arrived bare, several
/// arrived as a list, and `call.arguments[1]` worked or did not depending on
/// how many the caller sent. The message signature says how many there are,
/// and that is what decides the shape here -- one argument is a list of one,
/// none is an empty list, and only several were a list already.
fn arguments_of(message: &zbus::Message) -> DbusValue {
    let body = message.body();
    let count = match body.signature() {
        zbus::zvariant::Signature::Unit => 0,
        zbus::zvariant::Signature::Structure(fields) => fields.len(),
        _ => 1,
    };
    let decoded = crate::dbus_encode::decode_message_value(message).unwrap_or(DbusValue::Nil);
    match (count, decoded) {
        (0, _) => DbusValue::List(Vec::new()),
        (1, value) => DbusValue::List(vec![value]),
        (_, DbusValue::List(values)) => DbusValue::List(values),
        (_, value) => DbusValue::List(vec![value]),
    }
}

/// Several values as one message body.
///
/// A D-Bus body is a struct without its outer parentheses, which is what a
/// `Structure` serialises to when it is the whole body -- so a list of values
/// packed into one arrives as that many arguments.
fn positional_body(values: &[DbusValue]) -> Result<zbus::zvariant::Structure<'_>, String> {
    let mut builder = zbus::zvariant::StructureBuilder::new();
    for value in values {
        builder = builder.append_field(dbus_argument_value(value)?);
    }
    builder.build().map_err(|error| error.to_string())
}

impl Drop for DbusService {
    fn drop(&mut self) {
        // Releasing the name lets whoever is queued behind us take over
        // immediately rather than waiting for the connection to be noticed as
        // gone. A unique name cannot be released and the call is skipped.
        if !self.name.starts_with(':') {
            let _ = self.connection.release_name(self.name.as_str());
        }
        drop(self.join.take());
    }
}
