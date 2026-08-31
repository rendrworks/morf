use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// The evaluator side of an externally-registered effect, for tests.
///
/// `external_effect` is the only way anything in this workspace registers an
/// effect — the graph hands out a token and the host evaluates it — so the
/// tests drive that path rather than a second, graph-owned closure API that no
/// production code ever used.
/// One registered evaluator, boxed so the table can hold several shapes.
type TestEffect<'a, T> = Box<dyn FnMut(&mut EffectContext<'_, T>) -> Result<(), String> + 'a>;

#[derive(Default)]
struct Effects<'a, T> {
    callbacks: RefCell<Vec<TestEffect<'a, T>>>,
}

impl<'a, T: Clone + PartialEq + 'static> Effects<'a, T> {
    fn register(
        &self,
        graph: &mut Graph<T>,
        name: &str,
        callback: impl FnMut(&mut EffectContext<'_, T>) -> Result<(), String> + 'a,
    ) -> EffectId {
        let mut callbacks = self.callbacks.borrow_mut();
        let token = callbacks.len() as u64;
        callbacks.push(Box::new(callback));
        graph.external_effect(name, token)
    }

    fn flush(&self, graph: &mut Graph<T>) -> Result<FlushReport, GraphError> {
        graph.flush_external(|token, context| self.evaluate(token, context))
    }

    fn batch(
        &self,
        graph: &mut Graph<T>,
        update: impl FnOnce(&mut Graph<T>) -> Result<(), GraphError>,
    ) -> Result<FlushReport, GraphError> {
        graph.batch_external(|token, context| self.evaluate(token, context), update)
    }

    fn evaluate(&self, token: u64, context: &mut EffectContext<'_, T>) -> Result<(), String> {
        let mut callbacks = self.callbacks.borrow_mut();
        callbacks[token as usize](context)
    }
}

#[test]
fn changing_one_signal_runs_exactly_one_effect() {
    let effects = Effects::default();
    let mut graph = Graph::default();
    let input = graph.signal("input", 1);
    let unrelated = graph.signal("unrelated", 7);
    let observed = Rc::new(Cell::new(0));
    let runs = Rc::new(Cell::new(0));
    effects.register(&mut graph, "observer", {
        let observed = Rc::clone(&observed);
        let runs = Rc::clone(&runs);
        move |ctx| {
            observed.set(ctx.get(input).map_err(|error| error.to_string())?);
            runs.set(runs.get() + 1);
            Ok(())
        }
    });
    effects.register(&mut graph, "unrelated observer", move |ctx| {
        let _ = ctx.get(unrelated).map_err(|error| error.to_string())?;
        Ok(())
    });
    effects.flush(&mut graph).unwrap();
    runs.set(0);

    graph.write(input, 2).unwrap();
    let report = effects.flush(&mut graph).unwrap();

    assert_eq!(report.runs, 1);
    assert_eq!(runs.get(), 1);
    assert_eq!(observed.get(), 2);
}

#[test]
fn dependencies_are_recaptured_after_each_run() {
    let effects = Effects::default();
    let mut graph = Graph::default();
    let condition = graph.signal("condition", 1);
    let left = graph.signal("left", 1);
    let right = graph.signal("right", 2);
    effects.register(&mut graph, "conditional", move |ctx| {
        let selected = if ctx.get(condition).map_err(|error| error.to_string())? != 0 {
            left
        } else {
            right
        };
        let _ = ctx.get(selected).map_err(|error| error.to_string())?;
        Ok(())
    });
    effects.flush(&mut graph).unwrap();

    graph.write(condition, 0).unwrap();
    effects.flush(&mut graph).unwrap();
    graph.write(left, 9).unwrap();
    assert_eq!(effects.flush(&mut graph).unwrap().runs, 0);
    graph.write(right, 9).unwrap();
    assert_eq!(effects.flush(&mut graph).unwrap().runs, 1);
}

#[test]
fn derived_effects_recompute_in_depth_order() {
    let effects = Effects::default();
    let mut graph = Graph::default();
    let source = graph.signal("source", 1);
    let middle = graph.signal("middle", 0);
    let output = graph.signal("output", 0);
    effects.register(&mut graph, "derive middle", move |ctx| {
        let value = ctx.get(source).map_err(|error| error.to_string())?;
        ctx.set(middle, value + 1)
            .map_err(|error| error.to_string())
    });
    effects.register(&mut graph, "derive output", move |ctx| {
        let value = ctx.get(middle).map_err(|error| error.to_string())?;
        ctx.set(output, value + 1)
            .map_err(|error| error.to_string())
    });
    effects.flush(&mut graph).unwrap();

    graph.write(source, 10).unwrap();
    let report = effects.flush(&mut graph).unwrap();

    assert_eq!(report.runs, 2);
    assert_eq!(*graph.read(output).unwrap(), 12);
}

#[test]
fn a_batch_recomputes_an_effect_once_for_multiple_writes() {
    let effects = Effects::default();
    let mut graph = Graph::default();
    let left = graph.signal("left", 1);
    let right = graph.signal("right", 2);
    let runs = Rc::new(Cell::new(0));
    effects.register(&mut graph, "sum", {
        let runs = Rc::clone(&runs);
        move |ctx| {
            let _ = ctx.get(left).map_err(|error| error.to_string())?
                + ctx.get(right).map_err(|error| error.to_string())?;
            runs.set(runs.get() + 1);
            Ok(())
        }
    });
    effects.flush(&mut graph).unwrap();
    runs.set(0);

    let report = effects
        .batch(&mut graph, |graph| {
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
    let effects = Effects::default();
    let mut graph = Graph::default();
    let value = graph.signal("value", 1);
    effects.register(&mut graph, "failure", move |ctx| {
        ctx.set(value, 2).map_err(|error| error.to_string())?;
        Err("broken binding".to_owned())
    });

    let report = effects.flush(&mut graph).unwrap();

    assert_eq!(*graph.read(value).unwrap(), 1);
    assert_eq!(report.errors[0].effect, "failure");
}

#[test]
fn a_binding_loop_names_the_chain_and_keeps_last_good_values() {
    let effects = Effects::default();
    let mut graph = Graph::new(4);
    let left = graph.signal("left", 0);
    let right = graph.signal("right", 0);
    effects.register(&mut graph, "left binding", move |ctx| {
        let value = ctx.get(right).map_err(|error| error.to_string())?;
        ctx.set(left, value + 1).map_err(|error| error.to_string())
    });
    effects.register(&mut graph, "right binding", move |ctx| {
        let value = ctx.get(left).map_err(|error| error.to_string())?;
        ctx.set(right, value + 1).map_err(|error| error.to_string())
    });

    let error = effects.flush(&mut graph).unwrap_err();

    let message = error.to_string();
    assert!(message.contains("left binding"));
    assert!(message.contains("right binding"));
    assert!(message.contains("left"));
    assert!(message.contains("right"));
    assert_eq!(*graph.read(left).unwrap(), 0);
    assert_eq!(*graph.read(right).unwrap(), 0);
}

#[test]
fn externally_evaluated_effects_participate_in_loop_detection() {
    let mut graph = Graph::new(4);
    let left = graph.signal("left", 0);
    let right = graph.signal("right", 0);
    graph.external_effect("left binding", 1);
    graph.external_effect("right binding", 2);

    let error = graph
        .flush_external(|token, ctx| {
            if token == 1 {
                let value = ctx.get(right).map_err(|error| error.to_string())?;
                ctx.set(left, value + 1).map_err(|error| error.to_string())
            } else {
                let value = ctx.get(left).map_err(|error| error.to_string())?;
                ctx.set(right, value + 1).map_err(|error| error.to_string())
            }
        })
        .unwrap_err();

    assert!(error.to_string().contains("left binding"));
    assert!(error.to_string().contains("right binding"));
}

#[test]
fn dependency_snapshot_names_effects_and_signals() {
    let effects = Effects::default();
    let mut graph = Graph::default();
    let first = graph.signal("first", 1);
    let second = graph.signal("second", 2);
    effects.register(&mut graph, "sum binding", move |ctx| {
        let _ = ctx.get(first).map_err(|error| error.to_string())?
            + ctx.get(second).map_err(|error| error.to_string())?;
        Ok(())
    });
    effects.flush(&mut graph).unwrap();

    assert_eq!(
        graph.dependencies(),
        vec![DependencyEntry {
            effect: "sum binding".to_owned(),
            signals: vec!["first".to_owned(), "second".to_owned()],
            depth: 0,
        }]
    );
}
