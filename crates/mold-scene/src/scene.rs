impl Scene {
    /// Creates an empty scene arena.
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            properties: Graph::default(),
            behaviors: HashMap::new(),
            animations: HashMap::new(),
            physics: HashMap::new(),
            physics_specs: HashMap::new(),
        }
    }

    /// Allocates an element with every schema property initialized.
    pub fn create(&mut self, element: Element) -> NodeHandle {
        let node = self.nodes.insert_with_key(|id| {
            let properties = schema(element)
                .into_iter()
                .map(|spec| {
                    let prefix = format!("{}[{:?}].{}", element.name(), id, spec.name);
                    let current = self
                        .properties
                        .signal(format!("{prefix}.current"), spec.default.clone());
                    let target = self
                        .properties
                        .signal(format!("{prefix}.target"), spec.default);
                    (
                        spec.name,
                        PropertySlot {
                            current,
                            target,
                            kind: spec.kind,
                        },
                    )
                })
                .collect();
            Node {
                element,
                parent: None,
                children: Vec::new(),
                properties,
            }
        });
        NodeHandle(node)
    }

    /// Returns whether a handle still refers to a live node generation.
    pub fn contains(&self, node: NodeHandle) -> bool {
        self.nodes.contains_key(node.0)
    }

    /// Returns all live nodes without a parent in arena order.
    pub fn roots(&self) -> Vec<NodeHandle> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.parent.is_none())
            .map(|(id, _)| NodeHandle(id))
            .collect()
    }

    /// Returns the element kind for a live node.
    pub fn element(&self, node: NodeHandle) -> Result<Element, SceneError> {
        Ok(self.nodes[self.live(node)?].element)
    }

    /// Checks whether a live element schema declares a property.
    pub fn has_property(&self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
        Ok(self.nodes[self.live(node)?]
            .properties
            .contains_key(property))
    }

    /// Reports whether a property currently advances on animation ticks.
    pub fn is_animating(&self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
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
        Ok(self.animations.contains_key(&key) || self.physics.contains_key(&key))
    }

    /// Appends a node to a new parent while preserving the child's identity.
    pub fn reparent(
        &mut self,
        child: NodeHandle,
        parent: Option<NodeHandle>,
    ) -> Result<(), SceneError> {
        let child_id = self.live(child)?;
        let parent_id = parent.map(|handle| self.live(handle)).transpose()?;
        if parent_id == Some(child_id) {
            return Err(SceneError::ParentCycle);
        }
        let mut ancestor = parent_id;
        while let Some(node) = ancestor {
            if node == child_id {
                return Err(SceneError::ParentCycle);
            }
            ancestor = self.nodes[node].parent;
        }

        if let Some(old_parent) = self.nodes[child_id].parent {
            self.nodes[old_parent]
                .children
                .retain(|node| *node != child_id);
        }
        self.nodes[child_id].parent = parent_id;
        if let Some(parent) = parent_id {
            self.nodes[parent].children.push(child_id);
        }
        Ok(())
    }

    /// Returns the current parent handle.
    pub fn parent(&self, node: NodeHandle) -> Result<Option<NodeHandle>, SceneError> {
        Ok(self.nodes[self.live(node)?].parent.map(NodeHandle))
    }

    /// Returns child handles in paint order.
    pub fn children(&self, node: NodeHandle) -> Result<Vec<NodeHandle>, SceneError> {
        Ok(self.nodes[self.live(node)?]
            .children
            .iter()
            .copied()
            .map(NodeHandle)
            .collect())
    }

    /// Removes a node and all descendants, invalidating their handles.
    pub fn remove(&mut self, node: NodeHandle) -> Result<(), SceneError> {
        let id = self.live(node)?;
        if let Some(parent) = self.nodes[id].parent {
            self.nodes[parent].children.retain(|child| *child != id);
        }
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            pending.extend(self.nodes[current].children.iter().copied());
            self.behaviors.retain(|key, _| key.node != current);
            self.animations.retain(|key, _| key.node != current);
            self.physics.retain(|key, _| key.node != current);
            self.physics_specs.retain(|key, _| key.node != current);
            self.nodes.remove(current);
        }
        Ok(())
    }

    /// Assigns and coerces a plain value to both target and rendered property levels.
    pub fn assign(
        &mut self,
        node: NodeHandle,
        property: &str,
        value: impl Into<Value>,
    ) -> Result<(), SceneError> {
        let id = self.live(node)?;
        let element = self.nodes[id].element;
        let (property_name, slot) = self.nodes[id]
            .properties
            .get_key_value(property)
            .map(|(name, slot)| (*name, *slot))
            .ok_or_else(|| SceneError::UnknownProperty {
                element: element.name(),
                property: property.to_owned(),
            })?;
        let value = coerce(element, property, slot.kind, value.into())?;
        let key = PropertyKey {
            node: id,
            property: property_name,
        };
        if self.properties.read(slot.target)? == &value {
            return Ok(());
        }
        if let Some(spec) = self.physics_specs.get(&key).copied()
            && let Value::Number(target) = value
            && matches!(self.properties.read(slot.current)?, Value::Number(_))
        {
            let velocity = self
                .physics
                .get(&key)
                .map_or(0.0, PhysicsAnimation::velocity);
            let current = self.properties.read(slot.current)?.clone();
            let Value::Number(current) = current else {
                unreachable!("numeric physics target had a non-numeric current value")
            };
            self.animations.remove(&key);
            self.properties.write(slot.target, Value::Number(target))?;
            self.physics
                .insert(key, physics_animation(current, target, velocity, spec));
        } else if let Some(behavior) = self.behaviors.get(&key).copied()
            && behavior.duration > Duration::ZERO
            && interpolatable(self.properties.read(slot.current)?, &value)
        {
            let from = animation_start(
                property_name,
                self.properties.read(slot.current)?.clone(),
                &value,
                behavior.rotation_direction,
            );
            let initial_velocity = self
                .animations
                .get(&key)
                .map(Animation::velocity)
                .unwrap_or_else(|| zero_velocity(&from));
            self.properties.write(slot.target, value.clone())?;
            self.animations.insert(
                key,
                Animation::new(
                    from,
                    value,
                    initial_velocity,
                    initial_velocity.is_moving(),
                    behavior,
                ),
            );
        } else {
            self.animations.remove(&key);
            self.physics.remove(&key);
            self.properties.batch(|graph| {
                graph.write(slot.target, value.clone())?;
                graph.write(slot.current, value)?;
                Ok(())
            })?;
        }
        Ok(())
    }

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
            self.physics.remove(&key);
        } else {
            self.behaviors.remove(&key);
            self.animations.remove(&key);
        }
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
        self.physics.remove(&key);
        self.properties.batch(|graph| {
            graph.write(slot.current, from.clone())?;
            graph.write(slot.target, to.clone())?;
            Ok(())
        })?;
        if behavior.duration == Duration::ZERO {
            self.properties.write(slot.current, to)?;
            self.animations.remove(&key);
        } else {
            self.animations.insert(
                key,
                Animation::new(from, to, Velocity::Number(0.0), false, behavior),
            );
        }
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
        if !matches!(slot.kind, PropertyType::Number) {
            return Err(SceneError::InvalidPropertyType {
                element: self.nodes[id].element.name(),
                property: property.to_owned(),
                expected: "numeric property",
            });
        }
        let key = PropertyKey {
            node: id,
            property: name,
        };
        if let Some(physics) = physics {
            validate_physics(physics).map_err(SceneError::Reactive)?;
            self.physics_specs.insert(key, physics);
            self.behaviors.remove(&key);
            self.animations.remove(&key);
        } else {
            self.physics_specs.remove(&key);
            self.physics.remove(&key);
        }
        Ok(())
    }

    /// Advances every active behavior without invoking Lua.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, SceneError> {
        let mut frame = AnimationFrame::default();
        let keys: Vec<_> = self.animations.keys().copied().collect();
        let mut finished = Vec::new();
        for key in keys {
            let animation = self
                .animations
                .get_mut(&key)
                .expect("animation key vanished");
            let complete = !animation.clock.update(delta.as_secs_f32());
            let value = if complete {
                animation.to.clone()
            } else {
                animation.value()
            };
            let Some(node) = self.nodes.get(key.node) else {
                finished.push(key);
                continue;
            };
            let slot = node.properties[key.property];
            self.properties.write(slot.current, value)?;
            frame.changes.push(AnimatedChange {
                node: NodeHandle(key.node),
                property: key.property,
                class: property_class(key.property),
            });
            if complete {
                finished.push(key);
            }
        }
        for key in finished {
            self.animations.remove(&key);
        }
        let physics_keys: Vec<_> = self.physics.keys().copied().collect();
        let mut physics_finished = Vec::new();
        for key in physics_keys {
            let Some(node) = self.nodes.get(key.node) else {
                physics_finished.push(key);
                continue;
            };
            let slot = node.properties[key.property];
            let Value::Number(mut current) = *self.properties.read(slot.current)? else {
                physics_finished.push(key);
                continue;
            };
            let motion = self.physics.get_mut(&key).expect("physics key vanished");
            let settled = advance_physics(motion, &mut current, delta);
            self.properties
                .write(slot.current, Value::Number(current))?;
            frame.changes.push(AnimatedChange {
                node: NodeHandle(key.node),
                property: key.property,
                class: property_class(key.property),
            });
            if settled {
                physics_finished.push(key);
            }
        }
        for key in physics_finished {
            self.physics.remove(&key);
        }
        let report = self.properties.flush()?;
        if let Some(error) = report.errors.first() {
            return Err(SceneError::Reactive(format!(
                "{}: {}",
                error.effect, error.message
            )));
        }
        frame.active = !self.animations.is_empty() || !self.physics.is_empty();
        Ok(frame)
    }

}
