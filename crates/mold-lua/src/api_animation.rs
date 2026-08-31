use luna::{Callback, CallbackReturn, Context, Table, UserRef, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use mold_scene::{AnimationEnd, Color, NodeHandle, Scene, SceneError, Value as SceneValue};

use crate::{lua_values::*, reactive_bindings::*, scene_bindings::*, serialization::*, state::*};

/// Installs `mold.animation`, the imperative control surface over motion that
/// a `behavior` table has already declared.
///
/// Every call names a node and one of its properties, because that pair is what
/// the scene keys an animation by. Calls report whether they found an animation
/// to act on, so Lua can branch without first asking whether one is running.
pub(crate) fn install_animation_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    mold: Table<'gc>,
) {
    let animation = Table::new(&ctx);

    // Each control reads the same (node, property) pair and returns a boolean,
    // so they are built from one closure factory rather than repeated by hand.
    let install_control =
        |name: &'static str,
         control: fn(&mut Scene, NodeHandle, &str) -> Result<bool, SceneError>| {
            let state = Rc::clone(&state);
            let callback = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
                let (node, property): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
                let found = control(&mut state.borrow_mut().scene, node.handle, &property)
                    .map_err(|error| HostError(error.to_string()))?;
                stack.replace(ctx, found);
                Ok(CallbackReturn::Return)
            });
            animation.set_field(ctx, name, callback);
        };
    install_control("stop", Scene::stop_animation);
    install_control("finish", Scene::finish_animation);
    install_control("restart", Scene::restart_animation);
    install_control("reverse", Scene::reverse_animation);
    install_control("pause", |scene, node, property| {
        scene.set_animation_paused(node, property, true)
    });
    install_control("resume", |scene, node, property| {
        scene.set_animation_paused(node, property, false)
    });

    let active = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (node, property): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
            let active = state
                .borrow()
                .scene
                .is_animating(node.handle, &property)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, active);
            Ok(CallbackReturn::Return)
        }
    });
    animation.set_field(ctx, "active", active);

    let paused = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (node, property): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
            let paused = state
                .borrow()
                .scene
                .is_animation_paused(node.handle, &property)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, paused);
            Ok(CallbackReturn::Return)
        }
    });
    animation.set_field(ctx, "paused", paused);

    // Nil rather than zero when nothing is running: a settled property and one
    // sitting at the start of its interval are not the same state.
    let progress = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (node, property): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
            let progress = state
                .borrow()
                .scene
                .animation_progress(node.handle, &property)
                .map_err(|error| HostError(error.to_string()))?;
            match progress {
                Some(progress) => stack.replace(ctx, progress),
                None => stack.replace(ctx, LuaValue::Nil),
            }
            Ok(CallbackReturn::Return)
        }
    });
    animation.set_field(ctx, "progress", progress);

    let seek = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (node, property, progress): (UserRef<NodeToken>, String, f64) =
                stack.consume(ctx)?;
            if !progress.is_finite() {
                return Err(HostError("animation seek position must be finite".into()).into());
            }
            let found = state
                .borrow_mut()
                .scene
                .seek_animation(node.handle, &property, progress)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, found);
            Ok(CallbackReturn::Return)
        }
    });
    animation.set_field(ctx, "seek", seek);

    // Toggling an installed behavior leaves its settings in place, so a shell
    // can suppress motion for one write without rebuilding the declaration.
    let set_enabled = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (node, property, enabled): (UserRef<NodeToken>, String, bool) =
                stack.consume(ctx)?;
            let found = state
                .borrow_mut()
                .scene
                .set_behavior_enabled(node.handle, &property, enabled)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, found);
            Ok(CallbackReturn::Return)
        }
    });
    animation.set_field(ctx, "set_enabled", set_enabled);

    mold.set_field(ctx, "animation", animation);
}

/// Installs `mold.easing`, direct evaluation of a timing curve.
///
/// A behavior covers motion the scene owns. These are for the cases it does
/// not: positioning something along a curve by hand, staggering a set of
/// values, or interpolating a compound the scene has no single property for.
/// Every entry takes the same curve names a behavior's `easing` field accepts,
/// including a cubic Bezier table.
pub(crate) fn install_easing_api<'gc>(ctx: Context<'gc>, mold: Table<'gc>) {
    let easing = Table::new(&ctx);

    let value = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (curve, progress): (LuaValue, f64) = stack.consume(ctx)?;
        let curve = parse_easing(ctx, curve).map_err(HostError)?;
        stack.replace(ctx, curve.value_at(finite(progress, "easing progress")?));
        Ok(CallbackReturn::Return)
    });
    easing.set_field(ctx, "value", value);

    let number = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (curve, progress, start, end): (LuaValue, f64, f64, f64) = stack.consume(ctx)?;
        let curve = parse_easing(ctx, curve).map_err(HostError)?;
        let progress = finite(progress, "easing progress")?;
        let start = finite(start, "easing start")?;
        let end = finite(end, "easing end")?;
        stack.replace(ctx, curve.interpolate(progress, start, end));
        Ok(CallbackReturn::Return)
    });
    easing.set_field(ctx, "number", number);

    // Point and rect share one curve across their components rather than
    // easing each axis separately, so a diagonal path stays a straight line.
    let point = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (curve, progress, start, end): (LuaValue, f64, Table, Table) = stack.consume(ctx)?;
        let curve = parse_easing(ctx, curve).map_err(HostError)?;
        let progress = finite(progress, "easing progress")?;
        let start = read_axes(ctx, start, &["x", "y"], "easing point")?;
        let end = read_axes(ctx, end, &["x", "y"], "easing point")?;
        let eased = curve.interpolate_point(progress, [start[0], start[1]], [end[0], end[1]]);
        let table = Table::new(&ctx);
        table.set_field(ctx, "x", eased[0]);
        table.set_field(ctx, "y", eased[1]);
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    easing.set_field(ctx, "point", point);

    let rect = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (curve, progress, start, end): (LuaValue, f64, Table, Table) = stack.consume(ctx)?;
        let curve = parse_easing(ctx, curve).map_err(HostError)?;
        let progress = finite(progress, "easing progress")?;
        let axes = ["x", "y", "width", "height"];
        let start = read_axes(ctx, start, &axes, "easing rect")?;
        let end = read_axes(ctx, end, &axes, "easing rect")?;
        let eased = curve.interpolate_rect(
            progress,
            [start[0], start[1], start[2], start[3]],
            [end[0], end[1], end[2], end[3]],
        );
        let table = Table::new(&ctx);
        for (name, value) in axes.iter().zip(eased) {
            table.set_field(ctx, name, value);
        }
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    easing.set_field(ctx, "rect", rect);

    let color = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (curve, progress, start, end): (LuaValue, f64, LuaValue, LuaValue) =
            stack.consume(ctx)?;
        let curve = parse_easing(ctx, curve).map_err(HostError)?;
        let progress = finite(progress, "easing progress")?;
        // Accepts either form a colour property does: a `#rrggbb` style string
        // or the `{ r, g, b, a }` table a colour reads back as.
        let read = |value| match lua_to_scene(ctx, value, 0).map_err(HostError)? {
            SceneValue::Color(color) => Ok(color),
            SceneValue::String(name) => {
                Color::parse(&name).ok_or_else(|| HostError(format!("`{name}` is not a colour")))
            }
            _ => Err(HostError("easing colour must be a colour".into())),
        };
        let eased = curve.interpolate_color(progress, read(start)?, read(end)?);
        stack.replace(
            ctx,
            scene_to_lua(ctx, &SceneValue::Color(eased)).map_err(HostError)?,
        );
        Ok(CallbackReturn::Return)
    });
    easing.set_field(ctx, "color", color);

    mold.set_field(ctx, "easing", easing);
}

/// Rejects a non-finite argument before it reaches a curve.
pub(crate) fn finite(value: f64, what: &str) -> Result<f64, HostError> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| HostError(format!("{what} must be finite")))
}

/// Reads a fixed set of numeric fields from a Lua table, defaulting absent ones.
pub(crate) fn read_axes<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    axes: &[&str],
    what: &str,
) -> Result<Vec<f64>, HostError> {
    axes.iter()
        .map(|axis| match table.get_value(ctx, *axis) {
            LuaValue::Nil => Ok(0.0),
            LuaValue::Integer(value) => Ok(value as f64),
            LuaValue::Number(value) if value.is_finite() => Ok(value),
            _ => Err(HostError(format!(
                "{what} `{axis}` must be a finite number"
            ))),
        })
        .collect()
}

/// Names the reason an animation ended for the Lua callback that receives it.
pub(crate) fn animation_end_name(end: AnimationEnd) -> &'static str {
    match end {
        AnimationEnd::Completed => "completed",
        AnimationEnd::Stopped => "stopped",
        AnimationEnd::Canceled => "canceled",
    }
}
