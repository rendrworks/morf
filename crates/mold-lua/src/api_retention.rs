fn install_retention_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    mold: Table<'gc>,
    limits: Limits,
) {
    let retainable_lock = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let retainable: UserRef<RetainableToken> = stack.consume(ctx)?;
            let locks = state
                .borrow_mut()
                .retention
                .lock(retainable.node)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, i64::from(locks));
            Ok(CallbackReturn::Return)
        }
    });
    let retainable_unlock = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let retainable: UserRef<RetainableToken> = stack.consume(ctx)?;
            let (locks, destroy) = {
                let mut state = state.borrow_mut();
                let locks = state
                    .retention
                    .unlock(retainable.node)
                    .map_err(|error| HostError(error.to_string()))?;
                let destroy = state
                    .retention
                    .should_destroy(retainable.node)
                    .unwrap_or(false);
                (locks, destroy)
            };
            if destroy {
                finish_retained_destroy(&state, ctx, limits, retainable.node);
            }
            stack.replace(ctx, i64::from(locks));
            Ok(CallbackReturn::Return)
        }
    });
    let retainable_force_unlock = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let retainable: UserRef<RetainableToken> = stack.consume(ctx)?;
            let destroy = {
                let mut state = state.borrow_mut();
                state
                    .retention
                    .force_unlock(retainable.node)
                    .map_err(|error| HostError(error.to_string()))?;
                state
                    .retention
                    .should_destroy(retainable.node)
                    .unwrap_or(false)
            };
            if destroy {
                finish_retained_destroy(&state, ctx, limits, retainable.node);
            }
            Ok(CallbackReturn::Return)
        }
    });
    let retainable_retained = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let retainable: UserRef<RetainableToken> = stack.consume(ctx)?;
            let retained = state
                .borrow()
                .retention
                .state(retainable.node)
                .is_some_and(|state| state.dropped);
            stack.replace(ctx, retained);
            Ok(CallbackReturn::Return)
        }
    });
    let retainable_locks = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let retainable: UserRef<RetainableToken> = stack.consume(ctx)?;
            let locks = state
                .borrow()
                .retention
                .state(retainable.node)
                .map_or(0, |state| state.locks);
            stack.replace(ctx, i64::from(locks));
            Ok(CallbackReturn::Return)
        }
    });
    let retainable_methods = Table::new(&ctx);
    retainable_methods.set_field(ctx, "lock", retainable_lock);
    retainable_methods.set_field(ctx, "unlock", retainable_unlock);
    retainable_methods.set_field(ctx, "force_unlock", retainable_force_unlock);
    retainable_methods.set_field(ctx, "retained", retainable_retained);
    retainable_methods.set_field(ctx, "locks", retainable_locks);
    let retainable_metatable = Table::new(&ctx);
    retainable_metatable.set_field(ctx, "__index", retainable_methods);
    let retainable_metatable = ctx.stash(retainable_metatable);
    let retainable = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let retainable_metatable = retainable_metatable.clone();
        move |ctx, _, mut stack| {
            let (node, options): (UserRef<NodeToken>, LuaValue) = stack.consume(ctx)?;
            state
                .borrow()
                .scene
                .element(node.handle)
                .map_err(|error| HostError(error.to_string()))?;
            let mut callbacks = RetainCallbacks::default();
            let mut locked = false;
            match options {
                LuaValue::Nil => {}
                LuaValue::Table(options) => {
                    locked = table_bool(ctx, options, "locked", false).map_err(HostError)?;
                    callbacks.dropped =
                        optional_closure(ctx, options, "on_dropped").map_err(HostError)?;
                    callbacks.about_to_destroy =
                        optional_closure(ctx, options, "on_about_to_destroy").map_err(HostError)?;
                }
                _ => {
                    return Err(
                        HostError("retainable options must be a table or nil".into()).into(),
                    );
                }
            }
            {
                let mut state = state.borrow_mut();
                state.retention.register(node.handle);
                if locked {
                    state
                        .retention
                        .lock(node.handle)
                        .map_err(|error| HostError(error.to_string()))?;
                }
                state.retain_callbacks.insert(node.handle, callbacks);
            }
            let userdata = UserData::new_static(&ctx, RetainableToken { node: node.handle });
            userdata.set_metatable(ctx, Some(ctx.fetch(&retainable_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });

    let retain_lock_locked = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let lock: UserRef<RetainLockToken> = stack.consume(ctx)?;
        stack.replace(ctx, lock.locked.get());
        Ok(CallbackReturn::Return)
    });
    let retain_lock_set = Callback::from_fn(&ctx, {
        move |ctx, _, mut stack| {
            let (lock, locked): (UserRef<RetainLockToken>, bool) = stack.consume(ctx)?;
            if lock.locked.get() == locked {
                return Ok(CallbackReturn::Return);
            }
            let destroy = {
                let mut state = lock.state.borrow_mut();
                if locked {
                    state
                        .retention
                        .lock(lock.node)
                        .map_err(|error| HostError(error.to_string()))?;
                    false
                } else {
                    state
                        .retention
                        .unlock(lock.node)
                        .map_err(|error| HostError(error.to_string()))?;
                    state.retention.should_destroy(lock.node).unwrap_or(false)
                }
            };
            lock.locked.set(locked);
            if destroy {
                finish_retained_destroy(&lock.state, ctx, limits, lock.node);
            }
            Ok(CallbackReturn::Return)
        }
    });
    let retain_lock_retained = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let lock: UserRef<RetainLockToken> = stack.consume(ctx)?;
        let retained = lock
            .state
            .borrow()
            .retention
            .state(lock.node)
            .is_some_and(|state| state.dropped);
        stack.replace(ctx, retained);
        Ok(CallbackReturn::Return)
    });
    let retain_lock_methods = Table::new(&ctx);
    retain_lock_methods.set_field(ctx, "locked", retain_lock_locked);
    retain_lock_methods.set_field(ctx, "set_locked", retain_lock_set);
    retain_lock_methods.set_field(ctx, "retained", retain_lock_retained);
    let retain_lock_metatable = Table::new(&ctx);
    retain_lock_metatable.set_field(ctx, "__index", retain_lock_methods);
    let retain_lock_metatable = ctx.stash(retain_lock_metatable);
    let retain_lock = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (retainable, locked): (UserRef<RetainableToken>, LuaValue) = stack.consume(ctx)?;
            let locked = match locked {
                LuaValue::Nil => true,
                LuaValue::Boolean(locked) => locked,
                _ => return Err(HostError("retain lock state must be boolean".into()).into()),
            };
            if locked {
                state
                    .borrow_mut()
                    .retention
                    .lock(retainable.node)
                    .map_err(|error| HostError(error.to_string()))?;
            }
            let userdata = UserData::new_static(
                &ctx,
                RetainLockToken {
                    node: retainable.node,
                    locked: Cell::new(locked),
                    state: Rc::clone(&state),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&retain_lock_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });

    let effect = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (name, closure): (String, Closure) = stack.consume(ctx)?;
            {
                let mut state = state.borrow_mut();
                let token = state.next_effect;
                state.next_effect = state.next_effect.wrapping_add(1);
                state.effects.insert(
                    token,
                    LuaEffect {
                        closure: ctx.stash(closure),
                        sink: None,
                    },
                );
                state
                    .graph
                    .as_mut()
                    .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?
                    .external_effect(name, token);
            }
            replace_status(ctx, &mut stack, flush_reactive(&state, ctx, limits));
            Ok(CallbackReturn::Return)
        }
    });
    mold.set_field(ctx, "retainable", retainable);
    mold.set_field(ctx, "retain_lock", retain_lock);
    mold.set_field(ctx, "effect", effect);
}

