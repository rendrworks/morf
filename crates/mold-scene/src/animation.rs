use animato::{Spring, Tween};
use mold_reactive::GraphError;
use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

use crate::{groups::*, motion::*, types::*};

/// Frame-pipeline work invalidated by an animated property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyClass {
    /// Node transform or compositor uniform only.
    Transform,
    /// Geometry requiring a layout pass.
    Layout,
    /// Draw-list data requiring repaint only.
    Paint,
}

/// Easing applied to a property behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Easing {
    /// Constant interpolation rate.
    #[default]
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    /// Cubic acceleration.
    InCubic,
    /// Cubic deceleration.
    OutCubic,
    /// Cubic acceleration followed by deceleration.
    InOutCubic,
    InQuart,
    OutQuart,
    InOutQuart,
    InQuint,
    OutQuint,
    InOutQuint,
    InSine,
    OutSine,
    InOutSine,
    InExpo,
    OutExpo,
    InOutExpo,
    InCirc,
    OutCirc,
    InOutCirc,
    InBack,
    OutBack,
    InOutBack,
    InBounce,
    OutBounce,
    InOutBounce,
    /// CSS-style cubic Bezier timing curve.
    CubicBezier {
        /// First control point x coordinate.
        x1: f64,
        /// First control point y coordinate.
        y1: f64,
        /// Second control point x coordinate.
        x2: f64,
        /// Second control point y coordinate.
        y2: f64,
    },
}

impl Easing {
    pub fn value_at(self, progress: f64) -> f64 {
        f64::from(self.animato().apply(progress.clamp(0.0, 1.0) as f32))
    }

    pub fn interpolate(self, progress: f64, start: f64, end: f64) -> f64 {
        start + (end - start) * self.value_at(progress)
    }

    /// Applies the curve once and interpolates both point components with it.
    pub fn interpolate_point(self, progress: f64, start: [f64; 2], end: [f64; 2]) -> [f64; 2] {
        let eased = self.value_at(progress);
        std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * eased)
    }

    /// Applies the curve once and interpolates an x, y, width, height rectangle.
    pub fn interpolate_rect(self, progress: f64, start: [f64; 4], end: [f64; 4]) -> [f64; 4] {
        let eased = self.value_at(progress);
        std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * eased)
    }

    /// Applies the curve once and interpolates every colour channel with it.
    pub fn interpolate_color(self, progress: f64, start: Color, end: Color) -> Color {
        let eased = self.value_at(progress);
        let Value::Color(color) = interpolate(&Value::Color(start), &Value::Color(end), eased)
        else {
            unreachable!("colour interpolation produced a non-colour value")
        };
        color
    }

    pub(crate) fn animato(self) -> animato::Easing {
        match self {
            Self::Linear => animato::Easing::Linear,
            Self::InQuad => animato::Easing::EaseInQuad,
            Self::OutQuad => animato::Easing::EaseOutQuad,
            Self::InOutQuad => animato::Easing::EaseInOutQuad,
            Self::InCubic => animato::Easing::EaseInCubic,
            Self::OutCubic => animato::Easing::EaseOutCubic,
            Self::InOutCubic => animato::Easing::EaseInOutCubic,
            Self::InQuart => animato::Easing::EaseInQuart,
            Self::OutQuart => animato::Easing::EaseOutQuart,
            Self::InOutQuart => animato::Easing::EaseInOutQuart,
            Self::InQuint => animato::Easing::EaseInQuint,
            Self::OutQuint => animato::Easing::EaseOutQuint,
            Self::InOutQuint => animato::Easing::EaseInOutQuint,
            Self::InSine => animato::Easing::EaseInSine,
            Self::OutSine => animato::Easing::EaseOutSine,
            Self::InOutSine => animato::Easing::EaseInOutSine,
            Self::InExpo => animato::Easing::EaseInExpo,
            Self::OutExpo => animato::Easing::EaseOutExpo,
            Self::InOutExpo => animato::Easing::EaseInOutExpo,
            Self::InCirc => animato::Easing::EaseInCirc,
            Self::OutCirc => animato::Easing::EaseOutCirc,
            Self::InOutCirc => animato::Easing::EaseInOutCirc,
            Self::InBack => animato::Easing::EaseInBack,
            Self::OutBack => animato::Easing::EaseOutBack,
            Self::InOutBack => animato::Easing::EaseInOutBack,
            Self::InBounce => animato::Easing::EaseInBounce,
            Self::OutBounce => animato::Easing::EaseOutBounce,
            Self::InOutBounce => animato::Easing::EaseInOutBounce,
            Self::CubicBezier { x1, y1, x2, y2 } => {
                animato::Easing::CubicBezier(x1 as f32, y1 as f32, x2 as f32, y2 as f32)
            }
        }
    }
}

/// Angular path used to interpolate rotation values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RotationDirection {
    /// Interpolate the numeric values directly.
    #[default]
    Numerical,
    /// Use the smallest angular displacement.
    Shortest,
    /// Increase the angle until the target orientation is reached.
    Clockwise,
    /// Decrease the angle until the target orientation is reached.
    CounterClockwise,
}

/// Playback repetition applied to a timed animation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Repeat {
    /// Run the interval once and settle on the target.
    #[default]
    Once,
    /// Run the interval a fixed number of times.
    Times(u32),
    /// Restart the interval from its start value forever.
    Forever,
    /// Alternate forward and backward passes forever.
    PingPong,
    /// Alternate forward and backward for a fixed number of passes.
    PingPongTimes(u32),
}

impl Repeat {
    /// Reports whether the repetition never settles on its own.
    pub fn is_endless(self) -> bool {
        matches!(self, Self::Forever | Self::PingPong)
    }

    pub(crate) fn animato(self) -> animato::Loop {
        match self {
            Self::Once => animato::Loop::Once,
            Self::Times(count) => animato::Loop::Times(count.max(1)),
            Self::Forever => animato::Loop::Forever,
            Self::PingPong => animato::Loop::PingPong,
            Self::PingPongTimes(count) => animato::Loop::PingPongTimes(count.max(1)),
        }
    }
}

/// Write interceptor installed on an animatable property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Behavior {
    /// Time from current value to the new target.
    pub duration: Duration,
    /// Timing curve for uninterrupted motion.
    pub easing: Easing,
    /// Angular path used when the property is rotation.
    pub rotation_direction: RotationDirection,
    /// Dead time before the interval starts advancing.
    pub delay: Duration,
    /// Multiplier applied to every frame delta.
    pub time_scale: f64,
    /// Repetition applied once the interval reaches its end.
    pub repeat: Repeat,
    /// Whether the interceptor animates writes at all.
    pub enabled: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            duration: Duration::ZERO,
            easing: Easing::Linear,
            rotation_direction: RotationDirection::Numerical,
            delay: Duration::ZERO,
            time_scale: 1.0,
            repeat: Repeat::Once,
            enabled: true,
        }
    }
}

impl Behavior {
    /// Builds an eased behavior with default delay, scaling, and repetition.
    pub fn timed(duration: Duration, easing: Easing) -> Self {
        Self {
            duration,
            easing,
            ..Self::default()
        }
    }

    /// Reports whether this behavior can intercept a write.
    pub(crate) fn intercepts(self) -> bool {
        self.enabled && (self.duration > Duration::ZERO || self.delay > Duration::ZERO)
    }
}

/// Physics-driven motion installed on a numeric property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Physics {
    /// Damped spring that retains velocity when its target changes.
    Spring {
        /// Moving mass.
        mass: f64,
        /// Velocity damping coefficient.
        damping: f64,
        /// Restoring-force coefficient.
        stiffness: f64,
        /// Position and velocity threshold used to settle.
        epsilon: f64,
    },
    /// Constant-speed pursuit of the latest target.
    Smoothed {
        /// Maximum property units travelled per second.
        velocity: f64,
    },
    /// Friction that coasts to a halt from whatever velocity it is given.
    ///
    /// The odd one out: a spring and a smoothing both *pursue a target*, so a
    /// plain assignment is enough to start them. Decay has no target — it is
    /// given a velocity and stops where friction leaves it — so it is started
    /// by [`Scene::fling`] rather than by writing a value.
    Decay {
        /// Deceleration opposing motion, in property units per second squared.
        friction: f64,
        /// Speed below which the motion is considered stopped.
        min_velocity: f64,
        /// Inclusive limits the motion is caught by, if any.
        bounds: Option<(f64, f64)>,
        /// Constant acceleration along the property, in units per second
        /// squared. Positive pulls towards larger values, which for `y` is
        /// downwards.
        gravity: f64,
        /// How much speed survives hitting a bound: zero stops dead, one
        /// returns at the speed it arrived, values between lose energy the way
        /// a real bounce does.
        restitution: f64,
    },
}

/// Why an animation left the active set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationEnd {
    /// The interval or physics motion reached its target.
    Completed,
    /// Playback was halted and the target was pinned to the current value.
    Stopped,
    /// The animation was discarded before reaching its target.
    Canceled,
}

/// One animation that left the active set during or between ticks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationEvent {
    /// Node that owned the animation.
    pub node: NodeHandle,
    /// Property name.
    pub property: &'static str,
    /// Why the animation ended.
    pub end: AnimationEnd,
}

/// One property changed by an animation tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimatedChange {
    /// Node whose current property value changed.
    pub node: NodeHandle,
    /// Property name.
    pub property: &'static str,
    /// Frame-pipeline work required by the property.
    pub class: PropertyClass,
}

/// Result of a pure-Rust animation clock tick.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AnimationFrame {
    /// Properties whose current level advanced.
    /// How many property values this tick moved.
    ///
    /// A count, not a list. Each entry used to name the node, the property and
    /// a `PropertyClass` worked out per property per frame — and every consumer
    /// only ever asked whether the list was empty. The classification was
    /// plainly meant to let a transform-only change skip the draw-list rebuild;
    /// nothing ever wired it up, so it was arithmetic done sixty times a second
    /// to fill in a field nobody read.
    pub changed: usize,
    /// Animations that ended during or since the previous tick.
    pub events: Vec<AnimationEvent>,
    /// Animation groups that ended during or since the previous tick.
    pub groups: Vec<GroupEvent>,
    /// Whether another compositor frame callback is required.
    pub active: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Animation {
    pub(crate) from: Value,
    pub(crate) to: Value,
    pub(crate) initial_velocity: Velocity,
    pub(crate) preserve_velocity: bool,
    pub(crate) clock: Tween<f32>,
    pub(crate) behavior: Behavior,
}

#[derive(Clone, Debug)]
pub(crate) enum PhysicsAnimation {
    Spring {
        target: f64,
        motion: Spring,
    },
    /// Free motion under friction, gravity and elastic bounds.
    ///
    /// Integrated here rather than by Animato's `Inertia`, which models
    /// friction alone: it stops dead when friction is zero, its bounds merely
    /// clamp, and it settles on speed — and the top of a bounce is momentarily
    /// still, so settling on speed would end the motion mid-flight. Animato's
    /// tuning still supplies the friction numbers behind the named presets.
    Decay {
        position: f64,
        velocity: f64,
        friction: f64,
        gravity: f64,
        restitution: f64,
        min_velocity: f64,
        bounds: Option<(f64, f64)>,
    },
    Smoothed {
        target: f64,
        velocity: f64,
        limit: f64,
    },
}

impl PhysicsAnimation {
    pub(crate) fn target(&self) -> f64 {
        match self {
            Self::Spring { target, .. } | Self::Smoothed { target, .. } => *target,
            // A fling is not going anywhere in particular, so where it has
            // reached is the only honest answer. Assigning a value mid-flight
            // hands this position, and the velocity below, to whatever takes
            // over — which is how a flick can be caught by a spring.
            Self::Decay { position, .. } => *position,
        }
    }

    pub(crate) fn velocity(&self) -> f64 {
        match self {
            Self::Spring { motion, .. } => f64::from(motion.velocity()),
            Self::Smoothed { velocity, .. } => *velocity,
            Self::Decay { velocity, .. } => *velocity,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Velocity {
    Number(f64),
    Color([f64; 4]),
}

impl Velocity {
    pub(crate) fn is_moving(self) -> bool {
        match self {
            Self::Number(value) => value.abs() > 1e-6,
            Self::Color(values) => values.into_iter().any(|value| value.abs() > 1e-6),
        }
    }
}

#[derive(Clone)]
pub(crate) struct PropertySpec {
    pub(crate) name: &'static str,
    pub(crate) kind: PropertyType,
    pub(crate) default: Value,
}

/// A scene graph operation failure.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneError {
    /// A node handle no longer refers to a live node.
    StaleNode,
    /// A node cannot become a child of itself or one of its descendants.
    ParentCycle,
    /// The named property is absent from the element schema.
    UnknownProperty {
        /// Element type receiving the assignment.
        element: &'static str,
        /// Rejected property name.
        property: String,
    },
    /// A property value cannot be converted to its declared type.
    InvalidPropertyType {
        /// Element type receiving the assignment.
        element: &'static str,
        /// Property whose coercion failed.
        property: String,
        /// Required property type.
        expected: &'static str,
    },
    /// The property signal graph rejected an operation.
    Reactive(String),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleNode => f.write_str("stale scene node handle"),
            Self::ParentCycle => f.write_str("a node cannot parent itself or its ancestor"),
            Self::UnknownProperty { element, property } => {
                write!(f, "unknown {element} property `{property}`")
            }
            Self::InvalidPropertyType {
                element,
                property,
                expected,
            } => write!(
                f,
                "invalid {element} property `{property}`: expected {expected}"
            ),
            Self::Reactive(message) => write!(f, "reactive property error: {message}"),
        }
    }
}

impl StdError for SceneError {}

impl From<GraphError> for SceneError {
    fn from(error: GraphError) -> Self {
        Self::Reactive(error.to_string())
    }
}
