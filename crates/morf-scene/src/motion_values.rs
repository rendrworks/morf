//! A value between two others: interpolation, and the velocities that let
//! a motion hand over to the next one without a jolt. Numbers and colours
//! travel alike; a colour moves in the space its behavior names.

use std::time::Duration;

use crate::color::{ColorSpace, HueDirection};
use crate::motion::{advance_physics, physics_animation};
use crate::{animation::*, types::*};

pub(crate) fn zero_velocity(value: &Value) -> Velocity {
    match value {
        Value::Color(_) => Velocity::Color([0.0; 4]),
        _ => Velocity::Number(0.0),
    }
}

pub(crate) fn value_velocity(
    from: &Value,
    to: &Value,
    seconds: f64,
    space: ColorSpace,
    hue: HueDirection,
) -> Velocity {
    let seconds = seconds.max(f64::EPSILON);
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Velocity::Number((to - from) / seconds),
        (Value::Color(from), Value::Color(to)) => {
            let from = crate::color::coords(*from, space);
            let mut to = crate::color::coords(*to, space);
            if space == ColorSpace::Oklch {
                to[2] = crate::color::align_hue(from[2], to[2], hue);
            }
            Velocity::Color(std::array::from_fn(|index| {
                (to[index] - from[index]) / seconds
            }))
        }
        _ => Velocity::Number(0.0),
    }
}

/// A colour under a spring or a smoothing: the four premultiplied OkLab
/// components each carry the motion the spec describes.
pub(crate) fn physics_animation_color(
    current: Color,
    target: Color,
    velocity: [f64; 4],
    spec: Physics,
) -> PhysicsAnimation {
    let from = crate::color::coords(current, ColorSpace::Oklab);
    let to = crate::color::coords(target, ColorSpace::Oklab);
    PhysicsAnimation::Color {
        channels: Box::new(std::array::from_fn(|index| {
            physics_animation(from[index], to[index], velocity[index], spec)
        })),
    }
}

/// Advances a colour under physics; true once every component settled.
pub(crate) fn advance_physics_color(
    channels: &mut [PhysicsAnimation; 4],
    current: &mut Color,
    delta: Duration,
) -> bool {
    let mut coords = crate::color::coords(*current, ColorSpace::Oklab);
    let mut settled = true;
    for (index, channel) in channels.iter_mut().enumerate() {
        settled &= advance_physics(channel, &mut coords[index], delta);
    }
    *current = crate::color::from_coords(coords, ColorSpace::Oklab);
    settled
}

pub(crate) fn interpolate_in(
    from: &Value,
    to: &Value,
    progress: f64,
    space: ColorSpace,
    hue: HueDirection,
) -> Value {
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Value::Number(from + (to - from) * progress),
        (Value::Color(from), Value::Color(to)) => {
            Value::Color(crate::color::mix(*from, *to, progress, space, hue))
        }
        // A tree of values — a gradient's stops — moves leaf by leaf while the
        // two trees have the same shape, and snaps when they do not.
        (Value::List(from), Value::List(to)) if from.len() == to.len() => Value::List(
            from.iter()
                .zip(to)
                .map(|(from, to)| interpolate_in(from, to, progress, space, hue))
                .collect(),
        ),
        (Value::Map(from), Value::Map(to)) if from.keys().eq(to.keys()) => Value::Map(
            from.iter()
                .zip(to)
                .map(|((key, from), (_, to))| {
                    (key.clone(), interpolate_in(from, to, progress, space, hue))
                })
                .collect(),
        ),
        _ => to.clone(),
    }
}

pub(crate) fn interpolate_hermite(
    from: &Value,
    to: &Value,
    velocity: Velocity,
    duration: f64,
    progress: f64,
    space: ColorSpace,
    hue: HueDirection,
) -> Value {
    let t2 = progress * progress;
    let t3 = t2 * progress;
    let from_weight = 2.0 * t3 - 3.0 * t2 + 1.0;
    let velocity_weight = t3 - 2.0 * t2 + progress;
    let to_weight = -2.0 * t3 + 3.0 * t2;
    match (from, to, velocity) {
        (Value::Number(from), Value::Number(to), Velocity::Number(velocity)) => Value::Number(
            from_weight * from + velocity_weight * duration * velocity + to_weight * to,
        ),
        (Value::Color(from), Value::Color(to), Velocity::Color(velocity)) => {
            // Velocities are measured in the same space the value travels in.
            let from = crate::color::coords(*from, space);
            let mut to = crate::color::coords(*to, space);
            if space == ColorSpace::Oklch {
                to[2] = crate::color::align_hue(from[2], to[2], hue);
            }
            let channels: [f64; 4] = std::array::from_fn(|index| {
                from_weight * from[index]
                    + velocity_weight * duration * velocity[index]
                    + to_weight * to[index]
            });
            Value::Color(crate::color::from_coords(channels, space))
        }
        _ => interpolate_in(from, to, progress, space, hue),
    }
}
