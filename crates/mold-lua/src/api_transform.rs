use luna::{
    Callback, CallbackReturn, Context, Function, Table, UserData, UserRef, Value as LuaValue,
};
use std::cell::RefCell;
use std::rc::Rc;

use mold_layout::TransformWatcher as NativeTransformWatcher;

use crate::{runtime_helpers::*, scene_bindings::*, state::*};

pub(crate) fn install_transform_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    mold: Table<'gc>,
) {
    let transform_revision = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let watcher: UserRef<TransformWatcherToken> = stack.consume(ctx)?;
            let revision = state
                .borrow()
                .transform_watchers
                .get(&watcher.id)
                .map(|watcher| watcher.revision)
                .ok_or_else(|| HostError("transform watcher is stale".into()))?;
            stack.replace(ctx, revision as i64);
            Ok(CallbackReturn::Return)
        }
    });
    let transform_methods = Table::new(&ctx);
    transform_methods.set_field(ctx, "revision", transform_revision);
    let transform_metatable = Table::new(&ctx);
    transform_metatable.set_field(ctx, "__index", transform_methods);
    let transform_metatable = ctx.stash(transform_metatable);
    let transform_watcher = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let node = |field| match options.get_value(ctx, field) {
                LuaValue::UserData(value) => value
                    .downcast_static::<NodeToken>()
                    .map(|node| node.handle)
                    .map_err(|_| HostError(format!("transform watcher {field} must be a node"))),
                _ => Err(HostError(format!(
                    "transform watcher {field} must be a node"
                ))),
            };
            let a = node("a")?;
            let b = node("b")?;
            let common_parent = match options.get_value(ctx, "common_parent") {
                LuaValue::Nil => None,
                LuaValue::UserData(value) => Some(
                    value
                        .downcast_static::<NodeToken>()
                        .map_err(|_| {
                            HostError("transform watcher common_parent must be a node".into())
                        })?
                        .handle,
                ),
                _ => {
                    return Err(HostError(
                        "transform watcher common_parent must be a node or nil".into(),
                    )
                    .into());
                }
            };
            let callback = match options.get_value(ctx, "on_changed") {
                LuaValue::Nil => None,
                LuaValue::Function(Function::Closure(callback)) => Some(ctx.stash(callback)),
                _ => {
                    return Err(HostError(
                        "transform watcher on_changed must be a function".into(),
                    )
                    .into());
                }
            };
            let id = {
                let mut state = state.borrow_mut();
                if state.transform_watchers.len() >= 1_024 {
                    return Err(HostError("transform watcher limit reached".into()).into());
                }
                state
                    .scene
                    .element(a)
                    .and_then(|_| state.scene.element(b))
                    .map_err(|error| HostError(error.to_string()))?;
                if let Some(common) = common_parent {
                    state
                        .scene
                        .element(common)
                        .map_err(|error| HostError(error.to_string()))?;
                    if !scene_node_in_subtree(&state.scene, common, a)
                        || !scene_node_in_subtree(&state.scene, common, b)
                    {
                        return Err(HostError(
                            "transform watcher common_parent must contain both nodes".into(),
                        )
                        .into());
                    }
                }
                let id = state.next_transform_watcher;
                state.next_transform_watcher = id.wrapping_add(1);
                state.transform_watchers.insert(
                    id,
                    LuaTransformWatcher {
                        a,
                        b,
                        watcher: NativeTransformWatcher::new(a, b, common_parent),
                        callback,
                        revision: 0,
                        pending: false,
                    },
                );
                id
            };
            let value = UserData::new_static(&ctx, TransformWatcherToken { id });
            value.set_metatable(ctx, Some(ctx.fetch(&transform_metatable)));
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    mold.set_field(ctx, "transform_watcher", transform_watcher);
}
