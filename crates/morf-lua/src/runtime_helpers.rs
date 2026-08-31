use luna::{Context, Table, Value as LuaValue};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use morf_reactive::SignalId;
use morf_scene::{NodeHandle, Scene};

use crate::{events::*, reactive_execute::*, state::*, surface_types::*, types::*};

pub(crate) fn geometry_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

pub(crate) fn scene_node_in_subtree(scene: &Scene, root: NodeHandle, node: NodeHandle) -> bool {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if candidate == root {
            return true;
        }
        current = scene.parent(candidate).ok().flatten();
    }
    false
}

pub(crate) fn key_targets_in(state: &ReactiveState, root: NodeHandle) -> Vec<NodeHandle> {
    let mut targets = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if state.handlers.contains_key(&(node, UiEvent::KeyPressed))
            && state.scene.bool_value(node, "enabled").unwrap_or(false)
            && state.scene.bool_value(node, "visible").unwrap_or(false)
        {
            targets.push(node);
        }
        if let Ok(children) = state.scene.children(node) {
            pending.extend(children.iter().copied().rev());
        }
    }
    targets
}

pub(crate) fn remove_scene_subtree(state: &mut ReactiveState, node: NodeHandle) {
    let mut nodes = vec![node];
    let mut index = 0;
    while index < nodes.len() {
        let children = state.scene.children(nodes[index]).unwrap_or_default();
        nodes.extend_from_slice(children);
        index += 1;
    }
    state.scene_revision = state.scene_revision.wrapping_add(1);
    if state.scene.remove(node).is_err() {
        return;
    }
    let removed = nodes.into_iter().collect::<HashSet<_>>();
    for node in &removed {
        state.retention.unregister(*node);
        state.retain_callbacks.remove(node);
        state.states.remove(node);
        state.views.remove(node);
        state.timer_callbacks.remove(node);
        state.loader_factories.remove(node);
        state
            .animation_callbacks
            .retain(|(owner, _), _| owner != node);
        state.loaded_loaders.remove(node);
    }
    state
        .handlers
        .retain(|(node, _), _| !removed.contains(node));
    state
        .timers
        .retain(|timer| timer.node.is_none_or(|node| !removed.contains(&node)));
    state
        .property_signals
        .retain(|(node, _, _), _| !removed.contains(node));
    state
        .current_property_names
        .retain(|_, (node, _)| !removed.contains(node));
    state
        .transform_watchers
        .retain(|_, watcher| !removed.contains(&watcher.a) && !removed.contains(&watcher.b));
    let surface_count = state.window_surfaces.len();
    state
        .window_surfaces
        .retain(|_, surface| !removed.contains(&surface.root));
    let removed_anchors = state
        .popup_node_anchors
        .iter()
        .filter_map(|(id, anchor)| removed.contains(&anchor.node).then_some(*id))
        .collect::<Vec<_>>();
    for id in removed_anchors {
        state.popup_node_anchors.remove(&id);
        if let Some(surface) = state.window_surfaces.get_mut(&id) {
            surface.visible = false;
            state.window_surfaces_changed = true;
        }
    }
    let surface_ids = state
        .window_surfaces
        .keys()
        .copied()
        .collect::<HashSet<_>>();
    state
        .popup_node_anchors
        .retain(|id, anchor| surface_ids.contains(id) && !removed.contains(&anchor.node));
    state.window_surfaces_changed |= state.window_surfaces.len() != surface_count;
}

pub(crate) fn finish_retained_destroy(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    node: NodeHandle,
) {
    let callback = state
        .borrow()
        .retain_callbacks
        .get(&node)
        .and_then(|callbacks| callbacks.about_to_destroy.clone());
    if let Some(callback) = callback
        && let Err(error) = execute_handler_args(ctx, &callback, &[], limits)
    {
        state
            .borrow_mut()
            .logs
            .push(format!("Retainable about_to_destroy: {error}"));
    }
    remove_scene_subtree(&mut state.borrow_mut(), node);
}

pub(crate) fn drop_retainable(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    node: NodeHandle,
) {
    let registered = state.borrow().retention.state(node).is_some();
    if !registered {
        remove_scene_subtree(&mut state.borrow_mut(), node);
        return;
    }
    let callback = {
        let mut state = state.borrow_mut();
        let _ = state.retention.begin_drop(node);
        state
            .retain_callbacks
            .get(&node)
            .and_then(|callbacks| callbacks.dropped.clone())
    };
    if let Some(callback) = callback
        && let Err(error) = execute_handler_args(ctx, &callback, &[], limits)
    {
        state
            .borrow_mut()
            .logs
            .push(format!("Retainable dropped: {error}"));
    }
    if state
        .borrow()
        .retention
        .should_destroy(node)
        .unwrap_or(true)
    {
        finish_retained_destroy(state, ctx, limits, node);
    }
}

pub(crate) fn register_reloadable_value(
    state: &mut ReactiveState,
    name: String,
    initial: IpcValue,
) -> Result<(SignalId, bool), String> {
    // Here, not at each door. Four different entry points reach this one map,
    // and they applied three different rules between them — so what counted as
    // a legal name depended on which way you came in, and a name accepted by
    // one door could collide with, or be unreachable from, another.
    validate_scope_part(&name)?;
    if state.reloadable.contains_key(&name) {
        return Err(format!("reloadable id `{name}` is already registered"));
    }
    let mut restored = false;
    let value = match state.reload_seed.remove(&name) {
        Some(value) if std::mem::discriminant(&value) == std::mem::discriminant(&initial) => {
            restored = true;
            value
        }
        Some(_) => {
            state.logs.push(format!(
                "reloadable `{name}` changed value type; using its new default"
            ));
            initial
        }
        None => initial,
    };
    let id = state
        .graph
        .as_mut()
        .ok_or_else(|| "reactive graph is already running".to_owned())?
        .signal(format!("reloadable.{name}"), value.clone());
    state.values.insert(id, value);
    state.signals.push(id);
    state.reloadable.insert(name, id);
    Ok((id, restored))
}

pub(crate) fn create_persistent_token<'gc>(
    ctx: Context<'gc>,
    state: &Rc<RefCell<ReactiveState>>,
    name: &str,
    defaults: Table<'gc>,
) -> Result<PersistentToken, String> {
    if name.is_empty() || name.len() > 256 {
        return Err("persistent id must be 1..256 bytes".into());
    }
    let mut definitions = Vec::new();
    for (key, value) in defaults.iter(ctx) {
        let LuaValue::String(key) = key else {
            return Err("persistent property names must be strings".into());
        };
        let key = key.display_lossy().to_string();
        if key.is_empty() || key.len() > 256 || matches!(key.as_str(), "loaded" | "reloaded") {
            return Err(format!("invalid persistent property `{key}`"));
        }
        definitions.push((key, IpcValue::from_lua(value)?));
        if definitions.len() > 256 {
            return Err("persistent object exceeds 256 properties".into());
        }
    }
    definitions.sort_by(|left, right| left.0.cmp(&right.0));
    let mut properties = HashMap::new();
    let mut reloaded = false;
    let mut state = state.borrow_mut();
    for (key, initial) in definitions {
        let full_name = format!("{name}.{key}");
        let (id, restored) = register_reloadable_value(&mut state, full_name, initial)?;
        reloaded |= restored;
        properties.insert(key, id);
    }
    Ok(PersistentToken {
        properties,
        reloaded,
    })
}

pub(crate) fn validate_scope_part(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 {
        return Err("scope IDs must be 1..256 bytes".into());
    }
    if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
        return Err("scope IDs cannot contain empty segments".into());
    }
    Ok(())
}

pub(crate) fn scoped_id(prefix: &str, name: &str) -> Result<String, String> {
    validate_scope_part(prefix)?;
    validate_scope_part(name)?;
    let value = format!("{prefix}.{name}");
    if value.len() > 256 {
        return Err("scoped reloadable ID exceeds 256 bytes".into());
    }
    Ok(value)
}
