use crate::{animation::*, types::*};

impl Scene {
    /// Reads the value currently used by layout or paint.
    pub fn current(&self, node: NodeHandle, property: &str) -> Result<&Value, SceneError> {
        let slot = self.property(node, property)?;
        Ok(self.properties.read(slot.current)?)
    }

    /// Reads the settled value most recently produced by a binding or assignment.
    pub fn target(&self, node: NodeHandle, property: &str) -> Result<&Value, SceneError> {
        let slot = self.property(node, property)?;
        Ok(self.properties.read(slot.target)?)
    }

    /// Reads a numeric current property.
    pub fn number(&self, node: NodeHandle, property: &str) -> Result<f64, SceneError> {
        match self.current(node, property)? {
            Value::Number(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not numeric"
            ))),
        }
    }

    /// Reads a string current property.
    pub fn string_value(&self, node: NodeHandle, property: &str) -> Result<&str, SceneError> {
        match self.current(node, property)? {
            Value::String(value) => Ok(value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not a string"
            ))),
        }
    }

    /// Reads a boolean current property.
    pub fn bool_value(&self, node: NodeHandle, property: &str) -> Result<bool, SceneError> {
        match self.current(node, property)? {
            Value::Bool(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not boolean"
            ))),
        }
    }

    /// Reads a color current property.
    pub fn color_value(&self, node: NodeHandle, property: &str) -> Result<Color, SceneError> {
        match self.current(node, property)? {
            Value::Color(value) => Ok(*value),
            _ => Err(SceneError::Reactive(format!(
                "property `{property}` is not a color"
            ))),
        }
    }

    pub(crate) fn live(&self, node: NodeHandle) -> Result<NodeId, SceneError> {
        self.nodes
            .contains_key(node.0)
            .then_some(node.0)
            .ok_or(SceneError::StaleNode)
    }

    pub(crate) fn property(
        &self,
        node: NodeHandle,
        property: &str,
    ) -> Result<PropertySlot, SceneError> {
        // One arena lookup, not three: `live` used to probe for the node and
        // then index it twice more. Every property read of every node in every
        // frame goes through here, so the difference is worth the directness.
        let node = self.node_ref(node)?;
        node.properties
            .get(property)
            .copied()
            .ok_or_else(|| SceneError::UnknownProperty {
                element: node.element.name(),
                property: property.to_owned(),
            })
    }

    /// Borrows a live node, or reports that its handle has gone stale.
    pub(crate) fn node_ref(&self, node: NodeHandle) -> Result<&Node, SceneError> {
        self.nodes.get(node.id()).ok_or(SceneError::StaleNode)
    }
}
