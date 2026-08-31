use luna::{
    Callback, CallbackReturn, Closure, Context, Executor, Table, UserData, UserRef,
    Value as LuaValue, Variadic,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mold_scene::{Behavior, ListModel, Value as SceneValue, VirtualList};

use crate::{
    lua_values::*, reactive_bindings::*, reactive_execute::*, scene_bindings::*, serialization::*,
    state::*, surface_types::*, table_menu::*, types::*, views::*,
};

pub(crate) fn install_view_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    mold: Table<'gc>,
    limits: Limits,
) {
    let variants = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (items, factory): (Table, Closure) = stack.consume(ctx)?;
        let mut values = items
            .iter(ctx)
            .map(|(key, value)| match key {
                LuaValue::Integer(index) => Ok((index, value)),
                _ => Err(HostError("variants model keys must be integers".into())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() > 256 {
            return Err(HostError("variants model exceeds 256 entries".into()).into());
        }
        values.sort_by_key(|(index, _)| *index);
        let instances = Table::new(&ctx);
        // One budget for the whole call, not one per entry. Every factory here
        // runs from a single `mold.variants`, so giving each of up to 256 items
        // a full effect budget would let one construction spend 256 times what
        // any other Lua entry point is allowed.
        let mut budget = limits.effect_fuel;
        for (offset, (index, item)) in values.into_iter().enumerate() {
            if index != offset as i64 + 1 {
                return Err(HostError("variants model must be a dense sequence".into()).into());
            }
            let executor = Executor::start(ctx, factory.into(), Variadic(vec![item]));
            let spent = drive_executor(ctx, executor, limits, budget, "variant factory")
                .map_err(HostError)?;
            budget = budget.saturating_sub(spent);
            let value = executor
                .take_result::<LuaValue>(ctx)
                .map_err(|error| HostError(error.to_string()))??;
            instances.set(ctx, index, value)?;
        }
        stack.replace(ctx, instances);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "variants", variants);

    let model_len = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let model: UserRef<ListModelToken> = stack.consume(ctx)?;
        stack.replace(ctx, model.model.borrow().len() as i64);
        Ok(CallbackReturn::Return)
    });
    let model_get = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, index): (UserRef<ListModelToken>, i64) = stack.consume(ctx)?;
        let index = lua_index(index)?;
        let model = model.model.borrow();
        let value = model
            .get(index)
            .map(|(_, value)| scene_to_lua(ctx, value))
            .transpose()
            .map_err(HostError)?
            .unwrap_or(LuaValue::Nil);
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let model_index_of = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, value): (UserRef<ListModelToken>, LuaValue) = stack.consume(ctx)?;
        let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
        let model = model.model.borrow();
        let index = (0..model.len()).find(|index| {
            model
                .get(*index)
                .is_some_and(|(_, candidate)| candidate == &value)
        });
        match index {
            Some(index) => stack.replace(ctx, index as i64 + 1),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let model_insert = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, index, value): (UserRef<ListModelToken>, i64, LuaValue) = stack.consume(ctx)?;
        let index = lua_insert_index(index, model.model.borrow().len())?;
        let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
        if model.model.borrow_mut().insert(index, value).is_none() {
            return Err(HostError("list-model insert index is out of range".into()).into());
        }
        Ok(CallbackReturn::Return)
    });
    let model_remove = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, index): (UserRef<ListModelToken>, i64) = stack.consume(ctx)?;
        let index = lua_index(index)?;
        let value = model
            .model
            .borrow_mut()
            .remove(index)
            .map(|value| scene_to_lua(ctx, &value))
            .transpose()
            .map_err(HostError)?
            .unwrap_or(LuaValue::Nil);
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let model_move = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, from, to): (UserRef<ListModelToken>, i64, i64) = stack.consume(ctx)?;
        let from = lua_index(from)?;
        let to = lua_index(to)?;
        if !model.model.borrow_mut().move_item(from, to) {
            return Err(HostError("list-model move index is out of range".into()).into());
        }
        Ok(CallbackReturn::Return)
    });
    let model_set = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, index, value): (UserRef<ListModelToken>, i64, LuaValue) = stack.consume(ctx)?;
        let index = lua_index(index)?;
        let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
        if !model.model.borrow_mut().set(index, value) {
            return Err(HostError("list-model update index is out of range".into()).into());
        }
        Ok(CallbackReturn::Return)
    });
    let model_replace = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (model, items, object_property): (UserRef<ListModelToken>, Table, Option<String>) =
            stack.consume(ctx)?;
        if object_property
            .as_ref()
            .is_some_and(|property| property.is_empty() || property.len() > 128)
        {
            return Err(
                HostError("list-model object property must contain 1 to 128 bytes".into()).into(),
            );
        }
        let value = lua_to_scene(ctx, LuaValue::Table(items), 0).map_err(HostError)?;
        let SceneValue::List(values) = value else {
            return Err(HostError("list-model replacement needs an array table".into()).into());
        };
        model
            .model
            .borrow_mut()
            .reconcile(values, object_property.as_deref());
        Ok(CallbackReturn::Return)
    });
    let model_methods = Table::new(&ctx);
    model_methods.set_field(ctx, "len", model_len);
    model_methods.set_field(ctx, "get", model_get);
    model_methods.set_field(ctx, "index_of", model_index_of);
    model_methods.set_field(ctx, "insert", model_insert);
    model_methods.set_field(ctx, "remove", model_remove);
    model_methods.set_field(ctx, "move", model_move);
    model_methods.set_field(ctx, "set", model_set);
    model_methods.set_field(ctx, "replace", model_replace);
    let model_metatable = Table::new(&ctx);
    model_metatable.set_field(ctx, "__index", model_methods);
    let model_metatable = ctx.stash(model_metatable);
    let list_model = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let items: Table = stack.consume(ctx)?;
        let value = lua_to_scene(ctx, LuaValue::Table(items), 0).map_err(HostError)?;
        let SceneValue::List(values) = value else {
            return Err(HostError("list-model needs an array table".into()).into());
        };
        let userdata = UserData::new_static(
            &ctx,
            ListModelToken {
                model: Rc::new(RefCell::new(ListModel::new(values))),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&model_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "list_model", list_model);

    let virtual_visible = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let view: UserRef<VirtualListToken> = stack.consume(ctx)?;
        let model = view.model.borrow();
        let range = view.view.borrow().visible_range(model.len());
        let items = Table::new(&ctx);
        for (position, index) in range.enumerate() {
            let value = Table::new(&ctx);
            value.set_field(ctx, "index", index as i64 + 1);
            if let Some((_, item)) = model.get(index) {
                value.set_field(ctx, "item", scene_to_lua(ctx, item).map_err(HostError)?);
            }
            items
                .set(ctx, position as i64 + 1, value)
                .expect("virtual-list table accepts integer keys");
        }
        stack.replace(ctx, items);
        Ok(CallbackReturn::Return)
    });
    let virtual_offset = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (view, offset): (UserRef<VirtualListToken>, f64) = stack.consume(ctx)?;
        view.view.borrow_mut().set_offset(offset);
        Ok(CallbackReturn::Return)
    });
    let virtual_sync = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let view: UserRef<VirtualListToken> = stack.consume(ctx)?;
        let changes = view.model.borrow_mut().take_changes();
        let transitions = view.view.borrow_mut().sync(&view.model.borrow(), &changes);
        let result = Table::new(&ctx);
        for (index, transition) in transitions.into_iter().enumerate() {
            result
                .set(
                    ctx,
                    index as i64 + 1,
                    view_transition_to_lua(ctx, transition),
                )
                .expect("view-transition table accepts integer keys");
        }
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    });
    let virtual_methods = Table::new(&ctx);
    virtual_methods.set_field(ctx, "visible", virtual_visible);
    virtual_methods.set_field(ctx, "set_offset", virtual_offset);
    virtual_methods.set_field(ctx, "sync", virtual_sync);
    let virtual_metatable = Table::new(&ctx);
    virtual_metatable.set_field(ctx, "__index", virtual_methods);
    let virtual_metatable = ctx.stash(virtual_metatable);
    let virtual_list = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (model, item_extent, viewport_extent, overscan): (
            UserRef<ListModelToken>,
            f64,
            f64,
            i64,
        ) = stack.consume(ctx)?;
        let overscan = usize::try_from(overscan)
            .map_err(|_| HostError("virtual-list overscan cannot be negative".into()))?;
        let view = VirtualList::new(item_extent, viewport_extent, overscan)
            .ok_or_else(|| HostError("invalid virtual-list dimensions".into()))?;
        let userdata = UserData::new_static(
            &ctx,
            VirtualListToken {
                model: Rc::clone(&model.model),
                view: RefCell::new(view),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&virtual_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "virtual_list", virtual_list);

    let sync_state = Rc::clone(&state);
    let sync_view = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, offset): (UserRef<NodeToken>, f64) = stack.consume(ctx)?;
        let mut view = sync_state
            .borrow_mut()
            .views
            .remove(&node.handle)
            .ok_or_else(|| HostError("node is not a ListView".to_owned()))?;
        let result = reconcile_lua_view(&sync_state, ctx, limits, node.handle, offset, &mut view);
        sync_state.borrow_mut().views.insert(node.handle, view);
        let transitions = result.map_err(HostError)?;
        let values = Table::new(&ctx);
        for (index, transition) in transitions.into_iter().enumerate() {
            values
                .set(
                    ctx,
                    index as i64 + 1,
                    view_transition_to_lua(ctx, transition),
                )
                .expect("view-transition table accepts integer keys");
        }
        stack.replace(ctx, values);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "sync_view", sync_view);

    let transition_state = Rc::clone(&state);
    let transition_parent = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, parent, options): (UserRef<NodeToken>, UserRef<NodeToken>, Table) =
            stack.consume(ctx)?;
        let duration = table_number(ctx, options, "duration", 250.0).map_err(HostError)?;
        if duration < 0.0 {
            return Err(HostError("parent-transition duration cannot be negative".into()).into());
        }
        let easing = parse_easing(ctx, options.get_value(ctx, "easing")).map_err(HostError)?;
        let anchors = match options.get_value(ctx, "anchors") {
            LuaValue::Nil => None,
            value => match lua_to_scene(ctx, value, 0).map_err(HostError)? {
                SceneValue::Map(anchors) => Some(anchors),
                _ => return Err(HostError("transition anchors must be a table".into()).into()),
            },
        };
        transition_state
            .borrow_mut()
            .parent_transitions
            .push(ParentTransitionRequest {
                node: node.handle,
                parent: parent.handle,
                anchors,
                behavior: Behavior::timed(Duration::from_secs_f64(duration / 1_000.0), easing),
            });
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "transition_parent", transition_parent);
}
