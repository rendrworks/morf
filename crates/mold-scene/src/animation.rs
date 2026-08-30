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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Easing {
    /// Constant interpolation rate.
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

    fn animato(self) -> animato::Easing {
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

/// Write interceptor installed on an animatable property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Behavior {
    /// Time from current value to the new target.
    pub duration: Duration,
    /// Timing curve for uninterrupted motion.
    pub easing: Easing,
    /// Angular path used when the property is rotation.
    pub rotation_direction: RotationDirection,
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
    pub changes: Vec<AnimatedChange>,
    /// Whether another compositor frame callback is required.
    pub active: bool,
}

#[derive(Clone, Debug)]
struct Animation {
    from: Value,
    to: Value,
    initial_velocity: Velocity,
    preserve_velocity: bool,
    clock: Tween<f32>,
    behavior: Behavior,
}

#[derive(Clone, Debug)]
enum PhysicsAnimation {
    Spring {
        target: f64,
        motion: Spring,
    },
    Smoothed {
        target: f64,
        velocity: f64,
        limit: f64,
    },
}

impl PhysicsAnimation {
    fn velocity(&self) -> f64 {
        match self {
            Self::Spring { motion, .. } => f64::from(motion.velocity()),
            Self::Smoothed { velocity, .. } => *velocity,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Velocity {
    Number(f64),
    Color([f64; 4]),
}

impl Velocity {
    fn is_moving(self) -> bool {
        match self {
            Self::Number(value) => value.abs() > 1e-6,
            Self::Color(values) => values.into_iter().any(|value| value.abs() > 1e-6),
        }
    }
}

#[derive(Clone)]
struct PropertySpec {
    name: &'static str,
    kind: PropertyType,
    default: Value,
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
