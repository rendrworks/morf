fn configure_element<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    limits: Limits,
    node: NodeHandle,
    properties: Table<'gc>,
) -> Result<(), String> {
    let entries: Vec<_> = properties.iter(ctx).collect();
    let mut children = Vec::<(i64, NodeHandle)>::new();
    let mut named = Vec::<(String, LuaValue<'gc>)>::new();
    let mut state_value = None;
    for (key, value) in entries {
        match key {
            LuaValue::Integer(index) => {
                let LuaValue::UserData(child) = value else {
                    return Err(format!("child {index} must be a mold node"));
                };
                let child = child
                    .downcast_static::<NodeToken>()
                    .map_err(|_| format!("child {index} must be a mold node"))?;
                children.push((index, child.handle));
            }
            LuaValue::String(property) => {
                named.push((property.display_lossy().to_string(), value));
            }
            value => {
                return Err(format!(
                    "element table key must be a string or integer, found {}",
                    value.type_name()
                ));
            }
        }
    }
    if let Some((_, behavior)) = named.iter().find(|(name, _)| name == "behavior") {
        configure_behaviors(state, ctx, node, *behavior)?;
    }
    if let Some((_, states)) = named.iter().find(|(name, _)| name == "states") {
        let transitions = named
            .iter()
            .find(|(name, _)| name == "transitions")
            .map_or(LuaValue::Nil, |(_, value)| *value);
        configure_states(state, ctx, node, *states, transitions)?;
    }
    for (property, value) in named {
        if matches!(property.as_str(), "behavior" | "states" | "transitions") {
            continue;
        }
        if property == "state" {
            state_value = Some(value);
            continue;
        }
        if let Some(event) = handler_event(&property) {
            let LuaValue::Function(Function::Closure(closure)) = value else {
                return Err(format!("{property} must be a function"));
            };
            state
                .borrow_mut()
                .handlers
                .insert((node, event), ctx.stash(closure));
            continue;
        }
        if let LuaValue::Function(Function::Closure(closure)) = value {
            if !state
                .borrow()
                .scene
                .has_property(node, &property)
                .map_err(|error| error.to_string())?
            {
                let element = state
                    .borrow()
                    .scene
                    .element(node)
                    .map_err(|error| error.to_string())?;
                return Err(format!("unknown {element:?} property `{property}`"));
            }
            register_property_binding(state, ctx, limits, node, property, closure);
        } else {
            let value = lua_to_scene(ctx, value, 0)?;
            assign_scene_property(&mut state.borrow_mut(), node, &property, value)?;
        }
    }
    children.sort_by_key(|(index, _)| *index);
    for (_, child) in children {
        state
            .borrow_mut()
            .scene
            .reparent(child, Some(node))
            .map_err(|error| error.to_string())?;
    }
    if let Some(value) = state_value {
        match value {
            LuaValue::Function(Function::Closure(closure)) => {
                register_state_binding(state, ctx, limits, node, closure);
            }
            LuaValue::String(name) => {
                let mut remaining = limits.frame_fuel;
                apply_state(
                    state,
                    ctx,
                    limits,
                    &mut remaining,
                    node,
                    &name.display_lossy().to_string(),
                )?;
            }
            _ => return Err("state must be a string or binding function".into()),
        }
    }
    Ok(())
}

fn handler_event(property: &str) -> Option<UiEvent> {
    match property {
        "on_entered" => Some(UiEvent::PointerEntered),
        "on_exited" => Some(UiEvent::PointerExited),
        "on_position_changed" => Some(UiEvent::PointerMoved),
        "on_pressed" => Some(UiEvent::Pressed),
        "on_released" => Some(UiEvent::Released),
        "on_clicked" => Some(UiEvent::Clicked),
        "on_drag_started" => Some(UiEvent::DragStarted),
        "on_dragged" => Some(UiEvent::Dragged),
        "on_drag_finished" => Some(UiEvent::DragFinished),
        "on_wheel" => Some(UiEvent::Wheel),
        "on_key_pressed" => Some(UiEvent::KeyPressed),
        "on_touch_pressed" => Some(UiEvent::TouchPressed),
        "on_touch_moved" => Some(UiEvent::TouchMoved),
        "on_touch_released" => Some(UiEvent::TouchReleased),
        "on_touch_canceled" => Some(UiEvent::TouchCanceled),
        _ => None,
    }
}

fn configure_states<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: NodeHandle,
    states: LuaValue<'gc>,
    transitions: LuaValue<'gc>,
) -> Result<(), String> {
    let LuaValue::Table(states) = states else {
        return Err("states must be a name-keyed table".into());
    };
    let mut definitions = HashMap::new();
    for (name, definition) in states.iter(ctx) {
        let LuaValue::String(name) = name else {
            return Err("state names must be strings".into());
        };
        let LuaValue::Table(definition) = definition else {
            return Err("each state must be a table".into());
        };
        let mut properties = Vec::new();
        let mut anchors = None;
        let mut parent = None;
        for (key, value) in definition.iter(ctx) {
            let LuaValue::String(key) = key else {
                return Err("state fields must be strings".into());
            };
            match key.display_lossy().to_string().as_str() {
                "property_changes" => {
                    let LuaValue::Table(changes) = value else {
                        return Err("property_changes must be a table".into());
                    };
                    for (property, value) in changes.iter(ctx) {
                        let LuaValue::String(property) = property else {
                            return Err("property_changes keys must be strings".into());
                        };
                        let property = property.display_lossy().to_string();
                        if !state
                            .borrow()
                            .scene
                            .has_property(node, &property)
                            .map_err(|error| error.to_string())?
                        {
                            return Err(format!("state changes unknown property `{property}`"));
                        }
                        let value = match value {
                            LuaValue::Function(Function::Closure(closure)) => {
                                StateValue::Binding(ctx.stash(closure))
                            }
                            value => StateValue::Value(lua_to_scene(ctx, value, 0)?),
                        };
                        properties.push((property, value));
                    }
                }
                "anchors" | "anchor_changes" => {
                    let SceneValue::Map(value) = lua_to_scene(ctx, value, 0)? else {
                        return Err("anchor_changes must be a table".into());
                    };
                    anchors = Some(value);
                }
                "parent" | "parent_change" => {
                    let LuaValue::UserData(value) = value else {
                        return Err("parent_change must be a mold node".into());
                    };
                    parent = Some(
                        value
                            .downcast_static::<NodeToken>()
                            .map_err(|_| "parent_change must be a mold node".to_owned())?
                            .handle,
                    );
                }
                field => return Err(format!("unknown state field `{field}`")),
            }
        }
        definitions.insert(
            name.display_lossy().to_string(),
            StateDefinition {
                properties,
                anchors,
                parent,
            },
        );
    }
    let mut parsed_transitions = Vec::new();
    if let LuaValue::Table(transitions) = transitions {
        for (_, transition) in transitions.iter(ctx) {
            let LuaValue::Table(transition) = transition else {
                return Err("each transition must be a table".into());
            };
            let from = table_string(ctx, transition, "from", "*")?;
            let to = table_string(ctx, transition, "to", "*")?;
            let reversible = match transition.get_value(ctx, "reversible") {
                LuaValue::Nil => false,
                LuaValue::Boolean(value) => value,
                _ => return Err("transition reversible must be boolean".into()),
            };
            let duration = table_number(ctx, transition, "duration", 250.0)?;
            if duration < 0.0 {
                return Err("transition duration cannot be negative".into());
            }
            parsed_transitions.push(StateTransition {
                from,
                to,
                reversible,
                behavior: Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing: parse_easing(ctx, transition.get_value(ctx, "easing"))?,
                    rotation_direction: parse_rotation_direction(ctx, transition)?,
                },
            });
        }
    } else if !matches!(transitions, LuaValue::Nil) {
        return Err("transitions must be an array table".into());
    }
    state.borrow_mut().states.insert(
        node,
        StateSet {
            definitions,
            transitions: parsed_transitions,
            current: None,
        },
    );
    Ok(())
}

fn configure_behaviors<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: NodeHandle,
    value: LuaValue<'gc>,
) -> Result<(), String> {
    let LuaValue::Table(behaviors) = value else {
        return Err("behavior must be a property-keyed table".to_owned());
    };
    for (property, behavior) in behaviors.iter(ctx) {
        let LuaValue::String(property) = property else {
            return Err("behavior keys must be property names".to_owned());
        };
        let LuaValue::Table(behavior) = behavior else {
            return Err("each behavior must be a table".to_owned());
        };
        let property = property.display_lossy().to_string();
        let kind = match behavior.get_value(ctx, "kind") {
            LuaValue::Nil => None,
            LuaValue::String(value) => Some(value.display_lossy().to_string()),
            _ => return Err("behavior kind must be a string".to_owned()),
        };
        if kind.as_deref() == Some("spring") {
            let physics = Physics::Spring {
                mass: table_number(ctx, behavior, "mass", 1.0)?,
                damping: table_number(ctx, behavior, "damping", 18.0)?,
                stiffness: table_number(ctx, behavior, "stiffness", 180.0)?,
                epsilon: table_number(ctx, behavior, "epsilon", 0.001)?,
            };
            state
                .borrow_mut()
                .scene
                .set_physics(node, &property, Some(physics))
                .map_err(|error| error.to_string())?;
            continue;
        }
        if kind.as_deref() == Some("smoothed") {
            let physics = Physics::Smoothed {
                velocity: table_number(ctx, behavior, "velocity", 1_000.0)?,
            };
            state
                .borrow_mut()
                .scene
                .set_physics(node, &property, Some(physics))
                .map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(kind) = kind {
            return Err(format!("unknown behavior kind `{kind}`"));
        }
        let duration = match behavior.get_value(ctx, "duration") {
            LuaValue::Integer(value) => value as f64,
            LuaValue::Number(value) if value.is_finite() => value,
            _ => return Err("behavior duration must be milliseconds".to_owned()),
        };
        if duration < 0.0 {
            return Err("behavior duration cannot be negative".to_owned());
        }
        let easing = parse_easing(ctx, behavior.get_value(ctx, "easing"))?;
        let rotation_direction = parse_rotation_direction(ctx, behavior)?;
        state
            .borrow_mut()
            .scene
            .set_behavior(
                node,
                &property,
                Some(Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing,
                    rotation_direction,
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn parse_rotation_direction<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<RotationDirection, String> {
    match options.get_value(ctx, "rotation_direction") {
        LuaValue::Nil => Ok(RotationDirection::Numerical),
        LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
            "numerical" => Ok(RotationDirection::Numerical),
            "shortest" => Ok(RotationDirection::Shortest),
            "clockwise" => Ok(RotationDirection::Clockwise),
            "counterclockwise" => Ok(RotationDirection::CounterClockwise),
            _ => Err(
                "rotation_direction must be numerical, shortest, clockwise, or counterclockwise"
                    .to_owned(),
            ),
        },
        _ => Err("rotation_direction must be a string".to_owned()),
    }
}
