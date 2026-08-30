/// One stop on a keyframe track.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// Position along the track, zero through one.
    pub at: f64,
    /// Value the property holds at this stop.
    pub value: Value,
    /// Curve used to reach this stop from the one before it.
    ///
    /// The first stop has nothing before it, so its curve is never applied.
    pub easing: Easing,
}

impl Keyframe {
    /// A stop reached with the given curve.
    pub fn new(at: f64, value: impl Into<Value>, easing: Easing) -> Self {
        Self {
            at,
            value: value.into(),
            easing,
        }
    }
}

/// Expands a keyframe track into the steps a group already knows how to run.
///
/// A track is not a second animation runtime: each pair of neighbouring stops
/// becomes one ordinary property animation with an explicit `from` and `to`, so
/// retargeting, damage classification and completion events apply to a
/// keyframed property exactly as they do to any other. The offsets are
/// fractions of one total duration, which is what makes a track editable — a
/// stop can move without every following segment having to be recomputed by
/// hand.
pub fn keyframe_steps(
    node: NodeHandle,
    property: &str,
    duration: Duration,
    frames: &[Keyframe],
) -> Result<Vec<AnimationStep>, SceneError> {
    if frames.len() < 2 {
        return Err(SceneError::Reactive(format!(
            "keyframe track `{property}` needs at least two stops"
        )));
    }
    if frames
        .iter()
        .any(|frame| !frame.at.is_finite() || !(0.0..=1.0).contains(&frame.at))
    {
        return Err(SceneError::Reactive(format!(
            "keyframe track `{property}` has a stop outside zero through one"
        )));
    }
    if frames.windows(2).any(|pair| pair[1].at < pair[0].at) {
        return Err(SceneError::Reactive(format!(
            "keyframe track `{property}` has stops out of order"
        )));
    }
    let mut steps = Vec::with_capacity(frames.len());
    // A track that does not begin at zero holds its first value until it does,
    // which is the difference between a delayed track and a longer first
    // segment.
    let lead = seconds(frames[0].at, duration);
    if !lead.is_zero() {
        steps.push(AnimationStep::Pause(lead));
    }
    for pair in frames.windows(2) {
        let (previous, frame) = (&pair[0], &pair[1]);
        let span = seconds(frame.at - previous.at, duration);
        if span.is_zero() {
            // Two stops at the same offset are a deliberate jump: the value
            // changes with no time to interpolate over.
            steps.push(AnimationStep::Property {
                node,
                property: property.to_owned(),
                from: Some(previous.value.clone()),
                to: frame.value.clone(),
                behavior: Behavior::timed(Duration::ZERO, frame.easing),
            });
            continue;
        }
        steps.push(AnimationStep::Property {
            node,
            property: property.to_owned(),
            from: Some(previous.value.clone()),
            to: frame.value.clone(),
            behavior: Behavior::timed(span, frame.easing),
        });
    }
    Ok(steps)
}

/// The share of `duration` a normalized span occupies.
fn seconds(fraction: f64, duration: Duration) -> Duration {
    Duration::from_secs_f64((duration.as_secs_f64() * fraction.max(0.0)).max(0.0))
}
