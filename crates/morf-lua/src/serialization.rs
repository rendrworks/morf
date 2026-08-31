use luna::{Closure, Context, Executor, Table, UserData, Value as LuaValue};
use morf_io::DbusValue;
use morf_scene::{Value as SceneValue, ViewTransition};
use morf_services::XkbKeymap;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::{reactive_execute::*, scene_bindings::*, state::*, types::*};

pub(crate) fn default_module_roots() -> Vec<PathBuf> {
    std::env::var_os("MORF_RUNTIME_PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .collect()
}

pub(crate) fn load_runtime_module(roots: &[PathBuf], name: &str) -> Result<Vec<u8>, String> {
    if name.is_empty()
        || name.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
    {
        return Err(format!("invalid module name `{name}`"));
    }
    let relative = name.replace('.', "/");
    for root in roots {
        for path in [
            root.join(format!("{relative}.lua")),
            root.join(&relative).join("init.lua"),
            root.join("lua").join(format!("{relative}.lua")),
            root.join("lua").join(&relative).join("init.lua"),
        ] {
            match fs::read(&path) {
                Ok(source) => return Ok(source),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("could not read {}: {error}", path.display())),
            }
        }
    }
    Err(format!("module `{name}` is not available"))
}

pub(crate) fn json_to_lua<'gc>(
    ctx: Context<'gc>,
    value: &serde_json::Value,
    array_metatable: Table<'gc>,
    object_metatable: Table<'gc>,
    null: UserData<'gc>,
    depth: usize,
    entries: &mut usize,
) -> Result<LuaValue<'gc>, String> {
    if depth > 64 {
        return Err("JSON value exceeds maximum depth 64".to_owned());
    }
    *entries += 1;
    if *entries > 65_536 {
        return Err("JSON value exceeds 65536 entries".to_owned());
    }
    Ok(match value {
        serde_json::Value::Null => LuaValue::UserData(null),
        serde_json::Value::Bool(value) => LuaValue::Boolean(*value),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                LuaValue::Integer(value)
            } else if let Some(value) = value.as_u64().and_then(|value| i64::try_from(value).ok()) {
                LuaValue::Integer(value)
            } else {
                LuaValue::Number(
                    value
                        .as_f64()
                        .ok_or_else(|| "JSON number is not representable".to_owned())?,
                )
            }
        }
        serde_json::Value::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        serde_json::Value::Array(values) => {
            let table = Table::new(&ctx);
            table.set_metatable(ctx, Some(array_metatable));
            for (index, value) in values.iter().enumerate() {
                table
                    .set(
                        ctx,
                        index as i64 + 1,
                        json_to_lua(
                            ctx,
                            value,
                            array_metatable,
                            object_metatable,
                            null,
                            depth + 1,
                            entries,
                        )?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        serde_json::Value::Object(values) => {
            let table = Table::new(&ctx);
            table.set_metatable(ctx, Some(object_metatable));
            for (key, value) in values {
                table
                    .set(
                        ctx,
                        ctx.intern(key.as_bytes()),
                        json_to_lua(
                            ctx,
                            value,
                            array_metatable,
                            object_metatable,
                            null,
                            depth + 1,
                            entries,
                        )?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
    })
}

pub(crate) fn lua_to_json<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
    depth: usize,
    entries: &mut usize,
) -> Result<serde_json::Value, String> {
    if depth > 64 {
        return Err("JSON value exceeds maximum depth 64".to_owned());
    }
    *entries += 1;
    if *entries > 65_536 {
        return Err("JSON value exceeds 65536 entries".to_owned());
    }
    match value {
        LuaValue::Nil => Ok(serde_json::Value::Null),
        LuaValue::Boolean(value) => Ok(serde_json::Value::Bool(value)),
        LuaValue::Integer(value) => Ok(serde_json::Value::Number(value.into())),
        LuaValue::Number(value) if value.is_finite() => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "JSON number is not representable".to_owned()),
        LuaValue::String(value) => Ok(serde_json::Value::String(value.display_lossy().to_string())),
        LuaValue::UserData(value) if value.is_static::<JsonNullToken>() => {
            Ok(serde_json::Value::Null)
        }
        LuaValue::Table(table) => {
            let kind = table.metatable().and_then(|metatable| {
                let LuaValue::String(kind) = metatable.get_value(ctx, "__json_kind") else {
                    return None;
                };
                Some(kind.display_lossy().to_string())
            });
            let values = table.iter(ctx).collect::<Vec<_>>();
            let is_array = match kind.as_deref() {
                Some("array") => true,
                Some("object") => false,
                Some(_) => return Err("unknown JSON table kind".to_owned()),
                None => {
                    !values.is_empty()
                        && values
                            .iter()
                            .all(|(key, _)| matches!(key, LuaValue::Integer(_)))
                }
            };
            if is_array {
                let mut values = values
                    .into_iter()
                    .map(|(key, value)| {
                        let LuaValue::Integer(index) = key else {
                            return Err("JSON array keys must be integers".to_owned());
                        };
                        Ok((index, lua_to_json(ctx, value, depth + 1, entries)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                values.sort_by_key(|(index, _)| *index);
                for (offset, (index, _)) in values.iter().enumerate() {
                    if *index != offset as i64 + 1 {
                        return Err("JSON arrays must be dense sequences".to_owned());
                    }
                }
                Ok(serde_json::Value::Array(
                    values.into_iter().map(|(_, value)| value).collect(),
                ))
            } else {
                let mut object = serde_json::Map::new();
                for (key, value) in values {
                    let LuaValue::String(key) = key else {
                        return Err("JSON object keys must be strings".to_owned());
                    };
                    object.insert(
                        key.display_lossy().to_string(),
                        lua_to_json(ctx, value, depth + 1, entries)?,
                    );
                }
                Ok(serde_json::Value::Object(object))
            }
        }
        value => Err(format!(
            "JSON does not support Lua {} values",
            value.type_name()
        )),
    }
}

pub(crate) fn dbus_value_to_lua(
    ctx: Context<'_>,
    value: DbusValue,
) -> Result<LuaValue<'_>, String> {
    Ok(match value {
        DbusValue::Nil => LuaValue::Nil,
        DbusValue::Bool(value) => LuaValue::Boolean(value),
        DbusValue::Integer(value) => LuaValue::Integer(value),
        DbusValue::Unsigned(value) if value <= i64::MAX as u64 => LuaValue::Integer(value as i64),
        DbusValue::Unsigned(value) => LuaValue::Number(value as f64),
        DbusValue::Number(value) => LuaValue::Number(value),
        DbusValue::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        DbusValue::List(values) => {
            let table = Table::new(&ctx);
            for (index, value) in values.into_iter().enumerate() {
                table
                    .set(ctx, index as i64 + 1, dbus_value_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        DbusValue::Map(values) => {
            let table = Table::new(&ctx);
            for (key, value) in values {
                table
                    .set(
                        ctx,
                        ctx.intern(key.as_bytes()),
                        dbus_value_to_lua(ctx, value)?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        DbusValue::Typed { signature, value } => {
            let table = Table::new(&ctx);
            table.set_field(ctx, "signature", signature.as_str());
            table.set_field(ctx, "value", dbus_value_to_lua(ctx, *value)?);
            LuaValue::Table(table)
        }
    })
}

pub(crate) fn lua_to_dbus<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
    depth: usize,
) -> Result<DbusValue, String> {
    if depth > 8 {
        return Err("D-Bus value exceeds maximum depth 8".to_owned());
    }
    match value {
        LuaValue::Nil => Ok(DbusValue::Nil),
        LuaValue::Boolean(value) => Ok(DbusValue::Bool(value)),
        LuaValue::Integer(value) => Ok(DbusValue::Integer(value)),
        LuaValue::Number(value) if value.is_finite() => Ok(DbusValue::Number(value)),
        LuaValue::String(value) => Ok(DbusValue::String(value.display_lossy().to_string())),
        LuaValue::Table(table) => {
            if let LuaValue::String(signature) = table.get_value(ctx, "signature") {
                let value = table.get_value(ctx, "value");
                return Ok(DbusValue::Typed {
                    signature: signature.display_lossy().to_string(),
                    value: Box::new(lua_to_dbus(ctx, value, depth + 1)?),
                });
            }
            let entries = table.iter(ctx).collect::<Vec<_>>();
            if entries.len() > 256 {
                return Err("D-Bus table exceeds 256 entries".to_owned());
            }
            if entries.is_empty()
                || entries
                    .iter()
                    .all(|(key, _)| matches!(key, LuaValue::Integer(_)))
            {
                let mut values = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let LuaValue::Integer(index) = key else {
                            unreachable!()
                        };
                        Ok((index, lua_to_dbus(ctx, value, depth + 1)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                values.sort_by_key(|(index, _)| *index);
                for (offset, (index, _)) in values.iter().enumerate() {
                    if *index != offset as i64 + 1 {
                        return Err("D-Bus list must be a dense sequence".to_owned());
                    }
                }
                Ok(DbusValue::List(
                    values.into_iter().map(|(_, value)| value).collect(),
                ))
            } else if entries
                .iter()
                .all(|(key, _)| matches!(key, LuaValue::String(_)))
            {
                let mut values = BTreeMap::new();
                for (key, value) in entries {
                    let LuaValue::String(key) = key else {
                        unreachable!()
                    };
                    values.insert(
                        key.display_lossy().to_string(),
                        lua_to_dbus(ctx, value, depth + 1)?,
                    );
                }
                Ok(DbusValue::Map(values))
            } else {
                Err("D-Bus table keys must be all integers or all strings".to_owned())
            }
        }
        _ => Err("unsupported D-Bus value".to_owned()),
    }
}

pub(crate) fn lua_index(index: i64) -> Result<usize, HostError> {
    let index = index
        .checked_sub(1)
        .ok_or_else(|| HostError("list-model indexes start at one".into()))?;
    usize::try_from(index).map_err(|_| HostError("list-model index is out of range".into()))
}

pub(crate) fn lua_insert_index(index: i64, length: usize) -> Result<usize, HostError> {
    if index == length as i64 + 1 {
        Ok(length)
    } else {
        lua_index(index)
    }
}

pub(crate) fn scene_to_lua<'gc>(
    ctx: Context<'gc>,
    value: &SceneValue,
) -> Result<LuaValue<'gc>, String> {
    Ok(match value {
        SceneValue::Nil => LuaValue::Nil,
        SceneValue::Bool(value) => LuaValue::Boolean(*value),
        SceneValue::Number(value) => LuaValue::Number(*value),
        SceneValue::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        SceneValue::Color(color) => {
            let table = Table::new(&ctx);
            table.set_field(ctx, "r", color.red as f64);
            table.set_field(ctx, "g", color.green as f64);
            table.set_field(ctx, "b", color.blue as f64);
            table.set_field(ctx, "a", color.alpha as f64);
            LuaValue::Table(table)
        }
        SceneValue::List(values) => {
            let table = Table::new(&ctx);
            for (index, value) in values.iter().enumerate() {
                table
                    .set(ctx, index as i64 + 1, scene_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        SceneValue::Map(values) => {
            let table = Table::new(&ctx);
            for (key, value) in values {
                table
                    .set(ctx, ctx.intern(key.as_bytes()), scene_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
    })
}

pub(crate) fn xkb_keymap_to_lua<'gc>(ctx: Context<'gc>, keymap: &XkbKeymap) -> Table<'gc> {
    let result = Table::new(&ctx);
    result.set_field(ctx, "source", keymap.source.as_str());
    let keys = Table::new(&ctx);
    for (key_index, key) in keymap.keys.iter().enumerate() {
        let value = Table::new(&ctx);
        value.set_field(ctx, "keycode", i64::from(key.keycode));
        value.set_field(ctx, "evdev_code", i64::from(key.evdev_code));
        value.set_field(ctx, "name", key.name.as_str());
        value.set_field(ctx, "repeats", key.repeats);
        let layouts = Table::new(&ctx);
        for (layout_index, layout) in key.layouts.iter().enumerate() {
            let levels = Table::new(&ctx);
            for (level_index, level) in layout.iter().enumerate() {
                let symbols = Table::new(&ctx);
                for (symbol_index, symbol) in level.iter().enumerate() {
                    let item = Table::new(&ctx);
                    item.set_field(ctx, "keysym", i64::from(symbol.keysym));
                    item.set_field(ctx, "name", symbol.name.as_str());
                    item.set_field(ctx, "text", symbol.text.as_str());
                    symbols
                        .set(ctx, symbol_index as i64 + 1, item)
                        .expect("XKB symbol table accepts integer keys");
                }
                levels
                    .set(ctx, level_index as i64 + 1, symbols)
                    .expect("XKB level table accepts integer keys");
            }
            layouts
                .set(ctx, layout_index as i64 + 1, levels)
                .expect("XKB layout table accepts integer keys");
        }
        value.set_field(ctx, "layouts", layouts);
        keys.set(ctx, key_index as i64 + 1, value)
            .expect("XKB key table accepts integer keys");
    }
    result.set_field(ctx, "keys", keys);
    result
}

pub(crate) fn view_transition_to_lua(ctx: Context<'_>, transition: ViewTransition) -> Table<'_> {
    let table = Table::new(&ctx);
    let (kind, item, from, targets) = match transition {
        ViewTransition::Populate(item) => ("populate", item, None, Vec::new()),
        ViewTransition::Add(item) => ("add", item, None, Vec::new()),
        ViewTransition::Remove(item) => ("remove", item, None, Vec::new()),
        ViewTransition::Move {
            item,
            from,
            target_indexes,
        } => ("move", item, Some(from), target_indexes),
        ViewTransition::Displaced {
            item,
            from,
            target_indexes,
        } => ("displaced", item, Some(from), target_indexes),
    };
    table.set_field(ctx, "kind", kind);
    table.set_field(ctx, "id", item.id.raw() as i64);
    table.set_field(ctx, "index", item.index as i64 + 1);
    table.set_field(ctx, "destination", item.destination);
    table.set_field(
        ctx,
        "from",
        from.map_or(LuaValue::Nil, |index| LuaValue::Integer(index as i64 + 1)),
    );
    let target_indexes = Table::new(&ctx);
    for (index, target) in targets.into_iter().enumerate() {
        target_indexes
            .set(ctx, index as i64 + 1, target as i64 + 1)
            .expect("target-index table accepts integer keys");
    }
    table.set_field(ctx, "target_indexes", target_indexes);
    table
}

pub(crate) fn execute_module<'gc>(
    ctx: Context<'gc>,
    name: &str,
    source: &[u8],
    limits: Limits,
) -> Result<LuaValue<'gc>, String> {
    let closure = Closure::load(ctx, Some(name), source).map_err(|error| error.to_string())?;
    let executor = Executor::start(ctx, closure.into(), ());
    drive_executor(ctx, executor, limits, limits.effect_fuel, "module")?;
    match executor.take_result::<LuaValue>(ctx) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}
