use animato::Update;
use mold_reactive::Graph;
use slotmap::SlotMap;
use std::collections::HashMap;
use std::time::Duration;

use crate::{animation::*, hashing::*, motion::*, schema::*, types::*};

impl Scene {
    /// Creates an empty scene arena.
    pub fn new() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            properties: Graph::default(),
            behaviors: FastMap::default(),
            animations: FastMap::default(),
            physics: FastMap::default(),
            physics_specs: FastMap::default(),
            paused_physics: FastSet::default(),
            events: Vec::new(),
            groups: HashMap::new(),
            group_events: Vec::new(),
            next_group: 0,
            layout_revision: 0,
            removed: Vec::new(),
        }
    }

    /// Allocates an element with every schema property initialized.
    pub fn create(&mut self, element: Element) -> NodeHandle {
        self.layout_revision = self.layout_revision.wrapping_add(1);
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
        self.layout_revision = self.layout_revision.wrapping_add(1);
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
                .retain(|node| node.id() != child_id);
        }
        self.nodes[child_id].parent = parent_id;
        if let Some(parent) = parent_id {
            self.nodes[parent].children.push(child);
        }
        Ok(())
    }

    /// Returns the current parent handle.
    pub fn parent(&self, node: NodeHandle) -> Result<Option<NodeHandle>, SceneError> {
        Ok(self.nodes[self.live(node)?].parent.map(NodeHandle))
    }

    /// Returns child handles in paint order.
    pub fn children(&self, node: NodeHandle) -> Result<&[NodeHandle], SceneError> {
        Ok(&self.nodes[self.live(node)?].children)
    }

    /// Removes a node and all descendants, invalidating their handles.
    pub fn remove(&mut self, node: NodeHandle) -> Result<(), SceneError> {
        let id = self.live(node)?;
        self.layout_revision = self.layout_revision.wrapping_add(1);
        if let Some(parent) = self.nodes[id].parent {
            self.nodes[parent]
                .children
                .retain(|handle| handle.id() != id);
        }
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            pending.extend(self.nodes[current].children.iter().map(|child| child.id()));
            self.behaviors.retain(|key, _| key.node != current);
            self.animations.retain(|key, _| key.node != current);
            self.physics.retain(|key, _| key.node != current);
            self.physics_specs.retain(|key, _| key.node != current);
            self.paused_physics.retain(|key| key.node != current);
            self.removed.push(NodeHandle(current));
            self.nodes.remove(current);
        }
        self.retain_live_groups();
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
            && behavior.intercepts()
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
            let interrupted =
                self.animations.remove(&key).is_some() | self.physics.remove(&key).is_some();
            self.paused_physics.remove(&key);
            if interrupted {
                self.push_event(key, AnimationEnd::Canceled);
            }
            self.properties.batch(|graph| {
                graph.write(slot.target, value.clone())?;
                graph.write(slot.current, value)?;
                Ok(())
            })?;
        }
        // Conservative: an assignment that only sets an animation's target has
        // not moved anything yet, but the ticks that follow will, and one extra
        // layout pass is a great deal cheaper than a frame drawn at stale
        // geometry.
        self.touch_layout(property_name);
        Ok(())
    }

    /// Advances every active behavior without invoking Lua.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, SceneError> {
        let mut frame = AnimationFrame {
            groups: self.tick_groups(delta)?,
            events: std::mem::take(&mut self.events),
            ..AnimationFrame::default()
        };
        let keys: Vec<_> = self.animations.keys().copied().collect();
        let mut finished = Vec::new();
        for key in keys {
            let animation = self
                .animations
                .get_mut(&key)
                .expect("animation key vanished");
            let paused = animation.is_paused();
            let delayed = animation.is_delayed();
            let complete = !animation.clock.update(delta.as_secs_f32());
            // A settling animation lands exactly on its target; an endless one
            // is stopped at whatever point in the cycle the clock reports.
            let value = if complete && animation.settles() {
                animation.settled().clone()
            } else {
                animation.value()
            };
            let Some(node) = self.nodes.get(key.node) else {
                finished.push(key);
                continue;
            };
            // A paused clock holds its value, and one still draining its delay
            // has not left the start value, so neither is worth a repaint.
            let idle = paused || (delayed && animation.is_delayed());
            if !idle {
                let slot = node.properties[key.property];
                if affects_layout(key.property) {
                    self.layout_revision = self.layout_revision.wrapping_add(1);
                }
                self.properties.write(slot.current, value)?;
                frame.changed += 1;
            }
            if complete {
                finished.push(key);
            }
        }
        for key in finished {
            if self.animations.remove(&key).is_some() {
                self.settle_target(key)?;
            }
            frame.events.push(AnimationEvent {
                node: NodeHandle(key.node),
                property: key.property,
                end: AnimationEnd::Completed,
            });
        }
        let physics_keys: Vec<_> = self.physics.keys().copied().collect();
        let mut physics_finished = Vec::new();
        for key in physics_keys {
            if self.paused_physics.contains(&key) {
                continue;
            }
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
            // Physics moves a property without any assignment, so this is the
            // one write that has to say so itself. Without it a paint reuses
            // the layout it already had and the scene animates behind a still
            // picture — every other path reaches here through `assign`.
            if affects_layout(key.property) {
                self.layout_revision = self.layout_revision.wrapping_add(1);
            }
            self.properties
                .write(slot.current, Value::Number(current))?;
            frame.changed += 1;
            if settled {
                physics_finished.push(key);
            }
        }
        for key in physics_finished {
            self.physics.remove(&key);
            self.paused_physics.remove(&key);
            self.settle_target(key)?;
            frame.events.push(AnimationEvent {
                node: NodeHandle(key.node),
                property: key.property,
                end: AnimationEnd::Completed,
            });
        }
        let report = self.properties.flush()?;
        if let Some(error) = report.errors.first() {
            return Err(SceneError::Reactive(format!(
                "{}: {}",
                error.effect, error.message
            )));
        }
        frame.active =
            !self.animations.is_empty() || !self.physics.is_empty() || !self.groups.is_empty();
        Ok(frame)
    }
}
