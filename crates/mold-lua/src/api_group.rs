/// Deepest nesting a group specification may reach.
///
/// Configuration is parsed recursively, so the depth is capped rather than
/// left to exhaust the stack on a table that nests itself.
const MAX_GROUP_DEPTH: usize = 8;

/// Installs `mold.animation.play`, which schedules a group of property steps.
///
/// The returned handle carries the controls, so a caller holds one object for
/// the whole schedule instead of naming every property it touches.
fn install_group_api<'gc>(ctx: Context<'gc>, state: Rc<RefCell<ReactiveState>>, mold: Table<'gc>) {
    let methods = Table::new(&ctx);

    let control = |name: &'static str, control: fn(&mut Scene, GroupId) -> bool| {
        let state = Rc::clone(&state);
        let callback = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let handle: UserRef<GroupToken> = stack.consume(ctx)?;
            let acted = control(&mut state.borrow_mut().scene, handle.id);
            stack.replace(ctx, acted);
            Ok(CallbackReturn::Return)
        });
        methods.set_field(ctx, name, callback);
    };
    control("stop", Scene::stop_group);
    control("active", |scene, group| scene.is_group_active(group));
    control("pause", |scene, group| scene.set_group_paused(group, true));
    control("resume", |scene, group| {
        scene.set_group_paused(group, false)
    });

    let finish = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let handle: UserRef<GroupToken> = stack.consume(ctx)?;
            let finished = state
                .borrow_mut()
                .scene
                .finish_group(handle.id)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, finished);
            Ok(CallbackReturn::Return)
        }
    });
    methods.set_field(ctx, "finish", finish);

    let metatable = Table::new(&ctx);
    metatable.set_field(ctx, "__index", methods);
    let metatable = ctx.stash(metatable);

    let play = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            // An array of steps at the top level reads as a sequence, which is
            // what a group written out in order is nearly always meant to be.
            let step = parse_group_children(ctx, options, 0)
                .map(AnimationStep::Sequential)
                .map_err(HostError)?;
            let repeat = parse_repeat(ctx, options).map_err(HostError)?;
            let callback = match options.get_value(ctx, "on_finished") {
                LuaValue::Nil => None,
                LuaValue::Function(Function::Closure(callback)) => Some(ctx.stash(callback)),
                _ => {
                    return Err(
                        HostError("animation group on_finished must be a function".into()).into(),
                    );
                }
            };
            let id = {
                let mut state = state.borrow_mut();
                let id = state
                    .scene
                    .start_group(step, repeat)
                    .map_err(|error| HostError(error.to_string()))?;
                if let Some(callback) = callback {
                    state.group_callbacks.insert(id, callback);
                }
                id
            };
            let value = UserData::new_static(&ctx, GroupToken { id });
            value.set_metatable(ctx, Some(ctx.fetch(&metatable)));
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });

    let animation: Table = match mold.get_value(ctx, "animation") {
        LuaValue::Table(animation) => animation,
        _ => unreachable!("the animation table is installed before its group API"),
    };
    animation.set_field(ctx, "play", play);
}

/// Reads the array part of a table as an ordered list of steps.
fn parse_group_children<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    depth: usize,
) -> Result<Vec<AnimationStep>, String> {
    if depth > MAX_GROUP_DEPTH {
        return Err(format!(
            "animation group nests deeper than {MAX_GROUP_DEPTH} levels"
        ));
    }
    // Walked as a dense sequence so the declared order is the played order; the
    // named fields alongside it are options, not steps.
    let mut steps = Vec::new();
    for index in 1.. {
        match table.get(ctx, index).map_err(|error| error.to_string())? {
            LuaValue::Nil => break,
            LuaValue::Table(child) => steps.push(parse_group_step(ctx, child, depth)?),
            _ => return Err("each animation group step must be a table".to_owned()),
        }
    }
    Ok(steps)
}

/// Reads one step, dispatching on which shape of step the table describes.
fn parse_group_step<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    depth: usize,
) -> Result<AnimationStep, String> {
    match table.get_value(ctx, "pause") {
        LuaValue::Nil => {}
        LuaValue::Integer(millis) => {
            return Ok(AnimationStep::Pause(milliseconds(millis as f64, "pause")?));
        }
        LuaValue::Number(millis) => {
            return Ok(AnimationStep::Pause(milliseconds(millis, "pause")?));
        }
        _ => return Err("animation group pause must be milliseconds".to_owned()),
    }
    for (field, wrap) in [
        ("parallel", AnimationStep::Parallel as fn(_) -> _),
        ("sequential", AnimationStep::Sequential as fn(_) -> _),
    ] {
        match table.get_value(ctx, field) {
            LuaValue::Nil => {}
            LuaValue::Table(children) => {
                return Ok(wrap(parse_group_children(ctx, children, depth + 1)?));
            }
            _ => return Err(format!("animation group {field} must be an array table")),
        }
    }

    let LuaValue::UserData(node) = table.get_value(ctx, "node") else {
        return Err("an animation group step must name a node".to_owned());
    };
    let node = node
        .downcast_static::<NodeToken>()
        .map_err(|_| "animation group node must be a node".to_owned())?
        .handle;
    let LuaValue::String(property) = table.get_value(ctx, "property") else {
        return Err("an animation group step must name a property".to_owned());
    };
    let to = match table.get_value(ctx, "to") {
        LuaValue::Nil => return Err("an animation group step must have a `to` value".to_owned()),
        value => lua_to_scene(ctx, value, 0)?,
    };
    let from = match table.get_value(ctx, "from") {
        LuaValue::Nil => None,
        value => Some(lua_to_scene(ctx, value, 0)?),
    };
    Ok(AnimationStep::Property {
        node,
        property: property.display_lossy().to_string(),
        from,
        to,
        behavior: parse_step_behavior(ctx, table)?,
    })
}

/// Reads the timing fields a step shares with a declared `behavior`.
fn parse_step_behavior<'gc>(ctx: Context<'gc>, table: Table<'gc>) -> Result<Behavior, String> {
    let duration = match table.get_value(ctx, "duration") {
        LuaValue::Integer(value) => value as f64,
        LuaValue::Number(value) => value,
        LuaValue::Nil => return Err("an animation group step must have a duration".to_owned()),
        _ => return Err("animation group duration must be milliseconds".to_owned()),
    };
    let time_scale = table_number(ctx, table, "time_scale", 1.0)?;
    if time_scale <= 0.0 {
        return Err("animation group time_scale must be greater than zero".to_owned());
    }
    Ok(Behavior {
        duration: milliseconds(duration, "duration")?,
        easing: parse_easing(ctx, table.get_value(ctx, "easing"))?,
        rotation_direction: parse_rotation_direction(ctx, table)?,
        delay: milliseconds(table_number(ctx, table, "delay", 0.0)?, "delay")?,
        time_scale,
        repeat: parse_repeat(ctx, table)?,
        enabled: true,
    })
}

/// Converts a Lua millisecond count into a duration, rejecting nonsense.
fn milliseconds(value: f64, what: &str) -> Result<Duration, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "animation group {what} must be a non-negative number of milliseconds"
        ));
    }
    Ok(Duration::from_secs_f64(value / 1_000.0))
}
