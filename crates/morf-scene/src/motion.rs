use crate::motion_values::*;
use animato::{Spring, SpringConfig, Tween, TweenState, Update};

use std::time::Duration;

use crate::{animation::*, types::*};

impl Animation {
    pub(crate) fn new(
        from: Value,
        to: Value,
        initial_velocity: Velocity,
        preserve_velocity: bool,
        behavior: Behavior,
    ) -> Self {
        let clock = Tween::new(0.0, 1.0)
            .duration(behavior.duration.as_secs_f32())
            .easing(behavior.easing.animato())
            .delay(behavior.delay.as_secs_f32())
            .time_scale(behavior.time_scale.max(0.0) as f32)
            .looping(behavior.repeat.animato())
            .build();
        Self {
            from,
            to,
            initial_velocity,
            preserve_velocity,
            clock,
            behavior,
        }
    }

    pub(crate) fn progress(&self) -> f64 {
        let progress = f64::from(self.clock.progress());
        if self.clock.is_ping_pong_reversed() {
            1.0 - progress
        } else {
            progress
        }
    }

    /// Reports whether the interval is waiting out its behavior delay.
    pub(crate) fn is_delayed(&self) -> bool {
        matches!(self.clock.state(), TweenState::Idle)
    }

    /// Reports whether playback is halted without having reached the target.
    pub(crate) fn is_paused(&self) -> bool {
        matches!(self.clock.state(), TweenState::Paused)
    }

    /// Reports whether the animation settles on its own.
    pub(crate) fn settles(&self) -> bool {
        !self.behavior.repeat.is_endless()
    }

    /// The value a settling animation comes to rest on.
    ///
    /// An alternating repetition that ends on a backward pass finishes where it
    /// started, so the resting value is not always the target.
    pub(crate) fn settled(&self) -> &Value {
        if self.clock.is_ping_pong_reversed() {
            &self.from
        } else {
            &self.to
        }
    }

    pub(crate) fn value(&self) -> Value {
        let progress = if self.preserve_velocity {
            self.progress()
        } else {
            f64::from(self.clock.value())
        };
        if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                self.behavior.duration.as_secs_f64(),
                progress,
                self.behavior.color_space,
                self.behavior.hue,
            )
        } else {
            interpolate_in(
                &self.from,
                &self.to,
                progress,
                self.behavior.color_space,
                self.behavior.hue,
            )
        }
    }

    pub(crate) fn velocity(&self) -> Velocity {
        let duration = self.behavior.duration.as_secs_f64();
        if duration == 0.0 {
            return zero_velocity(&self.to);
        }
        let progress = self.progress();
        let epsilon = (1.0 / (duration * 1_000.0)).clamp(1e-6, 1e-3);
        let before = (progress - epsilon).max(0.0);
        let after = (progress + epsilon).min(1.0);
        let span = (after - before) * duration;
        let (space, hue) = (self.behavior.color_space, self.behavior.hue);
        let before_value = if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                duration,
                before,
                space,
                hue,
            )
        } else {
            interpolate_in(
                &self.from,
                &self.to,
                self.behavior.easing.value_at(before),
                space,
                hue,
            )
        };
        let after_value = if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                duration,
                after,
                space,
                hue,
            )
        } else {
            interpolate_in(
                &self.from,
                &self.to,
                self.behavior.easing.value_at(after),
                space,
                hue,
            )
        };
        value_velocity(&before_value, &after_value, span, space, hue)
    }
}

pub(crate) fn interpolatable(from: &Value, to: &Value) -> bool {
    match (from, to) {
        (Value::Number(_), Value::Number(_)) | (Value::Color(_), Value::Color(_)) => true,
        // A tree of values — a gradient's stops — moves when the two trees
        // have the same shape and every leaf can.
        (Value::List(from), Value::List(to)) => {
            from.len() == to.len()
                && from
                    .iter()
                    .zip(to)
                    .all(|(from, to)| interpolatable(from, to) || from == to)
        }
        (Value::Map(from), Value::Map(to)) => {
            from.keys().eq(to.keys())
                && from
                    .values()
                    .zip(to.values())
                    .all(|(from, to)| interpolatable(from, to) || from == to)
        }
        _ => false,
    }
}

pub(crate) fn animation_start(
    property: &str,
    from: Value,
    to: &Value,
    direction: RotationDirection,
) -> Value {
    let (Value::Number(from_number), Value::Number(to_number)) = (&from, to) else {
        return from;
    };
    if property != "rotation" {
        return from;
    }
    let delta = match direction {
        RotationDirection::Numerical => return from,
        RotationDirection::Shortest => (to_number - from_number + 180.0).rem_euclid(360.0) - 180.0,
        RotationDirection::Clockwise => (to_number - from_number).rem_euclid(360.0),
        RotationDirection::CounterClockwise => -((from_number - to_number).rem_euclid(360.0)),
    };
    Value::Number(to_number - delta)
}

pub(crate) fn validate_physics(physics: Physics) -> Result<(), String> {
    match physics {
        Physics::Spring {
            mass,
            damping,
            stiffness,
            epsilon,
        } if mass.is_finite()
            && mass > 0.0
            && damping.is_finite()
            && damping >= 0.0
            && stiffness.is_finite()
            && stiffness > 0.0
            && epsilon.is_finite()
            && epsilon > 0.0 =>
        {
            Ok(())
        }
        Physics::Smoothed { velocity } if velocity.is_finite() && velocity > 0.0 => Ok(()),
        Physics::Decay {
            friction,
            min_velocity,
            bounds,
            gravity,
            restitution,
        } if friction.is_finite()
            && friction >= 0.0
            && min_velocity.is_finite()
            && min_velocity >= 0.0
            && gravity.is_finite()
            && restitution.is_finite()
            && (0.0..=1.0).contains(&restitution)
            && bounds
                .is_none_or(|(low, high)| low.is_finite() && high.is_finite() && low <= high) =>
        {
            Ok(())
        }
        Physics::Spring { .. } => Err("spring values must be finite and physically valid".into()),
        Physics::Smoothed { .. } => {
            Err("smoothed velocity must be finite and greater than zero".into())
        }
        Physics::Decay { .. } => Err(
            "decay needs finite friction, gravity and minimum velocity, a restitution between \
             zero and one, and ordered bounds"
                .into(),
        ),
    }
}

pub(crate) fn physics_animation(
    current: f64,
    target: f64,
    velocity: f64,
    spec: Physics,
) -> PhysicsAnimation {
    match spec {
        // Decay pursues nothing, so there is no target for an assignment to
        // set. It is installed by `Scene::fling`, which is why `set_physics`
        // refuses it and this arm cannot be reached.
        Physics::Decay { .. } => unreachable!("decay is started by a fling, not by an assignment"),
        Physics::Spring {
            mass,
            damping,
            stiffness,
            epsilon,
        } => PhysicsAnimation::Spring {
            target,
            motion: Spring::from_velocity(
                current as f32,
                velocity as f32,
                target as f32,
                SpringConfig {
                    stiffness: stiffness as f32,
                    damping: damping as f32,
                    mass: mass as f32,
                    epsilon: epsilon as f32,
                },
            ),
        },
        Physics::Smoothed { velocity: limit } => PhysicsAnimation::Smoothed {
            target,
            velocity,
            limit,
        },
    }
}

/// Whether a coast has run out of speed, given the slowest it may travel.
///
/// Strictly slower, so that a `min_velocity` of zero means what it says: no
/// speed is too slow, and the coast ends only when something else ends it.
/// That is the one way a configuration can say "this property is under physics
/// and currently still" — which is what anything driving a property by force
/// rather than by throw needs, because a coast that has ended takes no
/// impulses, and a property stopped dead this way could never be pushed again.
pub(crate) fn slow_enough_to_stop(velocity: f64, min_velocity: f64) -> bool {
    min_velocity > 0.0 && velocity.abs() <= min_velocity
}

pub(crate) fn advance_physics(
    motion: &mut PhysicsAnimation,
    current: &mut f64,
    delta: Duration,
) -> bool {
    let seconds = delta.as_secs_f64();
    match motion {
        PhysicsAnimation::Spring {
            target,
            motion: spring,
        } => {
            let steps = (seconds / (1.0 / 120.0)).ceil().max(1.0) as usize;
            let step = (seconds / steps as f64) as f32;
            let mut active = true;
            for _ in 0..steps {
                active = spring.update(step);
            }
            *current = f64::from(spring.position());
            if !active {
                *current = *target;
                true
            } else {
                false
            }
        }
        PhysicsAnimation::Decay {
            position,
            velocity,
            friction,
            gravity,
            restitution,
            min_velocity,
            bounds,
        } => {
            // Semi-implicit Euler: accelerate, then move. Friction opposes the
            // motion and may bring it to a stop within the step, so it is
            // clamped rather than allowed to push the other way.
            *velocity += *gravity * seconds;
            let drag = *friction * seconds;
            if drag >= velocity.abs() {
                *velocity = 0.0;
            } else {
                *velocity -= drag * velocity.signum();
            }
            *position += *velocity * seconds;

            let mut resting = false;
            if let Some((low, high)) = *bounds {
                // A bound returns the speed it did not absorb, which is what
                // makes it a bounce rather than a wall.
                if *position < low {
                    *position = low;
                    *velocity = -*velocity * *restitution;
                } else if *position > high {
                    *position = high;
                    *velocity = -*velocity * *restitution;
                }
                // At rest means: too slow to leave, against the bound that
                // gravity holds it to. Without gravity either end will do.
                let held = (*position - low).abs() < f64::EPSILON && *gravity < 0.0
                    || (*position - high).abs() < f64::EPSILON && *gravity > 0.0
                    || *gravity == 0.0
                        && ((*position - low).abs() < f64::EPSILON
                            || (*position - high).abs() < f64::EPSILON);
                resting = held && slow_enough_to_stop(*velocity, *min_velocity);
            }
            *current = *position;
            if resting {
                *velocity = 0.0;
                return true;
            }
            // Running out of speed ends a coast, but not a fall: with gravity
            // the next step will start it moving again.
            *gravity == 0.0 && slow_enough_to_stop(*velocity, *min_velocity)
        }
        PhysicsAnimation::Color { .. } => true,
        PhysicsAnimation::Smoothed {
            target,
            velocity,
            limit,
        } => {
            let distance = *target - *current;
            let step = *limit * seconds;
            if distance.abs() <= step {
                *current = *target;
                *velocity = 0.0;
                true
            } else {
                *velocity = limit.copysign(distance);
                *current += *velocity * seconds;
                false
            }
        }
    }
}

/// Whether layout reads this property.
///
/// Deliberately not [`property_class`]. That answers "what work does a change
/// need", and it calls `x` a transform because the renderer offsets by it —
/// but layout bakes `x` into the geometry it produces, and reads `border_width`
/// for a `ClipRect`'s content inset even though painting owns the border. Using
/// it here would let a moved node keep a stale layout.
///
/// The list is negative on purpose: everything counts unless it is known never
/// to be read, so a property added to the schema without a thought here costs
/// one extra layout pass rather than a frame drawn at the wrong geometry.
pub(crate) fn affects_layout(property: &str) -> bool {
    !matches!(
        property,
        "scale"
            | "scale_x"
            | "scale_y"
            | "skew_x"
            | "skew_y"
            | "translate_x"
            | "translate_y"
            | "transform_origin_x"
            | "transform_origin_y"
            | "rotation"
            | "opacity"
            | "color"
            | "color_overlay"
            | "layer"
            | "radius"
            | "top_left_radius"
            | "top_right_radius"
            | "bottom_right_radius"
            | "bottom_left_radius"
            | "border_color"
            | "antialiasing"
            | "border_pixel_aligned"
            | "content_under_border"
            | "gradient"
            | "blur"
            | "shadow_color"
            | "shadow_blur"
            | "shadow_spread"
            | "shadow_offset_x"
            | "shadow_offset_y"
            | "shadow_inner"
            | "morph_progress"
            | "blend"
            | "thickness"
            | "softness"
            | "outline_width"
            | "outline_color"
            | "fill_color"
            | "stroke_color"
    )
}
