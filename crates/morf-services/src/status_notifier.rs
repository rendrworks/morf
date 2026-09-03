use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

use morf_io::{Bus, DbusProxy, DbusSignal, DbusValue, PendingReply};

/// Session-bus destination and object path for one tray item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusNotifierAddress {
    /// Unique or well-known D-Bus service name.
    pub service: String,
    /// StatusNotifierItem object path.
    pub path: String,
}

impl StatusNotifierAddress {
    /// Parses watcher entries with optional inline object paths.
    pub fn parse(value: &str) -> Result<Self, StatusNotifierError> {
        let (service, path) = value
            .find('/')
            .map_or((value, "/StatusNotifierItem"), |index| {
                (&value[..index], &value[index..])
            });
        if service.is_empty() || !path.starts_with('/') {
            return Err(StatusNotifierError(format!(
                "invalid status notifier address `{value}`"
            )));
        }
        Ok(Self {
            service: service.to_owned(),
            path: path.to_owned(),
        })
    }

    fn key(&self) -> String {
        format!("{}{}", self.service, self.path)
    }
}

/// Status notifier watcher or protocol failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusNotifierError(String);

impl fmt::Display for StatusNotifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl StdError for StatusNotifierError {}

/// Registered host and nonblocking watcher event streams.
pub struct StatusNotifierHost {
    watcher: DbusProxy,
    registered: DbusSignal,
    unregistered: DbusSignal,
    items: BTreeMap<String, StatusNotifierAddress>,
    initial: bool,
    /// Whether the watcher has answered once: its item list read and this
    /// host registered with it. Done from the poll rather than from
    /// `connect_to`, and the reason matters twice over. A watcher served by
    /// *this* process can only answer between frames, so a blocking fetch at
    /// connect time waited on itself; and a watcher that is not running yet
    /// used to make connecting an error, when what a shell wants is an empty
    /// tray that fills the moment one appears.
    bootstrapped: bool,
    /// Polls to wait before asking again. Zero means ask now.
    retry_in: u32,
    /// The item list, asked for and not yet answered.
    pending_items: Option<PendingReply>,
    /// The namespaces to look under, and which one the proxy is on now.
    ///
    /// A subscription needs no owner, so connecting to `org.freedesktop`
    /// succeeds whether or not anybody serves it; only the fetch finds out.
    /// When it finds out, the host moves on to the next namespace and asks
    /// there, which is how it ends up on `org.kde` where the watchers are.
    namespaces: Vec<String>,
    current: usize,
    /// The host registration, in flight. Kept only so the thread's answer has
    /// somewhere to go; nothing reads it.
    _pending_register: Option<PendingReply>,
}

impl StatusNotifierHost {
    /// The bus namespaces this engine will look under on its own.
    ///
    /// `org.kde` first: the specification says `org.freedesktop`, and every
    /// item in the wild registers under `org.kde`, because that is where the
    /// specification came from and nobody renamed. A host that asked only
    /// under the specified name found an empty tray on a desk full of icons.
    /// The neutral name stays as the fallback, and a configuration can name
    /// its own order.
    pub const DEFAULT_NAMESPACES: [&str; 2] = ["org.kde", "org.freedesktop"];

    /// Registers a host with the first default watcher that answers.
    pub fn connect() -> Result<Self, StatusNotifierError> {
        Self::connect_to(&Self::DEFAULT_NAMESPACES)
    }

    /// Registers a host with the first of `namespaces` that answers.
    ///
    /// The order is the configuration's, because it is the configuration that
    /// knows what is running: on a session whose tray answers to a vendor
    /// prefix, naming it here is a one-line fact about that session rather than
    /// a permanent assumption baked into the engine.
    pub fn connect_to(namespaces: &[&str]) -> Result<Self, StatusNotifierError> {
        let mut last_error = None;
        for (index, namespace) in namespaces.iter().enumerate() {
            match Self::connect_namespace(namespace) {
                Ok((watcher, registered, unregistered)) => {
                    return Ok(Self {
                        watcher,
                        registered,
                        unregistered,
                        items: BTreeMap::new(),
                        initial: true,
                        bootstrapped: false,
                        retry_in: 0,
                        pending_items: None,
                        _pending_register: None,
                        namespaces: namespaces.iter().map(|n| (*n).to_owned()).collect(),
                        current: index,
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            StatusNotifierError("status notifier watcher is unavailable".to_owned())
        }))
    }

    fn connect_namespace(
        namespace: &str,
    ) -> Result<(DbusProxy, DbusSignal, DbusSignal), StatusNotifierError> {
        let interface = format!("{namespace}.StatusNotifierWatcher");
        let watcher = DbusProxy::connect(
            Bus::Session,
            interface.clone(),
            "/StatusNotifierWatcher",
            interface,
        )
        .map_err(|error| StatusNotifierError(error.to_string()))?;
        let registered = watcher
            .subscribe("StatusNotifierItemRegistered")
            .map_err(|error| StatusNotifierError(error.to_string()))?;
        let unregistered = watcher
            .subscribe("StatusNotifierItemUnregistered")
            .map_err(|error| StatusNotifierError(error.to_string()))?;
        Ok((watcher, registered, unregistered))
    }

    /// Reads the watcher's list and registers this host, once it answers.
    ///
    /// Asked for off-thread and collected here on a later poll -- never waited
    /// on. `false` while there is no answer yet or the last one was a refusal;
    /// the poll asks again after a pause.
    fn bootstrap(&mut self) -> Result<bool, StatusNotifierError> {
        let pending = self
            .pending_items
            .get_or_insert_with(|| self.watcher.get_later("RegisteredStatusNotifierItems"));
        let Some(reply) = pending.try_take() else {
            return Ok(false);
        };
        self.pending_items = None;
        let Ok(value) = reply else {
            // Nobody answered under this name. Try the next namespace soon,
            // and once every name has been tried, wait about a second between
            // rounds: a session with no watcher at all should cost a little,
            // not a poll.
            self.current = (self.current + 1) % self.namespaces.len().max(1);
            if let Ok((watcher, registered, unregistered)) =
                Self::connect_namespace(&self.namespaces[self.current])
            {
                self.watcher = watcher;
                self.registered = registered;
                self.unregistered = unregistered;
            }
            self.retry_in = if self.current == 0 { 60 } else { 5 };
            return Ok(false);
        };
        for item in notifier_items(value)? {
            self.items.insert(item.key(), item);
        }
        let unique_name = self
            .watcher
            .unique_name()
            .ok_or_else(|| StatusNotifierError("session bus supplied no unique name".to_owned()))?;
        // Best effort: a watcher that lists items but refuses hosts still has
        // items worth showing.
        self._pending_register = Some(
            self.watcher
                .call_later_with("RegisterStatusNotifierHost", DbusValue::String(unique_name)),
        );
        Ok(true)
    }

    /// Drains watcher signals and returns a new complete item snapshot.
    pub fn poll_changed(
        &mut self,
    ) -> Result<Option<Vec<StatusNotifierAddress>>, StatusNotifierError> {
        let mut changed = std::mem::take(&mut self.initial);
        if !self.bootstrapped {
            if self.retry_in > 0 {
                self.retry_in -= 1;
            } else if self.bootstrap()? {
                self.bootstrapped = true;
                changed = true;
            }
        }
        for _ in 0..64 {
            let Some(value) = self.registered.next_value(Duration::ZERO) else {
                break;
            };
            let item = signal_address(value.map_err(StatusNotifierError)?)?;
            changed |= self.items.insert(item.key(), item).is_none();
        }
        for _ in 0..64 {
            let Some(value) = self.unregistered.next_value(Duration::ZERO) else {
                break;
            };
            let item = signal_address(value.map_err(StatusNotifierError)?)?;
            changed |= self.items.remove(&item.key()).is_some();
        }
        Ok(changed.then(|| self.items.values().cloned().collect()))
    }
}

fn notifier_items(value: DbusValue) -> Result<Vec<StatusNotifierAddress>, StatusNotifierError> {
    let DbusValue::List(values) = value else {
        return Err(StatusNotifierError(
            "watcher returned an invalid item list".to_owned(),
        ));
    };
    values
        .into_iter()
        .map(|value| match value {
            DbusValue::String(value) => StatusNotifierAddress::parse(&value),
            _ => Err(StatusNotifierError(
                "watcher returned a non-string item".to_owned(),
            )),
        })
        .collect()
}

fn signal_address(value: DbusValue) -> Result<StatusNotifierAddress, StatusNotifierError> {
    match value {
        DbusValue::String(value) => StatusNotifierAddress::parse(&value),
        DbusValue::List(mut values) if values.len() == 1 => match values.remove(0) {
            DbusValue::String(value) => StatusNotifierAddress::parse(&value),
            _ => Err(StatusNotifierError(
                "watcher returned a non-string item".to_owned(),
            )),
        },
        _ => Err(StatusNotifierError(
            "watcher returned an invalid item signal".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_service_and_combined_item_addresses() {
        assert_eq!(
            StatusNotifierAddress::parse("org.example.Player").unwrap(),
            StatusNotifierAddress {
                service: "org.example.Player".to_owned(),
                path: "/StatusNotifierItem".to_owned(),
            }
        );
        assert_eq!(
            StatusNotifierAddress::parse(":1.42/Tray").unwrap(),
            StatusNotifierAddress {
                service: ":1.42".to_owned(),
                path: "/Tray".to_owned(),
            }
        );
    }

    #[test]
    fn the_engine_names_no_desktop_environment() {
        // A Wayland engine that hard-codes one desktop's bus prefix has taken a
        // side, and every configuration built on it inherits that. The default
        // is the vendor-neutral name and nothing else; anything with somebody's
        // project in it is a fact about a particular session, which is the
        // configuration's to supply.
        assert_eq!(
            StatusNotifierHost::DEFAULT_NAMESPACES,
            ["org.kde", "org.freedesktop"]
        );
        assert!(
            StatusNotifierHost::DEFAULT_NAMESPACES.contains(&"org.freedesktop"),
            "the default watcher name belongs to no desktop environment",
        );
    }
}
