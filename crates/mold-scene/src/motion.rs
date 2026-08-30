impl Animation {
    fn new(
        from: Value,
        to: Value,
        initial_velocity: Velocity,
        preserve_velocity: bool,
        behavior: Behavior,
    ) -> Self {
        let clock = Tween::new(0.0, 1.0)
            .duration(behavior.duration.as_secs_f32())
            .easing(behavior.easing.animato())
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

    fn progress(&self) -> f64 {
        f64::from(self.clock.progress())
    }

    fn value(&self) -> Value {
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
            )
        } else {
            interpolate(
                &self.from,
                &self.to,
                progress,
            )
        }
    }

    fn velocity(&self) -> Velocity {
        let duration = self.behavior.duration.as_secs_f64();
        if duration == 0.0 {
            return zero_velocity(&self.to);
        }
        let progress = self.progress();
        let epsilon = (1.0 / (duration * 1_000.0)).clamp(1e-6, 1e-3);
        let before = (progress - epsilon).max(0.0);
        let after = (progress + epsilon).min(1.0);
        let span = (after - before) * duration;
        let before_value = if self.preserve_velocity {
            interpolate_hermite(
                &self.from,
                &self.to,
                self.initial_velocity,
                duration,
                before,
            )
        } else {
            interpolate(&self.from, &self.to, self.behavior.easing.value_at(before))
        };
        let after_value = if self.preserve_velocity {
            interpolate_hermite(&self.from, &self.to, self.initial_velocity, duration, after)
        } else {
            interpolate(&self.from, &self.to, self.behavior.easing.value_at(after))
        };
        value_velocity(&before_value, &after_value, span)
    }
}

fn interpolatable(from: &Value, to: &Value) -> bool {
    matches!(
        (from, to),
        (Value::Number(_), Value::Number(_)) | (Value::Color(_), Value::Color(_))
    )
}

fn animation_start(
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
        RotationDirection::Shortest => {
            (to_number - from_number + 180.0).rem_euclid(360.0) - 180.0
        }
        RotationDirection::Clockwise => (to_number - from_number).rem_euclid(360.0),
        RotationDirection::CounterClockwise => {
            -((from_number - to_number).rem_euclid(360.0))
        }
    };
    Value::Number(to_number - delta)
}

fn validate_physics(physics: Physics) -> Result<(), String> {
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
        Physics::Spring { .. } => Err("spring values must be finite and physically valid".into()),
        Physics::Smoothed { .. } => {
            Err("smoothed velocity must be finite and greater than zero".into())
        }
    }
}

fn physics_animation(
    current: f64,
    target: f64,
    velocity: f64,
    spec: Physics,
) -> PhysicsAnimation {
    match spec {
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

fn advance_physics(motion: &mut PhysicsAnimation, current: &mut f64, delta: Duration) -> bool {
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

fn zero_velocity(value: &Value) -> Velocity {
    match value {
        Value::Color(_) => Velocity::Color([0.0; 4]),
        _ => Velocity::Number(0.0),
    }
}

fn value_velocity(from: &Value, to: &Value, seconds: f64) -> Velocity {
    let seconds = seconds.max(f64::EPSILON);
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Velocity::Number((to - from) / seconds),
        (Value::Color(from), Value::Color(to)) => Velocity::Color([
            (to.red as f64 - from.red as f64) / seconds,
            (to.green as f64 - from.green as f64) / seconds,
            (to.blue as f64 - from.blue as f64) / seconds,
            (to.alpha as f64 - from.alpha as f64) / seconds,
        ]),
        _ => Velocity::Number(0.0),
    }
}

fn interpolate(from: &Value, to: &Value, progress: f64) -> Value {
    match (from, to) {
        (Value::Number(from), Value::Number(to)) => Value::Number(from + (to - from) * progress),
        (Value::Color(from), Value::Color(to)) => Value::Color(Color {
            red: (from.red as f64 + (to.red as f64 - from.red as f64) * progress) as f32,
            green: (from.green as f64 + (to.green as f64 - from.green as f64) * progress) as f32,
            blue: (from.blue as f64 + (to.blue as f64 - from.blue as f64) * progress) as f32,
            alpha: (from.alpha as f64 + (to.alpha as f64 - from.alpha as f64) * progress) as f32,
        }),
        _ => to.clone(),
    }
}

fn interpolate_hermite(
    from: &Value,
    to: &Value,
    velocity: Velocity,
    duration: f64,
    progress: f64,
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
            let from = [from.red, from.green, from.blue, from.alpha].map(f64::from);
            let to = [to.red, to.green, to.blue, to.alpha].map(f64::from);
            let channels: [f32; 4] = std::array::from_fn(|index| {
                (from_weight * from[index]
                    + velocity_weight * duration * velocity[index]
                    + to_weight * to[index])
                    .clamp(0.0, 1.0) as f32
            });
            Value::Color(Color {
                red: channels[0],
                green: channels[1],
                blue: channels[2],
                alpha: channels[3],
            })
        }
        _ => interpolate(from, to, progress),
    }
}

fn property_class(property: &str) -> PropertyClass {
    match property {
        "x"
        | "y"
        | "scale"
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
        | "transition_x"
        | "transition_y" => PropertyClass::Transform,
        "color"
        | "color_overlay"
        | "layer"
        | "radius"
        | "top_left_radius"
        | "top_right_radius"
        | "bottom_right_radius"
        | "bottom_left_radius"
        | "border_width"
        | "border_color"
        | "antialiasing"
        | "border_pixel_aligned"
        | "content_under_border"
        | "gradient_start_color"
        | "gradient_end_color"
        | "gradient_start_x"
        | "gradient_start_y"
        | "gradient_end_x"
        | "gradient_end_y"
        | "gradient_center_x"
        | "gradient_center_y"
        | "gradient_radius"
        | "gradient_angle"
        | "blur"
        | "shadow_color"
        | "shadow_blur"
        | "shadow_spread"
        | "shadow_offset_x"
        | "shadow_offset_y"
        | "shadow_inner"
        | "path"
        | "morph_progress"
        | "fill_color"
        | "stroke_color"
        | "stroke_width" => PropertyClass::Paint,
        _ => PropertyClass::Layout,
    }
}
