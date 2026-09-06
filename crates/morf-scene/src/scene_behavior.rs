use crate::{animation::*, motion::*, schema::*, types::*};

// How a property travels when it is assigned.
//
// Assignment says what a property becomes; these three say how it gets
// there. They sit apart from the tree and the writes in `scene.rs` because
// they are the only places that decide the *manner* of a change rather than
// its result.

impl Scene {
    /// Installs or removes a write-intercepting behavior on a property.
    pub fn set_behavior(
        &mut self,
        node: NodeHandle,
        property: &str,
        behavior: Option<Behavior>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let (name, _) = self.nodes[id]
            .properties
            .get_key_value(property)
            .ok_or_else(|| SceneError::UnknownProperty {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
            })?;
        let key = PropertyKey {
            node: id,
            property: name,
        };
        if let Some(behavior) = behavior {
            self.behaviors.insert(key, behavior);
            self.physics_specs.remove(&key);
        } else {
            self.behaviors.remove(&key);
        }
        // Either way, whatever was moving stops. Two of these four arms used to
        // tear the motion down silently — no `Canceled`, and in one case the
        // paused set left holding a key whose motion was gone — so whether a
        // configuration heard that its animation had ended depended on which
        // way it replaced it.
        self.cancel_motion(key);
        Ok(())
    }

    /// Starts a finite animation from an explicit current value.
    pub fn animate_from(
        &mut self,
        node: NodeHandle,
        property: &str,
        from: impl Into<Value>,
        to: impl Into<Value>,
        behavior: Behavior,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let element = self.nodes[id].element;
        let (name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .map(|(name, slot)| (*name, *slot))
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })?;
        let from = coerce(element, property, slot.kind, from.into())?;
        let to = coerce(element, property, slot.kind, to.into())?;
        if !interpolatable(&from, &to) {
            return Err(SceneError::InvalidPropertyType {
                element: element.name(),
                property: property.to_owned(),
                expected: "interpolatable values",
            });
        }
        let key = PropertyKey {
            node: id,
            property: name,
        };
        let from = animation_start(name, from, &to, behavior.rotation_direction);
        self.paused_physics.remove(&key);
        if self.physics.remove(&key).is_some() {
            self.push_event(key, AnimationEnd::Canceled);
        }
        self.properties.batch(|graph| {
            graph.write(slot.current, from.clone())?;
            graph.write(slot.target, to.clone())?;
            Ok(())
        })?;
        if !behavior.intercepts() {
            self.properties.write(slot.current, to)?;
            self.animations.remove(&key);
        } else {
            self.animations.insert(
                key,
                Animation::new(from, to, Velocity::Number(0.0), false, behavior),
            );
        }
        // This writes `current` directly — the jump to `from` above, and with a
        // zero duration the landing on `to` as well — so it has to announce the
        // move itself. Without it a paint reuses the layout it already has and
        // the geometry changes behind a still picture; at zero duration nothing
        // later bumps the revision, so the stale picture is permanent.
        self.touch_layout(name);
        Ok(())
    }

    /// Installs or removes physics-driven motion on a numeric property.
    pub fn set_physics(
        &mut self,
        node: NodeHandle,
        property: &str,
        physics: Option<Physics>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let (name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .ok_or_else(|| SceneError::UnknownProperty {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
            })?;
        if !matches!(slot.kind, PropertyType::Number | PropertyType::Color) {
            return Err(SceneError::InvalidPropertyType {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
                expected: "numeric or colour property",
            });
        }
        let key = PropertyKey {
            node: id,
            property: name,
        };
        if let Some(physics) = physics {
            // A behavior is "when this is assigned, animate to it". Decay has
            // nothing to animate to, so accepting it here would install motion
            // that no assignment could ever start.
            if matches!(physics, Physics::Decay { .. }) {
                return Err(SceneError::Reactive(
                    "decay is started with a fling, not installed as a behavior".to_owned(),
                ));
            }
            validate_physics(physics).map_err(SceneError::Reactive)?;
            self.physics_specs.insert(key, physics);
            self.behaviors.remove(&key);
        } else {
            self.physics_specs.remove(&key);
        }
        self.cancel_motion(key);
        Ok(())
    }
}
