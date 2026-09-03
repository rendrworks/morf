use crate::states::Capture;
use luna::{Context, Executor, Fuel, StashedClosure, Table, Value as LuaValue, Variadic};
use morf_io::{DbusCall, DbusValue};
use morf_reactive::EffectContext;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use morf_services::{StatusNotifierAddress, UdevEvent};

use crate::{
    reactive_bindings::*, scene_bindings::*, serialization::*, state::*, surface_types::*, types::*,
};

/// Runs one Lua executor to completion, or stops it when its fuel runs out.
///
/// This is the sandbox's only real defence: untrusted configuration code gets a
/// fixed number of VM instructions and is cut off at the end of them. It used
/// to be written out at every place that ran Lua — ten copies — and two of them
/// had already drifted: one skipped the error mapping every other copy applied,
/// and only one debited the per-frame budget, so a construction that ran a
/// factory once per item could spend a full effect budget on each of them.
///
/// Returns the fuel actually spent, so a caller that shares a budget across
/// several runs can debit it.
pub(crate) fn drive_executor<'gc>(
    ctx: Context<'gc>,
    executor: Executor<'gc>,
    limits: Limits,
    budget: u64,
    what: &str,
) -> Result<u64, String> {
    if budget == 0 {
        return Err(format!("Lua {what} fuel exhausted"));
    }
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua {what} fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            return Ok(budget - remaining);
        }
    }
}

pub(crate) fn evaluate_effect(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    frame_remaining: &mut u64,
    token: u64,
    effect: &mut EffectContext<'_, IpcValue>,
) -> Result<(), String> {
    let lua_effect = {
        let mut state = state.borrow_mut();
        if state.active.is_some() {
            return Err("reactive effects cannot run recursively".to_owned());
        }
        state.active = Some(Capture::default());
        state.effect_runs = state.effect_runs.saturating_add(1);
        state
            .effects
            .get(&token)
            .cloned()
            .ok_or_else(|| format!("missing Lua closure for effect {token}"))?
    };
    let result = execute_effect(
        ctx,
        &lua_effect.closure,
        limits,
        frame_remaining,
        lua_effect.sink.is_some(),
    );
    let state_result = if let (Ok(Some(value)), Some(EffectSink::State(node))) =
        (&result, lua_effect.sink.clone())
    {
        match value {
            IpcValue::String(name) => apply_state(state, ctx, limits, frame_remaining, node, name),
            _ => Err("state binding must return a string".into()),
        }
    } else {
        Ok(())
    };
    let state_result = state_result.and_then(|()| {
        if let (Ok(Some(value)), Some(sink)) = (&result, lua_effect.sink) {
            match sink {
                EffectSink::Property(sink) => assign_scene_property(
                    &mut state.borrow_mut(),
                    sink.node,
                    &sink.property,
                    value.to_scene(),
                ),
                EffectSink::State(_) => Ok(()),
            }
        } else {
            Ok(())
        }
    });
    let capture = state.borrow_mut().active.take().unwrap_or_default();
    for (node, property, target) in capture.property_reads {
        let key = (node, property.clone(), target);
        let signal = if let Some(signal) = state.borrow().property_signals.get(&key).copied() {
            signal
        } else {
            let name = format!("{node:?}.{property}{}", if target { "_target" } else { "" });
            let value = IpcValue::Integer(state.borrow().property_revision);
            let signal = effect.signal(name.clone(), value.clone());
            let mut state = state.borrow_mut();
            state.property_signals.insert(key, signal);
            if !target {
                state.current_property_names.insert(name, (node, property));
            }
            state.values.insert(signal, value);
            state.signals.push(signal);
            signal
        };
        effect.get(signal).map_err(|error| error.to_string())?;
    }
    for signal in capture.reads {
        effect.get(signal).map_err(|error| error.to_string())?;
    }
    if result.is_ok() {
        for (signal, value) in capture.writes {
            effect
                .set(signal, value.clone())
                .map_err(|error| error.to_string())?;
            state.borrow_mut().values.insert(signal, value);
        }
    }
    state_result?;
    result.map(|_| ())
}

pub(crate) fn execute_effect(
    ctx: Context<'_>,
    closure: &StashedClosure,
    limits: Limits,
    frame_remaining: &mut u64,
    capture_value: bool,
) -> Result<Option<IpcValue>, String> {
    let budget = limits.effect_fuel.min(*frame_remaining);
    if budget == 0 {
        return Err("Lua frame fuel exhausted".to_owned());
    }
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), ());
    match drive_executor(ctx, executor, limits, budget, "effect") {
        Err(error) => {
            *frame_remaining = frame_remaining.saturating_sub(budget);
            Err(error)
        }
        Ok(spent) => {
            *frame_remaining = frame_remaining.saturating_sub(spent);
            if capture_value {
                match executor.take_result::<LuaValue>(ctx) {
                    Ok(Ok(value)) => IpcValue::from_lua(value).map(Some),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                }
            } else {
                match executor.take_result::<()>(ctx) {
                    Ok(Ok(())) => Ok(None),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                }
            }
        }
    }
}

pub(crate) fn execute_handler_args(
    ctx: Context<'_>,
    closure: &StashedClosure,
    args: &[IpcValue],
    limits: Limits,
) -> Result<(), String> {
    let args = Variadic(
        args.iter()
            .map(|value| value.to_lua(ctx))
            .collect::<Vec<_>>(),
    );
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    drive_executor(ctx, executor, limits, limits.effect_fuel, "handler")?;
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn execute_screencopy_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    result: Result<Screencopy, String>,
    limits: Limits,
) -> Result<(), String> {
    let args = match result {
        Ok(frame) => {
            let value = Table::new(&ctx);
            value.set_field(ctx, "width", i64::from(frame.width));
            value.set_field(ctx, "height", i64::from(frame.height));
            value.set_field(ctx, "stride", i64::from(frame.stride));
            value.set_field(ctx, "format", frame.format.as_str());
            value.set_field(ctx, "y_invert", frame.y_invert);
            value.set_field(ctx, "source", ctx.intern(frame.source.as_bytes()));
            value.set_field(ctx, "pixels", ctx.intern(&frame.pixels));
            Variadic(vec![LuaValue::Table(value), LuaValue::Nil])
        }
        Err(error) => Variadic(vec![
            LuaValue::Nil,
            LuaValue::String(ctx.intern(error.as_bytes())),
        ]),
    };
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    drive_executor(ctx, executor, limits, limits.effect_fuel, "handler")?;
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn execute_dbus_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    value: DbusValue,
    limits: Limits,
) -> Result<(), String> {
    let argument = dbus_value_to_lua(ctx, value)?;
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), Variadic(vec![argument]));
    drive_executor(ctx, executor, limits, limits.effect_fuel, "handler")?;
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Hands one arriving method call to its Lua handler.
///
/// The call arrives as a table rather than as positional arguments because most
/// handlers dispatch on `member` and ignore the rest, and a handler that has to
/// name five parameters to read the second reads worse than one that does not.
/// `id` is opaque and only meaningful to `service:reply`.
pub(crate) fn execute_dbus_call_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    call: DbusCall,
    limits: Limits,
) -> Result<(), String> {
    let table = Table::new(&ctx);
    table.set_field(ctx, "id", call.id as i64);
    table.set_field(ctx, "interface", call.interface.as_str());
    table.set_field(ctx, "member", call.member.as_str());
    table.set_field(ctx, "path", call.path.as_str());
    table.set_field(ctx, "sender", call.sender.as_str());
    table.set_field(ctx, "arguments", dbus_value_to_lua(ctx, call.arguments)?);
    let executor = Executor::start(
        ctx,
        ctx.fetch(closure).into(),
        Variadic(vec![LuaValue::Table(table)]),
    );
    drive_executor(ctx, executor, limits, limits.effect_fuel, "handler")?;
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn udev_event_value(event: UdevEvent) -> DbusValue {
    let properties = event
        .properties
        .into_iter()
        .map(|(key, value)| (key, DbusValue::String(value)))
        .collect();
    DbusValue::Map(BTreeMap::from([
        ("action".to_owned(), DbusValue::String(event.action)),
        ("devpath".to_owned(), DbusValue::String(event.devpath)),
        (
            "subsystem".to_owned(),
            event.subsystem.map_or(DbusValue::Nil, DbusValue::String),
        ),
        (
            "devname".to_owned(),
            event.devname.map_or(DbusValue::Nil, DbusValue::String),
        ),
        ("properties".to_owned(), DbusValue::Map(properties)),
    ]))
}

pub(crate) fn status_notifier_value(items: Vec<StatusNotifierAddress>) -> DbusValue {
    DbusValue::List(
        items
            .into_iter()
            .map(|item| {
                DbusValue::Map(BTreeMap::from([
                    ("service".to_owned(), DbusValue::String(item.service)),
                    ("path".to_owned(), DbusValue::String(item.path)),
                ]))
            })
            .collect(),
    )
}

pub(crate) fn execute_ipc_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    args: &[IpcValue],
    limits: Limits,
) -> Result<Vec<IpcValue>, String> {
    let args = Variadic(
        args.iter()
            .map(|value| value.to_lua(ctx))
            .collect::<Vec<_>>(),
    );
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    drive_executor(ctx, executor, limits, limits.effect_fuel, "IPC handler")?;
    let values = match executor.take_result::<Variadic<Vec<LuaValue>>>(ctx) {
        Ok(Ok(values)) => values,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(error) => return Err(error.to_string()),
    };
    values.into_iter().map(IpcValue::from_lua).collect()
}
