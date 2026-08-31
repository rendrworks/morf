use crate::{animation::*, motion::*, types::*};

impl Scene {
    /// Sets a numeric property coasting from its current value.
    ///
    /// The counterpart to a behavior. A behavior answers "when this is
    /// assigned, how does it travel there"; a fling has no there — it is given
    /// a speed and stops where friction leaves it, or where a bound catches it.
    /// That is the motion a flick wants: the surface keeps going after the
    /// finger lifts, and where it lands is a consequence of how hard it was
    /// thrown rather than of anything the configuration chose.
    ///
    /// Any tween, spring or earlier fling on the property is replaced. Writing
    /// the property while it coasts takes it back: whatever animates it next
    /// starts from where the fling had reached, at the speed it was going.
    pub fn fling(
        &mut self,
        node: NodeHandle,
        property: &str,
        velocity: f64,
        physics: Physics,
    ) -> Result<(), SceneError> {
        let Physics::Decay {
            friction,
            min_velocity,
            bounds,
            gravity,
            restitution,
        } = physics
        else {
            return Err(SceneError::Reactive(
                "a fling needs decay physics; a spring or a smoothing pursues a target".to_owned(),
            ));
        };
        validate_physics(physics).map_err(SceneError::Reactive)?;
        if !velocity.is_finite() {
            return Err(SceneError::Reactive(
                "fling velocity must be finite".to_owned(),
            ));
        }
        let slot = self.property(node, property)?;
        let Value::Number(current) = *self.properties.read(slot.current)? else {
            return Err(SceneError::Reactive(format!(
                "property `{property}` is not numeric"
            )));
        };
        let key = PropertyKey {
            node: self.live(node)?,
            property: self
                .node_ref(node)?
                .properties
                .get_key_value(property)
                .map(|(name, _)| *name)
                .ok_or(SceneError::StaleNode)?,
        };
        if self.animations.remove(&key).is_some() {
            self.push_event(key, AnimationEnd::Canceled);
        }
        self.paused_physics.remove(&key);
        self.physics.insert(
            key,
            PhysicsAnimation::Decay {
                position: current,
                velocity,
                friction,
                gravity,
                restitution,
                min_velocity,
                bounds,
            },
        );
        self.touch_layout(property);
        Ok(())
    }
}

impl Scene {
    /// Adds to the speed of a property that is already coasting.
    ///
    /// [`Scene::fling`] *sets* a velocity, which is right for a flick and wrong
    /// for a force: a push from something else — one blob pulling on another,
    /// a wind, a nudge towards the middle — has to add to whatever the motion
    /// was already doing, or every push erases the last one and the motion
    /// becomes a series of restarts.
    ///
    /// This is what lets a configuration supply forces without owning the
    /// motion. It computes what a push should be, at whatever rate suits it,
    /// and the engine keeps integrating every frame in between. Writing
    /// positions instead would make the configuration the clock.
    ///
    /// Returns whether anything was pushed: a property that is not coasting has
    /// no velocity to add to, and is left alone rather than started.
    pub fn impulse(
        &mut self,
        node: NodeHandle,
        property: &str,
        delta: f64,
    ) -> Result<bool, SceneError> {
        if !delta.is_finite() {
            return Err(SceneError::Reactive("an impulse must be finite".to_owned()));
        }
        let live = self.node_ref(node)?;
        let name = live
            .properties
            .get_key_value(property)
            .map(|(name, _)| *name)
            .ok_or_else(|| SceneError::UnknownProperty {
                element: live.element.name(),
                property: property.to_owned(),
            })?;
        let key = PropertyKey {
            node: self.live(node)?,
            property: name,
        };
        let Some(PhysicsAnimation::Decay { velocity, .. }) = self.physics.get_mut(&key) else {
            return Ok(false);
        };
        *velocity += delta;
        Ok(true)
    }
}
