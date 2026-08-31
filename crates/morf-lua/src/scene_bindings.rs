use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef, Value as LuaValue};
use std::cell::RefCell;
use std::error::Error as StdError;
use std::fmt;
use std::rc::Rc;

use morf_scene::{Behavior, Element, NodeHandle, Value as SceneValue};

use crate::{reactive_bindings::*, serialization::*, state::*, surface_types::*};

pub(crate) fn create_node(state: &Rc<RefCell<ReactiveState>>, element: Element) -> NodeHandle {
    let mut state = state.borrow_mut();
    state.scene_revision = state.scene_revision.wrapping_add(1);
    state.scene.create(element)
}

pub(crate) fn bump_property_signal(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    target: bool,
) -> Result<(), String> {
    let Some(signal) = state
        .property_signals
        .get(&(node, property.to_owned(), target))
        .copied()
    else {
        return Ok(());
    };
    state.property_revision = state.property_revision.wrapping_add(1);
    let value = IpcValue::Integer(state.property_revision);
    if let Some(active) = &mut state.active {
        active.writes.push((signal, value.clone()));
    } else {
        state
            .graph
            .as_mut()
            .ok_or_else(|| "reactive graph is already running".to_owned())?
            .write(signal, value.clone())
            .map_err(|error| error.to_string())?;
    }
    state.values.insert(signal, value);
    Ok(())
}

pub(crate) fn assign_scene_property(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    value: SceneValue,
) -> Result<(), String> {
    let old_current = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    let old_target = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    state
        .scene
        .assign(node, property, value)
        .map_err(|error| error.to_string())?;
    let current_changed = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        != &old_current;
    let target_changed = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        != &old_target;
    if current_changed || target_changed {
        state.scene_revision = state.scene_revision.wrapping_add(1);
    }
    if current_changed {
        bump_property_signal(state, node, property, false)?;
    }
    if target_changed {
        bump_property_signal(state, node, property, true)?;
    }
    Ok(())
}

pub(crate) fn animate_scene_property(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    from: SceneValue,
    to: SceneValue,
    behavior: Behavior,
) -> Result<(), String> {
    let old_current = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    let old_target = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    state
        .scene
        .animate_from(node, property, from, to, behavior)
        .map_err(|error| error.to_string())?;
    if state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        != &old_current
    {
        state.scene_revision = state.scene_revision.wrapping_add(1);
        bump_property_signal(state, node, property, false)?;
    }
    if state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        != &old_target
    {
        bump_property_signal(state, node, property, true)?;
    }
    Ok(())
}

/// Wraps one scene node as a Lua handle.
///
/// The metatable is built once and shared by every node. Neither of its two
/// callbacks captures anything about a particular node — both take the node off
/// the stack as a `UserRef<NodeToken>` — so building a table and two closures
/// per node allocated three objects per node that were all identical.
pub(crate) fn node_userdata<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    handle: NodeHandle,
) -> UserData<'gc> {
    let existing = state.borrow().node_metatable.clone();
    let metatable = match existing {
        Some(stashed) => ctx.fetch(&stashed),
        None => {
            let built = node_metatable(ctx, Rc::clone(&state));
            state.borrow_mut().node_metatable = Some(ctx.stash(built));
            built
        }
    };
    let userdata = UserData::new_static(&ctx, NodeToken { handle });
    userdata.set_metatable(ctx, Some(metatable));
    userdata
}

pub(crate) fn node_metatable<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Table<'gc> {
    let read_state = Rc::clone(&state);
    let index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, key): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
        let element = read_state
            .borrow()
            .scene
            .element(node.handle)
            .map_err(|error| HostError(error.to_string()))?;
        if key == "item" && element == Element::Loader {
            let child = read_state
                .borrow()
                .scene
                .children(node.handle)
                .map_err(|error| HostError(error.to_string()))?
                .first()
                .copied();
            match child {
                Some(child) => {
                    stack.replace(ctx, node_userdata(ctx, Rc::clone(&read_state), child))
                }
                None => stack.replace(ctx, LuaValue::Nil),
            }
            return Ok(CallbackReturn::Return);
        }
        let key = if key == "active_async" && element == Element::Loader {
            "active".to_owned()
        } else {
            key
        };
        let (property, target) = key
            .strip_suffix("_target")
            .map_or((key.as_str(), false), |property| (property, true));
        let value = {
            let mut state = read_state.borrow_mut();
            if !state
                .scene
                .has_property(node.handle, property)
                .map_err(|error| HostError(error.to_string()))?
            {
                return Err(HostError(format!("unknown node property `{key}`")).into());
            }
            let property_key = (node.handle, property.to_owned(), target);
            let signal = state.property_signals.get(&property_key).copied();
            if let Some(active) = &mut state.active {
                if let Some(signal) = signal {
                    active.reads.insert(signal);
                } else {
                    active.property_reads.insert(property_key);
                }
            }
            if target {
                state.scene.target(node.handle, property)
            } else {
                state.scene.current(node.handle, property)
            }
            .map_err(|error| HostError(error.to_string()))?
            .clone()
        };
        stack.replace(ctx, scene_to_lua(ctx, &value).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let new_index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, property, value): (UserRef<NodeToken>, String, LuaValue) = stack.consume(ctx)?;
        let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
        assign_scene_property(&mut state.borrow_mut(), node.handle, &property, value)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let metatable = Table::new(&ctx);
    metatable.set_field(ctx, "__index", index);
    metatable.set_field(ctx, "__newindex", new_index);
    metatable
}

#[derive(Debug)]
pub(crate) struct HostError(pub(crate) String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for HostError {}
