fn dynamic_value(value: &Value<'_>) -> Result<DbusValue, String> {
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

fn structure_value(value: &Structure<'_>) -> Result<DbusValue, String> {
    value
        .fields()
        .iter()
        .map(dynamic_value)
        .collect::<Result<Vec<_>, _>>()
        .map(DbusValue::List)
}

fn array_value(value: &Array<'_>) -> Result<DbusValue, String> {
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
    events: mpsc::Receiver<zbus::Message>,
    connection: Option<DbusConnection>,
    join: Option<JoinHandle<()>>,
}

impl DbusSignal {
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
        if let Some(connection) = self.connection.take() {
            let _ = connection.close();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn basic_value(value: &OwnedValue) -> Result<DbusValue, String> {
    if matches!(
        &**value,
        Value::Array(_) | Value::Dict(_) | Value::Structure(_) | Value::Value(_)
    ) {
        return dynamic_value(value);
    }
    if let Ok(value) = bool::try_from(value) {
        return Ok(DbusValue::Bool(value));
    }
    if let Ok(value) = i16::try_from(value) {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = i32::try_from(value) {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = i64::try_from(value) {
        return Ok(DbusValue::Integer(value));
    }
    if let Ok(value) = u8::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u16::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u32::try_from(value) {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = u64::try_from(value) {
        return Ok(DbusValue::Unsigned(value));
    }
    if let Ok(value) = f64::try_from(value) {
        return Ok(DbusValue::Number(value));
    }
    if let Ok(value) = <&str>::try_from(value) {
        return Ok(DbusValue::String(value.to_owned()));
    }
    Err("D-Bus value is not a supported scalar".to_owned())
}

