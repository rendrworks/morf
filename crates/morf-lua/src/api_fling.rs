use luna::{Callback, CallbackReturn, Context, Table, UserRef, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use morf_scene::Physics;

use crate::{scene_bindings::*, state::*};

/// Presets from Animato, named so a configuration need not tune friction by
/// hand for the ordinary cases.
pub(crate) fn fling_preset(name: &str) -> Option<(f64, f64)> {
    Some(match name {
        // Long-running, for scrolling a list or a canvas.
        "smooth" => (1400.0, 2.0),
        // Short and responsive, for something being directly manipulated.
        "snappy" => (3600.0, 4.0),
        // Slow to give up, for large panels.
        "heavy" => (800.0, 1.0),
        _ => return None,
    })
}

/// Installs `morf.animation.fling`, which sets a property coasting.
///
/// The counterpart to a behavior: a behavior says how a property travels to a
/// value it was given, and a fling has no value to travel to — it is thrown at
/// a speed and goes where the forces take it. That is what a flick wants, and
/// it is why this is a verb rather than another `kind` in a behavior table.
///
/// Beyond friction it carries `gravity`, a constant pull along the property,
/// and `bounce`, how much speed survives hitting a bound. A fling with gravity
/// does not stop when it runs out of speed — the top of an arc is momentarily
/// still — it stops when it comes to rest against the bound gravity holds it
/// to.
pub(crate) fn install_fling_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let pushes = Rc::clone(&state);
    let fling = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let table: Table = stack.consume(ctx)?;
        let LuaValue::UserData(node) = table.get_value(ctx, "node") else {
            return Err(HostError("a fling must name a node".into()).into());
        };
        let node = node
            .downcast_static::<NodeToken>()
            .map_err(|_| HostError("fling node must be a node".into()))?
            .handle;
        let LuaValue::String(property) = table.get_value(ctx, "property") else {
            return Err(HostError("a fling must name a property".into()).into());
        };
        let property = property.display_lossy().to_string();
        let velocity = match table.get_value(ctx, "velocity") {
            LuaValue::Integer(value) => value as f64,
            LuaValue::Number(value) => value,
            _ => {
                return Err(HostError("a fling must have a numeric velocity".into()).into());
            }
        };

        // A preset supplies both numbers; either may then be overridden.
        let (mut friction, mut min_velocity) = match table.get_value(ctx, "preset") {
            LuaValue::Nil => (1400.0, 2.0),
            LuaValue::String(name) => {
                let name = name.display_lossy().to_string();
                fling_preset(&name).ok_or_else(|| {
                    HostError(format!(
                        "unknown fling preset `{name}`; expected smooth, snappy, or heavy"
                    ))
                })?
            }
            _ => return Err(HostError("fling preset must be a string".into()).into()),
        };
        if let Some(value) = optional_number(ctx, table, "friction")? {
            friction = value;
        }
        if let Some(value) = optional_number(ctx, table, "min_velocity")? {
            min_velocity = value;
        }
        let gravity = optional_number(ctx, table, "gravity")?.unwrap_or(0.0);
        let restitution = optional_number(ctx, table, "bounce")?.unwrap_or(0.0);
        let low = optional_number(ctx, table, "min")?;
        let high = optional_number(ctx, table, "max")?;
        let bounds = match (low, high) {
            (Some(low), Some(high)) => Some((low, high)),
            (None, None) => None,
            _ => {
                return Err(HostError("a fling bound needs both `min` and `max`".into()).into());
            }
        };

        state
            .borrow_mut()
            .scene
            .fling(
                node,
                &property,
                velocity,
                Physics::Decay {
                    friction,
                    min_velocity,
                    bounds,
                    gravity,
                    restitution,
                },
            )
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, true);
        Ok(CallbackReturn::Return)
    });

    let animation: Table = match morf.get_value(ctx, "animation") {
        LuaValue::Table(animation) => animation,
        _ => unreachable!("the animation table is installed before its fling API"),
    };
    animation.set_field(ctx, "fling", fling);
    animation.set_field(ctx, "impulse", impulse(ctx, pushes));
}

/// Builds `morf.animation.impulse`, which pushes a coasting property.
///
/// A fling *sets* a speed; an impulse *adds* to it. That is the difference
/// between a flick and a force, and it is what lets a configuration supply
/// forces — one shape pulling on another, a drift towards the middle — without
/// owning the motion. It computes the push at whatever rate suits it and the
/// engine keeps integrating every frame in between; writing positions instead
/// would make the configuration the clock.
pub(crate) fn impulse<'gc>(ctx: Context<'gc>, state: Rc<RefCell<ReactiveState>>) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, property, delta): (UserRef<NodeToken>, String, f64) = stack.consume(ctx)?;
        let pushed = state
            .borrow_mut()
            .scene
            .impulse(node.handle, &property, delta)
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, pushed);
        Ok(CallbackReturn::Return)
    })
}

/// Reads an optional numeric field, rejecting anything that is not a number.
pub(crate) fn optional_number<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
) -> Result<Option<f64>, HostError> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(value) => Ok(Some(value as f64)),
        LuaValue::Number(value) => Ok(Some(value)),
        _ => Err(HostError(format!("fling {field} must be a number"))),
    }
}
