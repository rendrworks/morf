use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    reactive_bindings::*, runtime_helpers::*, scene_bindings::*, state::*, surface_types::*,
    types::*,
};

pub(crate) fn install_signal_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
    limits: Limits,
) {
    let get = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let signal: UserRef<SignalToken> = stack.consume(ctx)?;
            let mut state = state.borrow_mut();
            let value = if let Some(active) = &mut state.active {
                active.reads.insert(signal.id);
                active
                    .writes
                    .iter()
                    .rev()
                    .find(|(id, _)| *id == signal.id)
                    .map(|(_, value)| value.clone())
                    .or_else(|| state.values.get(&signal.id).cloned())
            } else {
                state.values.get(&signal.id).cloned()
            }
            .ok_or_else(|| HostError("stale reactive signal".to_owned()))?;
            stack.replace(ctx, value.to_lua(ctx));
            Ok(CallbackReturn::Return)
        }
    });

    let set = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (signal, value): (UserRef<SignalToken>, LuaValue) = stack.consume(ctx)?;
            let value = IpcValue::from_lua(value).map_err(HostError)?;
            {
                let mut state = state.borrow_mut();
                if let Some(active) = &mut state.active {
                    active.writes.push((signal.id, value));
                    stack.replace(ctx, true);
                    return Ok(CallbackReturn::Return);
                }
                let graph = state
                    .graph
                    .as_mut()
                    .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?;
                graph
                    .write(signal.id, value.clone())
                    .map_err(|error| HostError(error.to_string()))?;
                state.values.insert(signal.id, value);
                // Inside a handler the write is enough: the graph is flushed
                // once, when the handler returns, however many writes it made.
                if state.handler_depth > 0 {
                    state.flush_pending = true;
                    stack.replace(ctx, true);
                    return Ok(CallbackReturn::Return);
                }
            }
            replace_status(ctx, &mut stack, flush_reactive(&state, ctx, limits));
            Ok(CallbackReturn::Return)
        }
    });

    let methods = Table::new(&ctx);
    methods.set_field(ctx, "get", get);
    methods.set_field(ctx, "set", set);
    let signal_metatable = Table::new(&ctx);
    signal_metatable.set_field(ctx, "__index", methods);
    let signal_metatable = ctx.stash(signal_metatable);

    let signal = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let signal_metatable = signal_metatable.clone();
        move |ctx, _, mut stack| {
            let (name, value): (String, LuaValue) = stack.consume(ctx)?;
            let value = IpcValue::from_lua(value).map_err(HostError)?;
            let id = {
                let mut state = state.borrow_mut();
                let id = state
                    .graph
                    .as_mut()
                    .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?
                    .signal(name, value.clone());
                state.values.insert(id, value);
                state.signals.push(id);
                id
            };
            let userdata = UserData::new_static(&ctx, SignalToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });

    let reloadable = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let signal_metatable = signal_metatable.clone();
        move |ctx, _, mut stack| {
            let (name, initial): (String, LuaValue) = stack.consume(ctx)?;
            let initial = IpcValue::from_lua(initial).map_err(HostError)?;
            let (id, _) = register_reloadable_value(&mut state.borrow_mut(), name, initial)
                .map_err(HostError)?;
            let userdata = UserData::new_static(&ctx, SignalToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });

    let persistent_index = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (persistent, key): (UserRef<PersistentToken>, String) = stack.consume(ctx)?;
            if key == "loaded" {
                stack.replace(ctx, true);
                return Ok(CallbackReturn::Return);
            }
            if key == "reloaded" {
                stack.replace(ctx, persistent.reloaded);
                return Ok(CallbackReturn::Return);
            }
            let id = persistent
                .properties
                .get(&key)
                .copied()
                .ok_or_else(|| HostError(format!("unknown persistent property `{key}`")))?;
            let value = {
                let mut state = state.borrow_mut();
                if let Some(active) = &mut state.active {
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
                .ok_or_else(|| HostError("stale persistent property".into()))?
            };
            stack.replace(ctx, value.to_lua(ctx));
            Ok(CallbackReturn::Return)
        }
    });
    let persistent_new_index = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (persistent, key, value): (UserRef<PersistentToken>, String, LuaValue) =
                stack.consume(ctx)?;
            if matches!(key.as_str(), "loaded" | "reloaded") {
                return Err(HostError(format!("persistent property `{key}` is read-only")).into());
            }
            let id = persistent
                .properties
                .get(&key)
                .copied()
                .ok_or_else(|| HostError(format!("unknown persistent property `{key}`")))?;
            let value = IpcValue::from_lua(value).map_err(HostError)?;
            {
                let mut state = state.borrow_mut();
                let current = state
                    .values
                    .get(&id)
                    .ok_or_else(|| HostError("stale persistent property".into()))?;
                if std::mem::discriminant(current) != std::mem::discriminant(&value) {
                    return Err(HostError(format!(
                        "persistent property `{key}` cannot change value type"
                    ))
                    .into());
                }
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
            }
            replace_status(ctx, &mut stack, flush_reactive(&state, ctx, limits));
            Ok(CallbackReturn::Return)
        }
    });
    let persistent_metatable = Table::new(&ctx);
    persistent_metatable.set_field(ctx, "__index", persistent_index);
    persistent_metatable.set_field(ctx, "__newindex", persistent_new_index);
    let persistent_metatable = ctx.stash(persistent_metatable);
    let persistent = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let persistent_metatable = persistent_metatable.clone();
        move |ctx, _, mut stack| {
            let (name, defaults): (String, Table) = stack.consume(ctx)?;
            let token = create_persistent_token(ctx, &state, &name, defaults).map_err(HostError)?;
            let userdata = UserData::new_static(&ctx, token);
            userdata.set_metatable(ctx, Some(ctx.fetch(&persistent_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });

    let scope_reloadable = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let signal_metatable = signal_metatable.clone();
        move |ctx, _, mut stack| {
            let (scope, name, initial): (UserRef<ScopeToken>, String, LuaValue) =
                stack.consume(ctx)?;
            let name = scoped_id(&scope.prefix, &name).map_err(HostError)?;
            let initial = IpcValue::from_lua(initial).map_err(HostError)?;
            let (id, _) = register_reloadable_value(&mut state.borrow_mut(), name, initial)
                .map_err(HostError)?;
            let userdata = UserData::new_static(&ctx, SignalToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    let scope_persistent = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let persistent_metatable = persistent_metatable.clone();
        move |ctx, _, mut stack| {
            let (scope, name, defaults): (UserRef<ScopeToken>, String, Table) =
                stack.consume(ctx)?;
            let name = scoped_id(&scope.prefix, &name).map_err(HostError)?;
            let token = create_persistent_token(ctx, &state, &name, defaults).map_err(HostError)?;
            let userdata = UserData::new_static(&ctx, token);
            userdata.set_metatable(ctx, Some(ctx.fetch(&persistent_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    let scope_id = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (scope, name): (UserRef<ScopeToken>, String) = stack.consume(ctx)?;
        stack.replace(ctx, scoped_id(&scope.prefix, &name).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let scope_methods = Table::new(&ctx);
    scope_methods.set_field(ctx, "reloadable", scope_reloadable);
    scope_methods.set_field(ctx, "persistent", scope_persistent);
    scope_methods.set_field(ctx, "id", scope_id);
    let scope_metatable = Table::new(&ctx);
    scope_metatable.set_field(ctx, "__index", scope_methods);
    let scope_metatable = ctx.stash(scope_metatable);
    let scope = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let prefix: String = stack.consume(ctx)?;
        validate_scope_part(&prefix).map_err(HostError)?;
        let userdata = UserData::new_static(&ctx, ScopeToken { prefix });
        userdata.set_metatable(ctx, Some(ctx.fetch(&scope_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "signal", signal);
    morf.set_field(ctx, "reloadable", reloadable);
    morf.set_field(ctx, "persistent", persistent);
    morf.set_field(ctx, "scope", scope);
    let clock = UserData::new_static(
        &ctx,
        SignalToken {
            id: state.borrow().clock,
        },
    );
    clock.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
    morf.set_field(ctx, "clock", clock);
}
