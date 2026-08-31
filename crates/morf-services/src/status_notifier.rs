use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

use morf_io::{Bus, DbusProxy, DbusSignal, DbusValue};

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
    _watcher: DbusProxy,
    registered: DbusSignal,
    unregistered: DbusSignal,
    items: BTreeMap<String, StatusNotifierAddress>,
    initial: bool,
}

impl StatusNotifierHost {
    /// Registers a host with an available freedesktop or KDE watcher.
    pub fn connect() -> Result<Self, StatusNotifierError> {
        let mut last_error = None;
        for namespace in ["org.freedesktop", "org.kde"] {
            match Self::connect_namespace(namespace) {
                Ok(host) => return Ok(host),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            StatusNotifierError("status notifier watcher is unavailable".to_owned())
        }))
    }

    fn connect_namespace(namespace: &str) -> Result<Self, StatusNotifierError> {
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
        let items = notifier_items(
            watcher
                .get_value("RegisteredStatusNotifierItems")
                .map_err(StatusNotifierError)?,
        )?;
        let unique_name = watcher
            .unique_name()
            .ok_or_else(|| StatusNotifierError("session bus supplied no unique name".to_owned()))?;
        watcher
            .call_value_with(
                "RegisterStatusNotifierHost",
                &DbusValue::String(unique_name),
            )
            .map_err(StatusNotifierError)?;
        Ok(Self {
            _watcher: watcher,
            registered,
            unregistered,
            items: items.into_iter().map(|item| (item.key(), item)).collect(),
            initial: true,
        })
    }

    /// Drains watcher signals and returns a new complete item snapshot.
    pub fn poll_changed(
        &mut self,
    ) -> Result<Option<Vec<StatusNotifierAddress>>, StatusNotifierError> {
        let mut changed = std::mem::take(&mut self.initial);
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
}
