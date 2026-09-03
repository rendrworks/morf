//! `morf.state(table)`: a table whose fields are reactive.
//!
//! A signal holds one scalar, so state that is a shape -- a screen, a
//! selection, a list of rows -- was a bag of separately named signals
//! written from every handler. This is the same graph with the shape kept:
//! each named field is its own signal, read through the proxy and tracked
//! by whatever binding reads it; a nested table is a nested proxy; an array
//! is a list model, the thing a `Repeater` follows. Writes inside a handler
//! are applied and flushed once when the handler returns, so an `update`
//! that touches five fields is one pass over the bindings.

use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use morf_scene::{ListModel, Value as SceneValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    reactive_bindings::*, runtime_helpers::*, scene_bindings::*, state::*, surface_types::*,
    types::*,
};

/// Whether a Lua table is an array: every key a positive integer, or none.
fn is_array<'gc>(ctx: Context<'gc>, table: Table<'gc>) -> bool {
    table
        .iter(ctx)
        .all(|(key, _)| matches!(key, LuaValue::Integer(index) if index >= 1))
}

fn build<'gc>(
    ctx: Context<'gc>,
    state: &Rc<RefCell<ReactiveState>>,
    metatable: &luna::StashedTable,
    path: &str,
    reloadable: Option<&str>,
    seed: Table<'gc>,
) -> Result<UserData<'gc>, String> {
    let mut fields = StateFields::default();
    for (key, value) in seed.iter(ctx) {
        let LuaValue::String(key) = key else {
            return Err(format!(
                "state fields must be named: `{path}` has a key that is not a string"
            ));
        };
        let key = key.display_lossy().to_string();
        let name = format!("{path}.{key}");
        match value {
            LuaValue::Table(table) if is_array(ctx, table) => {
                let SceneValue::List(values) = lua_to_scene(ctx, LuaValue::Table(table), 0)? else {
                    return Err(format!("state list `{name}` could not be read"));
                };
                let model = Rc::new(RefCell::new(ListModel::new(values)));
                let model_metatable = state
                    .borrow()
                    .model_metatable
                    .clone()
                    .ok_or("list models are not installed")?;
                let userdata = UserData::new_static(
                    &ctx,
                    ListModelToken {
                        model: Rc::clone(&model),
                    },
                );
                userdata.set_metatable(ctx, Some(ctx.fetch(&model_metatable)));
                fields.lists.insert(key, (ctx.stash(userdata), model));
            }
            LuaValue::Table(table) => {
                let child_reload = reloadable.map(|prefix| format!("{prefix}.{key}"));
                let child = build(ctx, state, metatable, &name, child_reload.as_deref(), table)?;
                fields.tables.insert(key, ctx.stash(child));
            }
            LuaValue::Function(_) | LuaValue::UserData(_) => {
                return Err(format!(
                    "state field `{name}` must be a value, a table or a list"
                ));
            }
            scalar => {
                let value = IpcValue::from_lua(scalar)?;
                let mut state = state.borrow_mut();
                let id = match reloadable {
                    // Named, and so kept across a configuration reload like
                    // a `morf.reloadable` signal is.
                    Some(prefix) => {
                        register_reloadable_value(&mut state, format!("{prefix}.{key}"), value)?.0
                    }
                    None => {
                        let id = state
                            .graph
                            .as_mut()
                            .ok_or("reactive graph is already running")?
                            .signal(name, value.clone());
                        state.values.insert(id, value);
                        state.signals.push(id);
                        id
                    }
                };
                fields.scalars.insert(key, id);
            }
        }
    }
    let userdata = UserData::new_static(
        &ctx,
        StateToken {
            fields: Rc::new(RefCell::new(fields)),
        },
    );
    userdata.set_metatable(ctx, Some(ctx.fetch(metatable)));
    Ok(userdata)
}

pub(crate) fn install_state_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
    limits: Limits,
) {
    let index = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (token, key): (UserRef<StateToken>, String) = stack.consume(ctx)?;
            let fields = token.fields.borrow();
            if let Some(&id) = fields.scalars.get(&key) {
                let mut state = state.borrow_mut();
                let value = if let Some(active) = &mut state.active {
                    active.reads.insert(id);
                    active
                        .writes
                        .iter()
                        .rev()
                        .find(|(signal, _)| *signal == id)
                        .map(|(_, value)| value.clone())
                        .or_else(|| state.values.get(&id).cloned())
                } else {
                    state.values.get(&id).cloned()
                }
                .ok_or_else(|| HostError("stale state field".to_owned()))?;
                stack.replace(ctx, value.to_lua(ctx));
                return Ok(CallbackReturn::Return);
            }
            if let Some(child) = fields.tables.get(&key) {
                stack.replace(ctx, ctx.fetch(child));
                return Ok(CallbackReturn::Return);
            }
            if let Some((list, _)) = fields.lists.get(&key) {
                stack.replace(ctx, ctx.fetch(list));
                return Ok(CallbackReturn::Return);
            }
            Err(HostError(format!("unknown state field `{key}`")).into())
        }
    });
    let new_index = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (token, key, value): (UserRef<StateToken>, String, LuaValue) =
                stack.consume(ctx)?;
            let fields = token.fields.borrow();
            if let Some(&id) = fields.scalars.get(&key) {
                let value = IpcValue::from_lua(value).map_err(HostError)?;
                {
                    let mut state = state.borrow_mut();
                    if let Some(active) = &mut state.active {
                        active.writes.push((id, value));
                        return Ok(CallbackReturn::Return);
                    }
                    state
                        .graph
                        .as_mut()
                        .ok_or_else(|| HostError("reactive graph is already running".into()))?
                        .write(id, value.clone())
                        .map_err(|error| HostError(error.to_string()))?;
                    state.values.insert(id, value);
                    if state.handler_depth > 0 {
                        state.flush_pending = true;
                        return Ok(CallbackReturn::Return);
                    }
                }
                flush_reactive(&state, ctx, limits).map_err(HostError)?;
                return Ok(CallbackReturn::Return);
            }
            if let Some((_, model)) = fields.lists.get(&key) {
                // A whole list assigned replaces the model's rows, matched
                // by value; `state.items:replace(rows, "id")` matches by key.
                let LuaValue::Table(table) = value else {
                    return Err(HostError(format!("state list `{key}` takes a table")).into());
                };
                let SceneValue::List(values) =
                    lua_to_scene(ctx, LuaValue::Table(table), 0).map_err(HostError)?
                else {
                    return Err(HostError(format!("state list `{key}` takes an array")).into());
                };
                model.borrow_mut().reconcile(values, None);
                let mut state = state.borrow_mut();
                state.scene_revision = state.scene_revision.wrapping_add(1);
                return Ok(CallbackReturn::Return);
            }
            if fields.tables.contains_key(&key) {
                return Err(HostError(format!(
                    "state table `{key}` is assigned a field at a time, not whole"
                ))
                .into());
            }
            Err(HostError(format!("unknown state field `{key}`")).into())
        }
    });
    let metatable = Table::new(&ctx);
    metatable.set_field(ctx, "__index", index);
    metatable.set_field(ctx, "__newindex", new_index);
    let metatable = ctx.stash(metatable);

    let create = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            // `morf.state(table, { reloadable = "name" })` keeps the scalar
            // fields across a reload under that name; lists start afresh.
            let (seed, options): (Table, Option<Table>) = stack.consume(ctx)?;
            let reloadable =
                options.and_then(|options| match options.get_value(ctx, "reloadable") {
                    LuaValue::String(name) => Some(name.display_lossy().to_string()),
                    _ => None,
                });
            let userdata = build(
                ctx,
                &state,
                &metatable,
                "state",
                reloadable.as_deref(),
                seed,
            )
            .map_err(HostError)?;
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    morf.set_field(ctx, "state", create);
}
