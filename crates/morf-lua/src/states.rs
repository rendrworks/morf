// Declarative states: what a configuration declares, and what the runtime keeps
// while moving between them.

use std::collections::{HashMap, HashSet};

use luna::StashedClosure;
use morf_reactive::SignalId;
use morf_scene::{Behavior, NodeHandle, Value as SceneValue};

use crate::surface_types::IpcValue;

#[derive(Clone)]
pub(crate) struct StateDefinition {
    pub(crate) properties: Vec<(String, StateValue)>,
    pub(crate) anchors: Option<std::collections::BTreeMap<String, SceneValue>>,
    pub(crate) parent: Option<NodeHandle>,
}

#[derive(Clone)]
pub(crate) enum StateValue {
    Value(SceneValue),
    Binding(StashedClosure),
}

#[derive(Clone)]
pub(crate) struct StateTransition {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) reversible: bool,
    pub(crate) behavior: Behavior,
}

#[derive(Default)]
pub(crate) struct StateSet {
    pub(crate) definitions: HashMap<String, StateDefinition>,
    pub(crate) transitions: Vec<StateTransition>,
    pub(crate) current: Option<String>,
}

#[derive(Default)]
pub(crate) struct Capture {
    pub(crate) reads: HashSet<SignalId>,
    pub(crate) property_reads: HashSet<(NodeHandle, String, bool)>,
    pub(crate) writes: Vec<(SignalId, IpcValue)>,
}
