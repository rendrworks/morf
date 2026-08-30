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
    let mut graph = Graph::default();
    let first = graph.signal("first", 1);
    let second = graph.signal("second", 2);
    graph.effect("sum binding", move |ctx| {
        let _ = ctx.get(first).map_err(|error| error.to_string())?
            + ctx.get(second).map_err(|error| error.to_string())?;
        Ok(())
    });
    graph.flush().unwrap();

    assert_eq!(
        graph.dependencies(),
        vec![DependencyEntry {
            effect: "sum binding".to_owned(),
            signals: vec!["first".to_owned(), "second".to_owned()],
            depth: 0,
        }]
    );
}
