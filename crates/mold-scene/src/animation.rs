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
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => progress,
            Self::InQuad => progress * progress,
            Self::OutQuad => 1.0 - (1.0 - progress).powi(2),
            Self::InOutQuad if progress < 0.5 => 2.0 * progress * progress,
            Self::InOutQuad => 1.0 - (-2.0 * progress + 2.0).powi(2) / 2.0,
            Self::InCubic => progress.powi(3),
            Self::OutCubic => 1.0 - (1.0 - progress).powi(3),
            Self::InOutCubic if progress < 0.5 => 4.0 * progress.powi(3),
            Self::InOutCubic => 1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0,
            Self::InQuart => progress.powi(4),
            Self::OutQuart => 1.0 - (1.0 - progress).powi(4),
            Self::InOutQuart if progress < 0.5 => 8.0 * progress.powi(4),
            Self::InOutQuart => 1.0 - (-2.0 * progress + 2.0).powi(4) / 2.0,
            Self::InQuint => progress.powi(5),
            Self::OutQuint => 1.0 - (1.0 - progress).powi(5),
            Self::InOutQuint if progress < 0.5 => 16.0 * progress.powi(5),
            Self::InOutQuint => 1.0 - (-2.0 * progress + 2.0).powi(5) / 2.0,
            Self::InSine => 1.0 - (progress * std::f64::consts::FRAC_PI_2).cos(),
            Self::OutSine => (progress * std::f64::consts::FRAC_PI_2).sin(),
            Self::InOutSine => -((std::f64::consts::PI * progress).cos() - 1.0) / 2.0,
            Self::InExpo if progress == 0.0 => 0.0,
            Self::InExpo => 2.0_f64.powf(10.0 * progress - 10.0),
            Self::OutExpo if progress == 1.0 => 1.0,
            Self::OutExpo => 1.0 - 2.0_f64.powf(-10.0 * progress),
            Self::InOutExpo if progress == 0.0 || progress == 1.0 => progress,
            Self::InOutExpo if progress < 0.5 => 2.0_f64.powf(20.0 * progress - 10.0) / 2.0,
            Self::InOutExpo => (2.0 - 2.0_f64.powf(-20.0 * progress + 10.0)) / 2.0,
            Self::InCirc => 1.0 - (1.0 - progress.powi(2)).sqrt(),
            Self::OutCirc => (1.0 - (progress - 1.0).powi(2)).sqrt(),
            Self::InOutCirc if progress < 0.5 => {
                (1.0 - (1.0 - (2.0 * progress).powi(2)).sqrt()) / 2.0
            }
            Self::InOutCirc => ((1.0 - (-2.0 * progress + 2.0).powi(2)).sqrt() + 1.0) / 2.0,
            Self::InBack => {
                const C1: f64 = 1.70158;
                (C1 + 1.0) * progress.powi(3) - C1 * progress.powi(2)
            }
            Self::OutBack => {
                const C1: f64 = 1.70158;
                1.0 + (C1 + 1.0) * (progress - 1.0).powi(3) + C1 * (progress - 1.0).powi(2)
            }
            Self::InOutBack => {
                const C2: f64 = 1.70158 * 1.525;
                if progress < 0.5 {
                    (2.0 * progress).powi(2) * ((C2 + 1.0) * 2.0 * progress - C2) / 2.0
                } else {
                    ((2.0 * progress - 2.0).powi(2) * ((C2 + 1.0) * (progress * 2.0 - 2.0) + C2)
                        + 2.0)
                        / 2.0
                }
            }
            Self::InBounce => 1.0 - out_bounce(1.0 - progress),
            Self::OutBounce => out_bounce(progress),
            Self::InOutBounce if progress < 0.5 => (1.0 - out_bounce(1.0 - 2.0 * progress)) / 2.0,
            Self::InOutBounce => (1.0 + out_bounce(2.0 * progress - 1.0)) / 2.0,
            Self::CubicBezier { .. } if progress == 0.0 || progress == 1.0 => progress,
            Self::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(progress, x1, y1, x2, y2),
        }
    }

    pub fn interpolate(self, progress: f64, start: f64, end: f64) -> f64 {
        start + (end - start) * self.value_at(progress)
    }
}

fn out_bounce(progress: f64) -> f64 {
    const N1: f64 = 7.5625;
    const D1: f64 = 2.75;
    if progress < 1.0 / D1 {
        N1 * progress * progress
    } else if progress < 2.0 / D1 {
        let progress = progress - 1.5 / D1;
        N1 * progress * progress + 0.75
    } else if progress < 2.5 / D1 {
        let progress = progress - 2.25 / D1;
        N1 * progress * progress + 0.9375
    } else {
        let progress = progress - 2.625 / D1;
        N1 * progress * progress + 0.984375
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
    elapsed: Duration,
    behavior: Behavior,
}

#[derive(Clone, Copy, Debug)]
struct PhysicsAnimation {
    target: f64,
    velocity: f64,
    spec: Physics,
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
