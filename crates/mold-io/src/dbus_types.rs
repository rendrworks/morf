/// Message bus used by a generic D-Bus proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bus {
    Session,
    System,
}

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
    pub fn connect(
        bus: Bus,
        destination: impl Into<String>,
        path: impl Into<String>,
        interface: impl Into<String>,
    ) -> zbus::Result<Self> {
        let connection = match bus {
            Bus::Session => DbusConnection::session()?,
            Bus::System => DbusConnection::system()?,
        };
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

    /// Writes one scalar property for an interpreter-facing facade.
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
            DbusValue::List(_) | DbusValue::Map(_) => {
                return Err("compound D-Bus properties are not supported".to_owned());
            }
        };
        result.map_err(|error| error.to_string())
    }

    /// Subscribes to one signal on a dedicated bus connection.
    pub fn subscribe(&self, signal: impl Into<String>) -> zbus::Result<DbusSignal> {
        let connection = match self.bus {
            Bus::Session => DbusConnection::session()?,
            Bus::System => DbusConnection::system()?,
        };
        let proxy = ZbusProxy::new_owned(
            connection.clone(),
            self.destination.clone(),
            self.path.clone(),
            self.interface.clone(),
        )?;
        let iterator = proxy.receive_signal(signal.into())?;
        let (tx, events) = mpsc::channel();
        let join = thread::spawn(move || {
            for message in iterator {
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

