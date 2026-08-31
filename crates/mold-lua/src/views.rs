use luna::{Context, Executor, Function, StashedClosure, UserRef, Value as LuaValue, Variadic};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use mold_scene::{Element, ListChange, NodeHandle, Scene, Value as SceneValue, ViewTransition};

use crate::{
    reactive_bindings::*, reactive_execute::*, scene_bindings::*, serialization::*, state::*,
    types::*,
};

pub(crate) fn execute_delegate(
    ctx: Context<'_>,
    delegate: &StashedClosure,
    item: &SceneValue,
    index: usize,
    limits: Limits,
) -> Result<DelegateInstance, String> {
    let args = Variadic(vec![
        scene_to_lua(ctx, item)?,
        LuaValue::Integer(index as i64 + 1),
    ]);
    let executor = Executor::start(ctx, ctx.fetch(delegate).into(), args);
    drive_executor(ctx, executor, limits, limits.effect_fuel, "delegate")?;
    let values = match executor.take_result::<Variadic<Vec<LuaValue>>>(ctx) {
        Ok(Ok(values)) => values,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(error) => return Err(error.to_string()),
    };
    let Some(LuaValue::UserData(node)) = values.first().copied() else {
        return Err("view delegate must return a mold node".to_owned());
    };
    let node = node
        .downcast_static::<NodeToken>()
        .map_err(|_| "view delegate must return a mold node".to_owned())?;
    let updater = match values.get(1).copied().unwrap_or(LuaValue::Nil) {
        LuaValue::Nil => None,
        LuaValue::Function(Function::Closure(updater)) => Some(ctx.stash(updater)),
        _ => return Err("view delegate updater must be a function".to_owned()),
    };
    Ok(DelegateInstance {
        node: node.handle,
        updater,
    })
}

pub(crate) fn execute_delegate_updater(
    ctx: Context<'_>,
    updater: &StashedClosure,
    item: &SceneValue,
    index: usize,
    limits: Limits,
) -> Result<(), String> {
    let args = Variadic(vec![
        scene_to_lua(ctx, item)?,
        LuaValue::Integer(index as i64 + 1),
    ]);
    let executor = Executor::start(ctx, ctx.fetch(updater).into(), args);
    drive_executor(
        ctx,
        executor,
        limits,
        limits.effect_fuel,
        "delegate updater",
    )?;
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn execute_node_factory(
    ctx: Context<'_>,
    factory: &StashedClosure,
    limits: Limits,
) -> Result<NodeHandle, String> {
    let executor = Executor::start(ctx, ctx.fetch(factory).into(), ());
    drive_executor(ctx, executor, limits, limits.effect_fuel, "Loader source")?;
    match executor.take_result::<UserRef<NodeToken>>(ctx) {
        Ok(Ok(node)) => Ok(node.handle),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn position_view_child(
    scene: &mut Scene,
    node: NodeHandle,
    index: usize,
    row_extent: f64,
    offset: f64,
    columns: usize,
    column_extent: f64,
) -> Result<(), String> {
    scene
        .assign(node, "x", (index % columns) as f64 * column_extent)
        .map_err(|error| error.to_string())?;
    scene
        .assign(node, "y", (index / columns) as f64 * row_extent - offset)
        .map_err(|error| error.to_string())
}

pub(crate) fn reconcile_lua_view(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    parent: NodeHandle,
    offset: f64,
    view: &mut LuaVirtualView,
) -> Result<Vec<ViewTransition>, String> {
    if !offset.is_finite() || offset < 0.0 {
        return Err("ListView offset must be finite and non-negative".to_owned());
    }
    view.view.set_offset(offset);
    let changes = view.model.borrow_mut().take_changes();
    let updated = changes
        .iter()
        .filter_map(|change| match change {
            ListChange::Updated { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let invalidated = changes
        .iter()
        .filter_map(|change| match change {
            ListChange::Removed { id, .. } | ListChange::Updated { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    for id in &invalidated {
        if let Some(instance) = view.reusable.remove(id) {
            state
                .borrow_mut()
                .scene
                .remove(instance.node)
                .map_err(|error| error.to_string())?;
        }
    }
    view.reuse_order.retain(|id| !invalidated.contains(id));
    let model = view.model.borrow();
    let transitions = view.view.sync(&model, &changes);
    let visible = view
        .view
        .visible_range(model.len())
        .filter_map(|index| {
            model
                .get(index)
                .map(|(id, value)| (id, index, value.clone()))
        })
        .collect::<Vec<_>>();
    drop(model);
    let visible_ids = visible.iter().map(|(id, _, _)| *id).collect::<HashSet<_>>();
    let mut prepared = Vec::new();
    for (id, index, item) in &visible {
        if !view.active.contains_key(id) || updated.contains(id) {
            if let Some(instance) = view.reusable.remove(id) {
                view.reuse_order.retain(|candidate| candidate != id);
                prepared.push((*id, *index, instance));
                continue;
            }
            let reusable_id = view.reuse_order.iter().copied().find(|candidate| {
                view.reusable
                    .get(candidate)
                    .is_some_and(|instance| instance.updater.is_some())
            });
            if let Some(reusable_id) = reusable_id {
                view.reuse_order
                    .retain(|candidate| *candidate != reusable_id);
                let instance = view
                    .reusable
                    .remove(&reusable_id)
                    .expect("reuse order contains a live delegate");
                let update = execute_delegate_updater(
                    ctx,
                    instance.updater.as_ref().expect("updater was checked"),
                    item,
                    *index,
                    limits,
                )
                .and_then(|()| flush_reactive(state, ctx, limits));
                if let Err(error) = update {
                    let _ = state.borrow_mut().scene.remove(instance.node);
                    for (_, _, prepared) in prepared {
                        let _ = state.borrow_mut().scene.remove(prepared.node);
                    }
                    return Err(error);
                }
                prepared.push((*id, *index, instance));
                continue;
            }
            match execute_delegate(ctx, &view.delegate, item, *index, limits) {
                Ok(instance) => prepared.push((*id, *index, instance)),
                Err(error) => {
                    for (_, _, prepared) in prepared {
                        let _ = state.borrow_mut().scene.remove(prepared.node);
                    }
                    return Err(error);
                }
            }
        }
    }
    let removed = view
        .active
        .iter()
        .filter(|(id, _)| !visible_ids.contains(id) || updated.contains(id))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in removed {
        let instance = view.active.remove(&id).expect("removed delegate is active");
        if invalidated.contains(&id) {
            state
                .borrow_mut()
                .scene
                .remove(instance.node)
                .map_err(|error| error.to_string())?;
            continue;
        }
        let pool_root = match view.pool_root {
            Some(node) => node,
            None => {
                let pool = create_node(state, Element::Item);
                state
                    .borrow_mut()
                    .scene
                    .assign(pool, "visible", false)
                    .map_err(|error| error.to_string())?;
                state
                    .borrow_mut()
                    .scene
                    .reparent(pool, Some(parent))
                    .map_err(|error| error.to_string())?;
                view.pool_root = Some(pool);
                pool
            }
        };
        state
            .borrow_mut()
            .scene
            .reparent(instance.node, Some(pool_root))
            .map_err(|error| error.to_string())?;
        view.reusable.insert(id, instance);
        view.reuse_order.push_back(id);
    }
    while view.reusable.len() > view.reuse_limit {
        let Some(id) = view.reuse_order.pop_front() else {
            break;
        };
        if let Some(instance) = view.reusable.remove(&id) {
            state
                .borrow_mut()
                .scene
                .remove(instance.node)
                .map_err(|error| error.to_string())?;
        }
    }
    for (id, index, instance) in prepared {
        position_view_child(
            &mut state.borrow_mut().scene,
            instance.node,
            index,
            view.view.item_extent(),
            offset,
            view.view.columns(),
            view.column_extent,
        )?;
        state
            .borrow_mut()
            .scene
            .reparent(instance.node, Some(parent))
            .map_err(|error| error.to_string())?;
        view.active.insert(id, instance);
    }
    for (id, index, _) in visible {
        if let Some(instance) = view.active.get(&id) {
            position_view_child(
                &mut state.borrow_mut().scene,
                instance.node,
                index,
                view.view.item_extent(),
                offset,
                view.view.columns(),
                view.column_extent,
            )?;
        }
    }
    Ok(transitions)
}
