fn install_view_api<'gc>(
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
        for (offset, (index, item)) in values.into_iter().enumerate() {
            if index != offset as i64 + 1 {
                return Err(HostError("variants model must be a dense sequence".into()).into());
            }
            let executor = Executor::start(ctx, factory.into(), Variadic(vec![item]));
            let budget = limits.effect_fuel;
            let mut remaining = budget;
            loop {
                if remaining == 0 {
                    executor.stop(&ctx);
                    return Err(HostError(format!(
                        "Lua variant factory fuel exhausted after {budget} instructions"
                    ))
                    .into());
                }
                let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
                let mut fuel = Fuel::with(allowance);
                let finished = executor.step(ctx, &mut fuel)?;
                let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
                remaining = remaining.saturating_sub(consumed.max(1));
                if finished {
                    let value = executor
                        .take_result::<LuaValue>(ctx)
                        .map_err(|error| HostError(error.to_string()))??;
                    instances.set(ctx, index, value)?;
                    break;
                }
            }
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

    let flick_drag = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (flick, delta): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
        flick.state.borrow_mut().drag_by(delta);
        stack.replace(ctx, flick.state.borrow().offset);
        Ok(CallbackReturn::Return)
    });
    let flick_release = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (flick, velocity): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
        flick.state.borrow_mut().release(velocity);
        Ok(CallbackReturn::Return)
    });
    let flick_tick = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (flick, milliseconds): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
        if !milliseconds.is_finite() || milliseconds < 0.0 {
            return Err(HostError("flick delta must be finite and non-negative".into()).into());
        }
        let active = flick
            .state
            .borrow_mut()
            .tick(Duration::from_secs_f64(milliseconds / 1_000.0));
        stack.replace(ctx, (flick.state.borrow().offset, active));
        Ok(CallbackReturn::Return)
    });
    let flick_position = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let flick: UserRef<FlickToken> = stack.consume(ctx)?;
        stack.replace(ctx, flick.state.borrow().offset);
        Ok(CallbackReturn::Return)
    });
    let flick_methods = Table::new(&ctx);
    flick_methods.set_field(ctx, "drag_by", flick_drag);
    flick_methods.set_field(ctx, "release", flick_release);
    flick_methods.set_field(ctx, "tick", flick_tick);
    flick_methods.set_field(ctx, "position", flick_position);
    let flick_metatable = Table::new(&ctx);
    flick_metatable.set_field(ctx, "__index", flick_methods);
    let flick_metatable = ctx.stash(flick_metatable);
    let flickable = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let offset = table_number(ctx, options, "offset", 0.0).map_err(HostError)?;
        let minimum = table_number(ctx, options, "minimum", 0.0).map_err(HostError)?;
        let maximum = table_number(ctx, options, "maximum", 0.0).map_err(HostError)?;
        let deceleration =
            table_number(ctx, options, "deceleration", 2_500.0).map_err(HostError)?;
        if !offset.is_finite()
            || !minimum.is_finite()
            || !maximum.is_finite()
            || !deceleration.is_finite()
            || minimum > maximum
            || deceleration < 0.0
        {
            return Err(HostError("invalid flickable state".into()).into());
        }
        let userdata = UserData::new_static(
            &ctx,
            FlickToken {
                state: RefCell::new(FlickState {
                    offset: offset.clamp(minimum, maximum),
                    velocity: 0.0,
                    minimum,
                    maximum,
                    deceleration,
                }),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&flick_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "flickable", flickable);

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
                behavior: Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing,
                    rotation_direction: RotationDirection::Numerical,
                },
            });
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "transition_parent", transition_parent);
}
