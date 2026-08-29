//! Reactive signal graph for mold.

use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error as StdError;
use std::fmt;

use slotmap::{Key, SlotMap, new_key_type};

new_key_type! {
    /// Generational handle to a signal slot.
    pub struct SignalId;
    /// Generational handle to a reactive effect.
    pub struct EffectId;
}

type EffectFn<T> = dyn for<'a> FnMut(&mut EffectContext<'a, T>) -> Result<(), String>;

struct Signal<T> {
    name: String,
    value: T,
    subscribers: HashSet<EffectId>,
    producer: Option<EffectId>,
}

struct Effect<T> {
    name: String,
    callback: Option<Box<EffectFn<T>>>,
    dependencies: HashSet<SignalId>,
    depth: usize,
    dirty: bool,
}

/// Non-fatal failures reported while draining a batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectError {
    /// Name assigned when the effect was registered.
    pub effect: String,
    /// Error returned by the effect callback.
    pub message: String,
}

/// Statistics and recoverable errors from one recompute pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlushReport {
    /// Number of effect evaluations performed.
    pub runs: usize,
    /// Effect failures whose staged writes were discarded.
    pub errors: Vec<EffectError>,
}

/// A graph operation that cannot be recovered within the current batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    /// A stale or foreign signal handle was used.
    InvalidSignal,
    /// A stale or foreign effect handle was used.
    InvalidEffect,
    /// Effects repeatedly invalidated one another within a single batch.
    Loop { chain: Vec<String> },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSignal => f.write_str("invalid reactive signal handle"),
            Self::InvalidEffect => f.write_str("invalid reactive effect handle"),
            Self::Loop { chain } => write!(f, "reactive loop: {}", chain.join(" -> ")),
        }
    }
}

impl StdError for GraphError {}

/// Dependency-capturing access available while an effect evaluates.
pub struct EffectContext<'a, T> {
    graph: &'a mut Graph<T>,
    effect: EffectId,
    dependencies: HashSet<SignalId>,
    writes: Vec<(SignalId, T)>,
}

impl<T: Clone + PartialEq + 'static> EffectContext<'_, T> {
    /// Reads a value and captures the dependency edge.
    pub fn get(&mut self, signal: SignalId) -> Result<T, GraphError> {
        if let Some((_, value)) = self.writes.iter().rev().find(|(id, _)| *id == signal) {
            self.dependencies.insert(signal);
            return Ok(value.clone());
        }
        let slot = self
            .graph
            .signals
            .get(signal)
            .ok_or(GraphError::InvalidSignal)?;
        self.dependencies.insert(signal);
        Ok(slot.value.clone())
    }

    /// Stages a signal write which is committed only if the effect succeeds.
    pub fn set(&mut self, signal: SignalId, value: T) -> Result<(), GraphError> {
        if !self.graph.signals.contains_key(signal) {
            return Err(GraphError::InvalidSignal);
        }
        self.writes.push((signal, value));
        Ok(())
    }
}

/// A generational signal arena with dynamic dependency capture.
pub struct Graph<T> {
    signals: SlotMap<SignalId, Signal<T>>,
    effects: SlotMap<EffectId, Effect<T>>,
    recompute_budget: usize,
}

impl<T: Clone + PartialEq + 'static> Default for Graph<T> {
    fn default() -> Self {
        Self::new(64)
    }
}

impl<T: Clone + PartialEq + 'static> Graph<T> {
    /// Creates a graph with a per-effect recompute budget for each batch.
    pub fn new(recompute_budget: usize) -> Self {
        Self {
            signals: SlotMap::with_key(),
            effects: SlotMap::with_key(),
            recompute_budget: recompute_budget.max(1),
        }
    }

    /// Allocates a named signal and returns its generational handle.
    pub fn signal(&mut self, name: impl Into<String>, value: T) -> SignalId {
        self.signals.insert(Signal {
            name: name.into(),
            value,
            subscribers: HashSet::new(),
            producer: None,
        })
    }

    /// Registers a named effect and queues its initial dependency-capturing run.
    pub fn effect<F>(&mut self, name: impl Into<String>, callback: F) -> EffectId
    where
        F: for<'a> FnMut(&mut EffectContext<'a, T>) -> Result<(), String> + 'static,
    {
        self.effects.insert(Effect {
            name: name.into(),
            callback: Some(Box::new(callback)),
            dependencies: HashSet::new(),
            depth: 0,
            dirty: true,
        })
    }

    /// Reads a signal without capturing a dependency.
    pub fn read(&self, signal: SignalId) -> Result<&T, GraphError> {
        self.signals
            .get(signal)
            .map(|slot| &slot.value)
            .ok_or(GraphError::InvalidSignal)
    }

    /// Returns the diagnostic name assigned to a signal.
    pub fn signal_name(&self, signal: SignalId) -> Result<&str, GraphError> {
        self.signals
            .get(signal)
            .map(|slot| slot.name.as_str())
            .ok_or(GraphError::InvalidSignal)
    }

    /// Writes a signal and queues only effects that currently depend on it.
    pub fn write(&mut self, signal: SignalId, value: T) -> Result<bool, GraphError> {
        self.write_from(signal, value, None)
    }

    /// Applies one event's writes and drains them in a single recompute pass.
    pub fn batch(
        &mut self,
        update: impl FnOnce(&mut Self) -> Result<(), GraphError>,
    ) -> Result<FlushReport, GraphError> {
        update(self)?;
        self.flush()
    }

    /// Removes an effect and every captured dependency edge.
    pub fn remove_effect(&mut self, effect: EffectId) -> Result<(), GraphError> {
        let removed = self
            .effects
            .remove(effect)
            .ok_or(GraphError::InvalidEffect)?;
        for signal in removed.dependencies {
            if let Some(slot) = self.signals.get_mut(signal) {
                slot.subscribers.remove(&effect);
            }
        }
        Ok(())
    }

    /// Drains all dirty effects in dependency depth order.
    pub fn flush(&mut self) -> Result<FlushReport, GraphError> {
        let mut report = FlushReport::default();
        let mut runs = HashMap::<EffectId, usize>::new();
        let mut trace = VecDeque::<String>::new();
        let mut originals = HashMap::<SignalId, (T, Option<EffectId>)>::new();

        while let Some(effect) = self.next_dirty() {
            let count = runs.entry(effect).or_default();
            *count += 1;
            if *count > self.recompute_budget {
                for (signal, (value, producer)) in originals {
                    if let Some(slot) = self.signals.get_mut(signal) {
                        slot.value = value;
                        slot.producer = producer;
                    }
                }
                for (_, pending) in &mut self.effects {
                    pending.dirty = false;
                }
                return Err(GraphError::Loop {
                    chain: trace.into_iter().collect(),
                });
            }

            let name = self.effects[effect].name.clone();
            trace.push_back(name.clone());
            let max_trace = self.recompute_budget.saturating_mul(2).max(4);
            if trace.len() > max_trace {
                trace.pop_front();
            }

            report.runs += 1;
            match self.run_effect(effect, &mut originals) {
                Ok(writes) => trace.extend(writes),
                Err(message) => report.errors.push(EffectError {
                    effect: name,
                    message,
                }),
            }
            while trace.len() > max_trace {
                trace.pop_front();
            }
        }

        Ok(report)
    }

    fn next_dirty(&self) -> Option<EffectId> {
        self.effects
            .iter()
            .filter(|(_, effect)| effect.dirty)
            .min_by_key(|(id, effect)| (effect.depth, id.data().as_ffi()))
            .map(|(id, _)| id)
    }

    fn run_effect(
        &mut self,
        effect: EffectId,
        originals: &mut HashMap<SignalId, (T, Option<EffectId>)>,
    ) -> Result<Vec<String>, String> {
        let old_dependencies = {
            let slot = self
                .effects
                .get_mut(effect)
                .ok_or_else(|| GraphError::InvalidEffect.to_string())?;
            slot.dirty = false;
            std::mem::take(&mut slot.dependencies)
        };
        for signal in &old_dependencies {
            if let Some(slot) = self.signals.get_mut(*signal) {
                slot.subscribers.remove(&effect);
            }
        }

        let mut callback = self.effects[effect]
            .callback
            .take()
            .ok_or_else(|| "effect is already running".to_owned())?;
        let mut context = EffectContext {
            graph: self,
            effect,
            dependencies: HashSet::new(),
            writes: Vec::new(),
        };
        let result = callback(&mut context);
        let dependencies = context.dependencies;
        let writes = context.writes;
        context.graph.effects[effect].callback = Some(callback);

        let dependencies = if result.is_ok() || !dependencies.is_empty() {
            dependencies
        } else {
            old_dependencies
        };
        let depth = dependencies
            .iter()
            .filter_map(|signal| context.graph.signals.get(*signal)?.producer)
            .filter_map(|producer| context.graph.effects.get(producer))
            .map(|producer| producer.depth.saturating_add(1))
            .max()
            .unwrap_or(0);
        context.graph.effects[effect].dependencies = dependencies.clone();
        context.graph.effects[effect].depth = depth;
        for signal in dependencies {
            if let Some(slot) = context.graph.signals.get_mut(signal) {
                slot.subscribers.insert(effect);
            }
        }

        result?;
        let mut written_names = Vec::with_capacity(writes.len());
        for (signal, value) in writes {
            if let Some(slot) = context.graph.signals.get(signal) {
                originals
                    .entry(signal)
                    .or_insert_with(|| (slot.value.clone(), slot.producer));
                written_names.push(slot.name.clone());
            }
            context
                .graph
                .write_from(signal, value, Some(context.effect))
                .map_err(|error| error.to_string())?;
        }
        Ok(written_names)
    }

    fn write_from(
        &mut self,
        signal: SignalId,
        value: T,
        producer: Option<EffectId>,
    ) -> Result<bool, GraphError> {
        let slot = self
            .signals
            .get_mut(signal)
            .ok_or(GraphError::InvalidSignal)?;
        if slot.value == value {
            if producer.is_some() {
                slot.producer = producer;
            }
            return Ok(false);
        }
        slot.value = value;
        slot.producer = producer;
        let subscribers: Vec<_> = slot.subscribers.iter().copied().collect();
        for effect in subscribers {
            if let Some(effect) = self.effects.get_mut(effect) {
                effect.dirty = true;
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn changing_one_signal_runs_exactly_one_effect() {
        let mut graph = Graph::default();
        let input = graph.signal("input", 1);
        let unrelated = graph.signal("unrelated", 7);
        let observed = Rc::new(Cell::new(0));
        let runs = Rc::new(Cell::new(0));
        graph.effect("observer", {
            let observed = Rc::clone(&observed);
            let runs = Rc::clone(&runs);
            move |ctx| {
                observed.set(ctx.get(input).map_err(|error| error.to_string())?);
                runs.set(runs.get() + 1);
                Ok(())
            }
        });
        graph.effect("unrelated observer", move |ctx| {
            let _ = ctx.get(unrelated).map_err(|error| error.to_string())?;
            Ok(())
        });
        graph.flush().unwrap();
        runs.set(0);

        graph.write(input, 2).unwrap();
        let report = graph.flush().unwrap();

        assert_eq!(report.runs, 1);
        assert_eq!(runs.get(), 1);
        assert_eq!(observed.get(), 2);
    }

    #[test]
    fn dependencies_are_recaptured_after_each_run() {
        let mut graph = Graph::default();
        let condition = graph.signal("condition", 1);
        let left = graph.signal("left", 1);
        let right = graph.signal("right", 2);
        graph.effect("conditional", move |ctx| {
            let selected = if ctx.get(condition).map_err(|error| error.to_string())? != 0 {
                left
            } else {
                right
            };
            let _ = ctx.get(selected).map_err(|error| error.to_string())?;
            Ok(())
        });
        graph.flush().unwrap();

        graph.write(condition, 0).unwrap();
        graph.flush().unwrap();
        graph.write(left, 9).unwrap();
        assert_eq!(graph.flush().unwrap().runs, 0);
        graph.write(right, 9).unwrap();
        assert_eq!(graph.flush().unwrap().runs, 1);
    }

    #[test]
    fn derived_effects_recompute_in_depth_order() {
        let mut graph = Graph::default();
        let source = graph.signal("source", 1);
        let middle = graph.signal("middle", 0);
        let output = graph.signal("output", 0);
        graph.effect("derive middle", move |ctx| {
            let value = ctx.get(source).map_err(|error| error.to_string())?;
            ctx.set(middle, value + 1)
                .map_err(|error| error.to_string())
        });
        graph.effect("derive output", move |ctx| {
            let value = ctx.get(middle).map_err(|error| error.to_string())?;
            ctx.set(output, value + 1)
                .map_err(|error| error.to_string())
        });
        graph.flush().unwrap();

        graph.write(source, 10).unwrap();
        let report = graph.flush().unwrap();

        assert_eq!(report.runs, 2);
        assert_eq!(*graph.read(output).unwrap(), 12);
    }

    #[test]
    fn a_batch_recomputes_an_effect_once_for_multiple_writes() {
        let mut graph = Graph::default();
        let left = graph.signal("left", 1);
        let right = graph.signal("right", 2);
        let runs = Rc::new(Cell::new(0));
        graph.effect("sum", {
            let runs = Rc::clone(&runs);
            move |ctx| {
                let _ = ctx.get(left).map_err(|error| error.to_string())?
                    + ctx.get(right).map_err(|error| error.to_string())?;
                runs.set(runs.get() + 1);
                Ok(())
            }
        });
        graph.flush().unwrap();
        runs.set(0);

        let report = graph
            .batch(|graph| {
                graph.write(left, 3)?;
                graph.write(right, 4)?;
                Ok(())
            })
            .unwrap();

        assert_eq!(report.runs, 1);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn a_failed_effect_discards_its_writes() {
        let mut graph = Graph::default();
        let value = graph.signal("value", 1);
        graph.effect("failure", move |ctx| {
            ctx.set(value, 2).map_err(|error| error.to_string())?;
            Err("broken binding".to_owned())
        });

        let report = graph.flush().unwrap();

        assert_eq!(*graph.read(value).unwrap(), 1);
        assert_eq!(report.errors[0].effect, "failure");
    }

    #[test]
    fn a_binding_loop_names_the_chain_and_keeps_last_good_values() {
        let mut graph = Graph::new(4);
        let left = graph.signal("left", 0);
        let right = graph.signal("right", 0);
        graph.effect("left binding", move |ctx| {
            let value = ctx.get(right).map_err(|error| error.to_string())?;
            ctx.set(left, value + 1).map_err(|error| error.to_string())
        });
        graph.effect("right binding", move |ctx| {
            let value = ctx.get(left).map_err(|error| error.to_string())?;
            ctx.set(right, value + 1).map_err(|error| error.to_string())
        });

        let error = graph.flush().unwrap_err();

        let message = error.to_string();
        assert!(message.contains("left binding"));
        assert!(message.contains("right binding"));
        assert!(message.contains("left"));
        assert!(message.contains("right"));
        assert_eq!(*graph.read(left).unwrap(), 0);
        assert_eq!(*graph.read(right).unwrap(), 0);
    }
}
