fn dbus_argument_value(value: &DbusValue) -> Result<Value<'_>, String> {
    match value {
        DbusValue::Bool(value) => Ok(Value::Bool(*value)),
        DbusValue::Integer(value) => Ok(Value::I64(*value)),
        DbusValue::Unsigned(value) => Ok(Value::U64(*value)),
        DbusValue::Number(value) => Ok(Value::F64(*value)),
        DbusValue::String(value) => Ok(Value::Str(value.as_str().into())),
        DbusValue::Typed { signature, value } => typed_dbus_value(signature, value),
        DbusValue::Nil => Err("nil cannot be a positional D-Bus argument".to_owned()),
        DbusValue::List(_) | DbusValue::Map(_) => {
            Err("nested D-Bus arguments need an explicit signature".to_owned())
        }
    }
}

fn typed_dbus_value<'a>(signature: &str, value: &'a DbusValue) -> Result<Value<'a>, String> {
    let signature = Signature::try_from(signature)
        .map_err(|error| format!("invalid D-Bus signature: {error}"))?;
    dbus_value_for_signature(&signature, value)
}

fn dbus_value_for_signature<'a>(
    signature: &Signature,
    value: &'a DbusValue,
) -> Result<Value<'a>, String> {
    let name = signature.to_string();
    let integer = || match value {
        DbusValue::Integer(value) => Ok(i128::from(*value)),
        DbusValue::Unsigned(value) => Ok(i128::from(*value)),
        _ => Err(format!("D-Bus `{name}` value must be an integer")),
    };
    let range_error = || format!("D-Bus `{name}` integer is out of range");
    Ok(match signature {
        Signature::U8 => Value::U8(u8::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I16 => Value::I16(i16::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U16 => Value::U16(u16::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I32 => Value::I32(i32::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U32 => Value::U32(u32::try_from(integer()?).map_err(|_| range_error())?),
        Signature::I64 => Value::I64(i64::try_from(integer()?).map_err(|_| range_error())?),
        Signature::U64 => Value::U64(u64::try_from(integer()?).map_err(|_| range_error())?),
        Signature::F64 => match value {
            DbusValue::Number(value) => Value::F64(*value),
            DbusValue::Integer(value) => Value::F64(*value as f64),
            DbusValue::Unsigned(value) => Value::F64(*value as f64),
            _ => return Err("D-Bus `d` value must be numeric".to_owned()),
        },
        Signature::Bool => match value {
            DbusValue::Bool(value) => Value::Bool(*value),
            _ => return Err("D-Bus `b` value must be boolean".to_owned()),
        },
        Signature::Str => match value {
            DbusValue::String(value) => Value::Str(value.as_str().into()),
            _ => return Err("D-Bus `s` value must be a string".to_owned()),
        },
        Signature::ObjectPath => match value {
            DbusValue::String(value) => Value::ObjectPath(
                ObjectPath::try_from(value.as_str()).map_err(|error| error.to_string())?,
            ),
            _ => return Err("D-Bus `o` value must be a string".to_owned()),
        },
        Signature::Signature => match value {
            DbusValue::String(value) => Value::Signature(
                Signature::try_from(value.as_str()).map_err(|error| error.to_string())?,
            ),
            _ => return Err("D-Bus `g` value must be a string".to_owned()),
        },
        Signature::Variant => Value::Value(Box::new(inferred_dbus_value(value)?)),
        Signature::Array(child) => {
            let DbusValue::List(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a list"));
            };
            let mut array = Array::new(child.signature());
            for value in values {
                array
                    .append(dbus_value_for_signature(child.signature(), value)?)
                    .map_err(|error| error.to_string())?;
            }
            Value::Array(array)
        }
        Signature::Dict {
            key: key_signature,
            value: value_signature,
        } => {
            let DbusValue::Map(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a map"));
            };
            let mut dict = Dict::new(key_signature.signature(), value_signature.signature());
            for (key, value) in values {
                dict.append(
                    dbus_map_key(key_signature.signature(), key)?,
                    dbus_value_for_signature(value_signature.signature(), value)?,
                )
                .map_err(|error| error.to_string())?;
            }
            Value::Dict(dict)
        }
        Signature::Structure(fields) => {
            let DbusValue::List(values) = value else {
                return Err(format!("D-Bus `{name}` value must be a list"));
            };
            if values.len() != fields.len() {
                return Err(format!(
                    "D-Bus `{name}` needs {} fields, found {}",
                    fields.len(),
                    values.len()
                ));
            }
            let mut structure = StructureBuilder::new();
            for (field, value) in fields.iter().zip(values) {
                structure = structure.append_field(dbus_value_for_signature(field, value)?);
            }
            Value::Structure(structure.build().map_err(|error| error.to_string())?)
        }
        Signature::Unit => return Err("D-Bus unit values cannot be arguments".to_owned()),
        #[cfg(unix)]
        Signature::Fd => return Err("D-Bus file descriptors cannot come from Lua".to_owned()),
        #[allow(unreachable_patterns)]
        _ => return Err(format!("unsupported explicit D-Bus signature `{name}`")),
    })
}

fn dbus_map_key<'a>(signature: &Signature, key: &'a str) -> Result<Value<'a>, String> {
    match signature {
        Signature::Str => Ok(Value::Str(key.into())),
        Signature::ObjectPath => Ok(Value::ObjectPath(
            ObjectPath::try_from(key).map_err(|error| error.to_string())?,
        )),
        Signature::Signature => Ok(Value::Signature(
            Signature::try_from(key).map_err(|error| error.to_string())?,
        )),
        _ => Err(format!(
            "D-Bus map keys from Lua cannot use signature `{signature}`"
        )),
    }
}

fn inferred_dbus_value(value: &DbusValue) -> Result<Value<'_>, String> {
    match value {
        DbusValue::Typed { signature, value } => typed_dbus_value(signature, value),
        DbusValue::Bool(value) => Ok(Value::Bool(*value)),
        DbusValue::Integer(value) => Ok(Value::I64(*value)),
        DbusValue::Unsigned(value) => Ok(Value::U64(*value)),
        DbusValue::Number(value) => Ok(Value::F64(*value)),
        DbusValue::String(value) => Ok(Value::Str(value.as_str().into())),
        DbusValue::Nil => Err("nil cannot be a D-Bus variant".to_owned()),
        DbusValue::List(_) | DbusValue::Map(_) => {
            Err("compound D-Bus variants need an explicit signature".to_owned())
        }
    }
}

fn decode_message_value(message: &zbus::Message) -> Result<DbusValue, String> {
    let body = message.body();
    if body.deserialize::<()>().is_ok() {
        return Ok(DbusValue::Nil);
    }
    if let Ok(value) = body.deserialize::<bool>() {
        return Ok(DbusValue::Bool(value));
    }
    if let Ok(value) = body.deserialize::<i16>() {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = body.deserialize::<i32>() {
        return Ok(DbusValue::Integer(value as i64));
    }
    if let Ok(value) = body.deserialize::<i64>() {
        return Ok(DbusValue::Integer(value));
    }
    if let Ok(value) = body.deserialize::<u8>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u16>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u32>() {
        return Ok(DbusValue::Unsigned(value as u64));
    }
    if let Ok(value) = body.deserialize::<u64>() {
        return Ok(DbusValue::Unsigned(value));
    }
    if let Ok(value) = body.deserialize::<f64>() {
        return Ok(DbusValue::Number(value));
    }
    if let Ok(value) = body.deserialize::<String>() {
        return Ok(DbusValue::String(value));
    }
    if let Ok(value) = body.deserialize::<Structure<'_>>() {
        return structure_value(&value);
    }
    if let Ok(value) = body.deserialize::<Array<'_>>() {
        return array_value(&value);
    }
    Err("D-Bus reply type is not supported".to_owned())
}

