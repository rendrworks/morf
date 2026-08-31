use crate::{animation::*, motion::*, types::*};

impl Scene {
    /// Returns the write interceptor installed on a property, if any.
    pub fn behavior(
        &self,
        node: NodeHandle,
        property: &str,
    ) -> Result<Option<Behavior>, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        Ok(self.behaviors.get(&key).copied())
    }

    /// Turns an installed behavior on or off without discarding its settings.
    ///
    /// Disabling one leaves any animation already in flight to finish; only the
    /// next write is applied directly. Returns whether a behavior was found.
    pub fn set_behavior_enabled(
        &mut self,
        node: NodeHandle,
        property: &str,
        enabled: bool,
    ) -> Result<bool, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        let Some(behavior) = self.behaviors.get_mut(&key) else {
            return Ok(false);
        };
        behavior.enabled = enabled;
        Ok(true)
    }

    /// Reports the eased position of an in-flight interval animation.
    ///
    /// Physics motion has no timeline and always reports `None`.
    pub fn animation_progress(
        &self,
        node: NodeHandle,
        property: &str,
    ) -> Result<Option<f64>, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        Ok(self.animations.get(&key).map(Animation::progress))
    }

    /// Reports whether an animation on the property is halted mid-flight.
    pub fn is_animation_paused(
        &self,
        node: NodeHandle,
        property: &str,
    ) -> Result<bool, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        Ok(self.animations.get(&key).is_some_and(Animation::is_paused)
            || self.paused_physics.contains(&key))
    }

    /// Halts or resumes an animation in place, keeping its target and velocity.
    ///
    /// Returns whether an animation was found to pause.
    pub fn set_animation_paused(
        &mut self,
        node: NodeHandle,
        property: &str,
        paused: bool,
    ) -> Result<bool, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        let mut found = false;
        if let Some(animation) = self.animations.get_mut(&key) {
            if paused {
                animation.clock.pause();
            } else {
                animation.clock.resume();
            }
            found = true;
        }
        if self.physics.contains_key(&key) {
            if paused {
                self.paused_physics.insert(key);
            } else {
                self.paused_physics.remove(&key);
            }
            found = true;
        }
        Ok(found)
    }

    /// Stops an animation where it stands and pins the target to that value.
    ///
    /// This is the halt that does not snap: the property keeps whatever value
    /// the last tick produced. Returns whether an animation was stopped.
    pub fn stop_animation(&mut self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
        let (key, slot) = self.property_key(node, property)?;
        if !self.animations.contains_key(&key) && !self.physics.contains_key(&key) {
            return Ok(false);
        }
        self.animations.remove(&key);
        self.physics.remove(&key);
        self.paused_physics.remove(&key);
        let current = self.properties.read(slot.current)?.clone();
        self.properties.write(slot.target, current)?;
        self.push_event(key, AnimationEnd::Stopped);
        Ok(true)
    }

    /// Ends an animation immediately at its target value.
    ///
    /// Returns whether an animation was finished early.
    pub fn finish_animation(
        &mut self,
        node: NodeHandle,
        property: &str,
    ) -> Result<bool, SceneError> {
        let (key, slot) = self.property_key(node, property)?;
        let target = match (self.animations.remove(&key), self.physics.remove(&key)) {
            (Some(animation), _) => animation.to,
            (None, Some(motion)) => Value::Number(motion.target()),
            (None, None) => return Ok(false),
        };
        self.paused_physics.remove(&key);
        self.touch_layout(key.property);
        self.properties.batch(|graph| {
            graph.write(slot.current, target.clone())?;
            graph.write(slot.target, target)?;
            Ok(())
        })?;
        self.push_event(key, AnimationEnd::Completed);
        Ok(true)
    }

    /// Replays an in-flight animation from its start, including any delay.
    ///
    /// Physics motion is re-seeded from the current value with no velocity, so
    /// a spring rebuilds its approach instead of continuing the old one.
    pub fn restart_animation(
        &mut self,
        node: NodeHandle,
        property: &str,
    ) -> Result<bool, SceneError> {
        let (key, slot) = self.property_key(node, property)?;
        if let Some(animation) = self.animations.get_mut(&key) {
            animation.clock.reset();
            let value = animation.value();
            self.properties.write(slot.current, value)?;
            self.touch_layout(key.property);
            return Ok(true);
        }
        let Some(motion) = self.physics.get(&key) else {
            return Ok(false);
        };
        let target = motion.target();
        let spec = self
            .physics_specs
            .get(&key)
            .copied()
            .expect("physics motion without an installed spec");
        let Value::Number(current) = *self.properties.read(slot.current)? else {
            return Ok(false);
        };
        self.physics
            .insert(key, physics_animation(current, target, 0.0, spec));
        Ok(true)
    }

    /// Sends an interval animation back to the value it set out from.
    ///
    /// The property does not jump: the reversal is a retarget from where it
    /// stands, so the installed behavior carries its current velocity into the
    /// return leg the same way an interrupting write would. Physics motion has
    /// no start value to return to and reports `false`.
    pub fn reverse_animation(
        &mut self,
        node: NodeHandle,
        property: &str,
    ) -> Result<bool, SceneError> {
        let (key, _) = self.property_key(node, property)?;
        let Some(origin) = self
            .animations
            .get(&key)
            .map(|animation| animation.from.clone())
        else {
            return Ok(false);
        };
        self.assign(node, property, origin)?;
        Ok(true)
    }

    /// Jumps an interval animation to a normalized position in its interval.
    ///
    /// Scrubbing keeps the animation active, so the next tick resumes from the
    /// requested position. Physics motion reports `false`.
    pub fn seek_animation(
        &mut self,
        node: NodeHandle,
        property: &str,
        progress: f64,
    ) -> Result<bool, SceneError> {
        let (key, slot) = self.property_key(node, property)?;
        let Some(animation) = self.animations.get_mut(&key) else {
            return Ok(false);
        };
        animation.clock.seek(progress.clamp(0.0, 1.0) as f32);
        let value = animation.value();
        self.properties.write(slot.current, value)?;
        self.touch_layout(key.property);
        Ok(true)
    }

    /// Resolves a property to its arena key and signal slot in one step.
    pub(crate) fn property_key(
        &self,
        node: NodeHandle,
        property: &str,
    ) -> Result<(PropertyKey, PropertySlot), SceneError> {
        let id = self.live(node)?;
        let (name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .map(|(name, slot)| (*name, *slot))
            .ok_or_else(|| SceneError::UnknownProperty {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
            })?;
        Ok((
            PropertyKey {
                node: id,
                property: name,
            },
            slot,
        ))
    }

    /// Queues an end-of-animation event for the next tick to report.
    pub(crate) fn push_event(&mut self, key: PropertyKey, end: AnimationEnd) {
        self.events.push(AnimationEvent {
            node: NodeHandle(key.node),
            property: key.property,
            end,
        });
    }
}
