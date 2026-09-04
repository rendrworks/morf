use crate::api_shader::attach_shader;
use crate::configure_states::configure_states;
use luna::{Context, Function, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use morf_scene::{Behavior, NodeHandle, Physics, Repeat, RotationDirection};

use crate::{
    events::*, lua_values::*, reactive_bindings::*, scene_bindings::*, state::*, table_menu::*,
    types::*,
};

pub(crate) fn configure_element<'gc>(
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
                    return Err(format!("child {index} must be a morf node"));
                };
                let child = child
                    .downcast_static::<NodeToken>()
                    .map_err(|_| format!("child {index} must be a morf node"))?;
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
    let named_behavior = named
        .iter()
        .find(|(name, _)| name == "behavior")
        .map(|(_, value)| *value);
    let mut state_selector = None;
    if let Some((_, states)) = named.iter().find(|(name, _)| name == "states") {
        let transitions = named
            .iter()
            .find(|(name, _)| name == "transitions")
            .map_or(LuaValue::Nil, |(_, value)| *value);
        state_selector = configure_states(state, ctx, limits, node, *states, transitions)?;
    }
    // A shader is resolved here rather than kept as a property: the name is
    // looked up once, at configuration time, so painting never consults a
    // registry and a name that does not resolve is reported where it was
    // written.
    if let Some((_, LuaValue::String(name))) = named.iter().find(|(key, _)| key == "shader") {
        let overrides = named
            .iter()
            .find(|(key, _)| key == "shader_params")
            .and_then(|(_, value)| match value {
                LuaValue::Table(table) => Some(*table),
                _ => None,
            });
        attach_shader(
            state,
            ctx,
            node,
            &name.display_lossy().to_string(),
            overrides,
        )?;
    }
    for (property, value) in named {
        if matches!(
            property.as_str(),
            "behavior" | "states" | "transitions" | "shader" | "shader_params"
        ) {
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
    // Behaviors are installed only once every declared property has been
    // assigned. A behavior intercepts writes, so installing it first would make
    // an element animate its own construction — every colour easing up from the
    // schema default, every width growing from zero — which is a flash on
    // startup, not a transition. Qt's `Behavior` withholds itself during
    // component construction for the same reason. Anything that changes after
    // this point, including the state applied below, animates normally.
    if let Some(behavior) = named_behavior {
        configure_behaviors(state, ctx, node, behavior)?;
    }
    children.sort_by_key(|(index, _)| *index);
    for (_, child) in children {
        state
            .borrow_mut()
            .scene
            .reparent(child, Some(node))
            .map_err(|error| error.to_string())?;
    }
    if let Some(selector) = state_selector {
        if state_value.is_some() {
            return Err("states with `when` choose themselves; drop `state`".into());
        }
        register_state_binding(state, ctx, limits, node, selector);
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

pub(crate) fn handler_event(property: &str) -> Option<UiEvent> {
    EVENT_PROPERTIES
        .iter()
        .find(|(_, name)| *name == property)
        .map(|(event, _)| *event)
}

pub(crate) fn configure_behaviors<'gc>(
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
        // Registered before the kind branches below return early, so spring and
        // smoothed motion report their settling the same way a tween does.
        match behavior.get_value(ctx, "on_finished") {
            LuaValue::Nil => {
                state
                    .borrow_mut()
                    .animation_callbacks
                    .remove(&(node, property.clone()));
            }
            LuaValue::Function(Function::Closure(callback)) => {
                let callback = ctx.stash(callback);
                state
                    .borrow_mut()
                    .animation_callbacks
                    .insert((node, property.clone()), callback);
            }
            _ => return Err("behavior on_finished must be a function".to_owned()),
        }
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
        let delay = table_number(ctx, behavior, "delay", 0.0)?;
        if delay < 0.0 {
            return Err("behavior delay cannot be negative".to_owned());
        }
        let time_scale = table_number(ctx, behavior, "time_scale", 1.0)?;
        if time_scale <= 0.0 {
            return Err("behavior time_scale must be greater than zero".to_owned());
        }
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
                    delay: Duration::from_secs_f64(delay / 1_000.0),
                    time_scale,
                    repeat: parse_repeat(ctx, behavior)?,
                    enabled: parse_enabled(ctx, behavior)?,
                    color_space: parse_color_space(ctx, behavior)?,
                    hue: parse_hue(ctx, behavior)?,
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Reads the `loops` and `ping_pong` pair into a repetition mode.
///
/// `loops` is either a pass count or one of the endless names, and `ping_pong`
/// turns whichever of those was given into an alternating variant. Lua reserves
/// `repeat` as a keyword, so the count field cannot carry that name.
pub(crate) fn parse_repeat<'gc>(ctx: Context<'gc>, options: Table<'gc>) -> Result<Repeat, String> {
    let alternating = match options.get_value(ctx, "ping_pong") {
        LuaValue::Nil => false,
        LuaValue::Boolean(value) => value,
        _ => return Err("behavior ping_pong must be boolean".to_owned()),
    };
    let count = |value: f64| -> Result<u32, String> {
        if !value.is_finite() || value < 1.0 {
            return Err("behavior loops must be at least one pass".to_owned());
        }
        Ok(value as u32)
    };
    match options.get_value(ctx, "loops") {
        LuaValue::Nil if alternating => Ok(Repeat::PingPong),
        LuaValue::Nil => Ok(Repeat::Once),
        LuaValue::Integer(value) => Ok(match alternating {
            true => Repeat::PingPongTimes(count(value as f64)?),
            false => Repeat::Times(count(value as f64)?),
        }),
        LuaValue::Number(value) => Ok(match alternating {
            true => Repeat::PingPongTimes(count(value)?),
            false => Repeat::Times(count(value)?),
        }),
        LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
            "once" => Ok(Repeat::Once),
            "forever" => Ok(Repeat::Forever),
            "ping_pong" => Ok(Repeat::PingPong),
            name => Err(format!("unknown behavior loops mode `{name}`")),
        },
        _ => Err("behavior loops must be a pass count or a mode name".to_owned()),
    }
}

/// Reads the optional `enabled` switch, defaulting an absent one to on.
pub(crate) fn parse_enabled<'gc>(ctx: Context<'gc>, options: Table<'gc>) -> Result<bool, String> {
    match options.get_value(ctx, "enabled") {
        LuaValue::Nil => Ok(true),
        LuaValue::Boolean(value) => Ok(value),
        _ => Err("behavior enabled must be boolean".to_owned()),
    }
}

/// `space = "srgb" | "oklab" | "oklch"`: where a colour travels.
pub(crate) fn parse_color_space<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<morf_scene::ColorSpace, String> {
    match options.get_value(ctx, "space") {
        LuaValue::Nil => Ok(morf_scene::ColorSpace::default()),
        LuaValue::String(value) => {
            morf_scene::ColorSpace::parse(&value.display_lossy().to_string())
                .ok_or_else(|| "space must be srgb, oklab or oklch".to_owned())
        }
        _ => Err("space must be a string".to_owned()),
    }
}

/// `hue = "shorter" | "longer"`: which way round the wheel in `oklch`.
pub(crate) fn parse_hue<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<morf_scene::HueDirection, String> {
    match options.get_value(ctx, "hue") {
        LuaValue::Nil => Ok(morf_scene::HueDirection::default()),
        LuaValue::String(value) => {
            morf_scene::HueDirection::parse(&value.display_lossy().to_string())
                .ok_or_else(|| "hue must be shorter or longer".to_owned())
        }
        _ => Err("hue must be a string".to_owned()),
    }
}

pub(crate) fn parse_rotation_direction<'gc>(
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
