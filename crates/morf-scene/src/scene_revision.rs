use crate::{animation::*, motion::*, types::*};

impl Scene {
    /// Records that a property layout reads has moved.
    ///
    /// Conservative on purpose: a spurious bump costs one extra layout pass, a
    /// missed one leaves the scene drawn at stale geometry.
    pub(crate) fn touch_layout(&mut self, property: &str) {
        if affects_layout(property) {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
    }

    /// How many times something layout reads has changed.
    ///
    /// A paint that finds this unmoved since its last one may reuse that
    /// layout instead of computing another.
    pub fn layout_revision(&self) -> u64 {
        self.layout_revision
    }
}

impl Scene {
    /// Points a property's target at wherever its motion actually stopped.
    ///
    /// Motion moves `current`; `target` is what the last write asked for, and
    /// the two part company whenever the motion did not land where it was
    /// aimed — an alternating repetition resting on its start value, or a
    /// fling, which is never aimed anywhere at all.
    ///
    /// Leaving them apart is not cosmetic. [`Scene::assign`] answers "is this
    /// property already what you are asking for" by reading `target` alone, so
    /// a stale target makes a later write to the pre-motion value a silent
    /// no-op: fling `y` away from zero, and `node.y = 0` afterwards does
    /// nothing at all. Every path that ends a motion has to come through here.
    pub(crate) fn settle_target(&mut self, key: PropertyKey) -> Result<(), SceneError> {
        let Some(node) = self.nodes.get(key.node) else {
            return Ok(());
        };
        let slot = node.properties[key.property];
        let settled = self.properties.read(slot.current)?.clone();
        if self.properties.read(slot.target)? != &settled {
            self.properties.write(slot.target, settled)?;
        }
        Ok(())
    }
}

impl Scene {
    /// Takes the nodes destroyed since this was last called.
    ///
    /// Everything holding state keyed on a node lives in another crate and
    /// cannot see one die. Whoever drives the frame drains this once and hands
    /// it to them; nobody else has both the scene and those caches in scope.
    pub fn take_removed_nodes(&mut self) -> Vec<NodeHandle> {
        std::mem::take(&mut self.removed)
    }

    /// Whether anything was destroyed since the list was last drained.
    pub fn has_removed_nodes(&self) -> bool {
        !self.removed.is_empty()
    }
}

impl Scene {
    /// Stops whatever is moving a property, and says so.
    ///
    /// Every path that replaces one kind of motion with another has to come
    /// through here. Two of the four used to do the removals inline and skip
    /// the event, so a configuration waiting on `on_finished` never heard from
    /// an animation that a `set_physics` had quietly thrown away.
    pub(crate) fn cancel_motion(&mut self, key: PropertyKey) {
        let stopped = self.animations.remove(&key).is_some() | self.physics.remove(&key).is_some();
        self.paused_physics.remove(&key);
        if stopped {
            self.push_event(key, AnimationEnd::Canceled);
        }
    }
}
