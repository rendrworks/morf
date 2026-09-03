//! Named states, their transitions, and the `when` that lets a state
//! choose itself.
//!
//! Split from `configure` at the line gate.

use luna::{Closure, Context, Executor, Function, Table, Value as LuaValue, Variadic};
use morf_scene::{Behavior, NodeHandle, Value as SceneValue};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use crate::configure::parse_rotation_direction;
use crate::reactive_execute::drive_executor;
use crate::states::*;
use crate::{lua_values::*, reactive_bindings::*, state::*, table_menu::*, types::*};

/// Parses `states` and `transitions`.
///
/// Returns the selector for states that carry a `when`: a binding that
/// picks the first state, in name order, whose `when` is true, else the
/// state named `default`, else nothing -- in which case the node keeps the
/// state it has.
pub(crate) fn configure_states<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    limits: Limits,
    node: NodeHandle,
    states: LuaValue<'gc>,
    transitions: LuaValue<'gc>,
) -> Result<Option<Closure<'gc>>, String> {
    let LuaValue::Table(states) = states else {
        return Err("states must be a name-keyed table".into());
    };
    let mut definitions = HashMap::new();
    for (name, definition) in states.iter(ctx) {
        let LuaValue::String(name) = name else {
            return Err("state names must be strings".into());
        };
        let LuaValue::Table(definition) = definition else {
            return Err("each state must be a table".into());
        };
        let mut properties = Vec::new();
        let mut anchors = None;
        let mut parent = None;
        let mut when = None;
        for (key, value) in definition.iter(ctx) {
            let LuaValue::String(key) = key else {
                return Err("state fields must be strings".into());
            };
            match key.display_lossy().to_string().as_str() {
                "property_changes" => {
                    let LuaValue::Table(changes) = value else {
                        return Err("property_changes must be a table".into());
                    };
                    for (property, value) in changes.iter(ctx) {
                        let LuaValue::String(property) = property else {
                            return Err("property_changes keys must be strings".into());
                        };
                        let property = property.display_lossy().to_string();
                        if !state
                            .borrow()
                            .scene
                            .has_property(node, &property)
                            .map_err(|error| error.to_string())?
                        {
                            return Err(format!("state changes unknown property `{property}`"));
                        }
                        let value = match value {
                            LuaValue::Function(Function::Closure(closure)) => {
                                StateValue::Binding(ctx.stash(closure))
                            }
                            value => StateValue::Value(lua_to_scene(ctx, value, 0)?),
                        };
                        properties.push((property, value));
                    }
                }
                "anchors" | "anchor_changes" => {
                    let SceneValue::Map(value) = lua_to_scene(ctx, value, 0)? else {
                        return Err("anchor_changes must be a table".into());
                    };
                    anchors = Some(value);
                }
                "when" => {
                    let LuaValue::Function(Function::Closure(closure)) = value else {
                        return Err("a state's `when` must be a function".into());
                    };
                    when = Some(ctx.stash(closure));
                }
                "parent" | "parent_change" => {
                    let LuaValue::UserData(value) = value else {
                        return Err("parent_change must be a morf node".into());
                    };
                    parent = Some(
                        value
                            .downcast_static::<NodeToken>()
                            .map_err(|_| "parent_change must be a morf node".to_owned())?
                            .handle,
                    );
                }
                field => return Err(format!("unknown state field `{field}`")),
            }
        }
        definitions.insert(
            name.display_lossy().to_string(),
            StateDefinition {
                properties,
                anchors,
                parent,
                when,
            },
        );
    }
    let selector = build_state_selector(ctx, limits, &definitions)?;
    let mut parsed_transitions = Vec::new();
    if let LuaValue::Table(transitions) = transitions {
        for (_, transition) in transitions.iter(ctx) {
            let LuaValue::Table(transition) = transition else {
                return Err("each transition must be a table".into());
            };
            let from = table_string(ctx, transition, "from", "*")?;
            let to = table_string(ctx, transition, "to", "*")?;
            let reversible = match transition.get_value(ctx, "reversible") {
                LuaValue::Nil => false,
                LuaValue::Boolean(value) => value,
                _ => return Err("transition reversible must be boolean".into()),
            };
            let duration = table_number(ctx, transition, "duration", 250.0)?;
            if duration < 0.0 {
                return Err("transition duration cannot be negative".into());
            }
            parsed_transitions.push(StateTransition {
                from,
                to,
                reversible,
                behavior: Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing: parse_easing(ctx, transition.get_value(ctx, "easing"))?,
                    rotation_direction: parse_rotation_direction(ctx, transition)?,
                    ..Behavior::default()
                },
            });
        }
    } else if !matches!(transitions, LuaValue::Nil) {
        return Err("transitions must be an array table".into());
    }
    state.borrow_mut().states.insert(
        node,
        StateSet {
            definitions,
            transitions: parsed_transitions,
            current: None,
        },
    );
    Ok(selector)
}

/// One Lua function that asks each `when` in turn, built once per node.
///
/// Name order rather than declaration order, because a Lua table has no
/// declaration order to offer; two states true at once are the author's
/// to sort out.
fn build_state_selector<'gc>(
    ctx: Context<'gc>,
    limits: Limits,
    definitions: &HashMap<String, StateDefinition>,
) -> Result<Option<Closure<'gc>>, String> {
    let mut conditional = definitions
        .iter()
        .filter_map(|(name, definition)| definition.when.as_ref().map(|when| (name, when)))
        .collect::<Vec<_>>();
    if conditional.is_empty() {
        return Ok(None);
    }
    conditional.sort_by(|(a, _), (b, _)| a.cmp(b));
    let names = Table::new(&ctx);
    let tests = Table::new(&ctx);
    for (index, (name, when)) in conditional.iter().enumerate() {
        names
            .set(ctx, index as i64 + 1, ctx.intern(name.as_bytes()))
            .map_err(|error| error.to_string())?;
        tests
            .set(ctx, index as i64 + 1, ctx.fetch(*when))
            .map_err(|error| error.to_string())?;
    }
    let fallback = if definitions.contains_key("default") {
        LuaValue::String(ctx.intern(b"default"))
    } else {
        LuaValue::Nil
    };
    let source = br#"
        local names, tests, fallback = ...
        return function()
            for index = 1, #names do
                if tests[index]() then return names[index] end
            end
            return fallback
        end
    "#;
    let factory = Closure::load(ctx, Some("state selector"), &source[..])
        .map_err(|error| error.to_string())?;
    let executor = Executor::start(
        ctx,
        factory.into(),
        Variadic(vec![
            LuaValue::Table(names),
            LuaValue::Table(tests),
            fallback,
        ]),
    );
    drive_executor(ctx, executor, limits, limits.effect_fuel, "state selector")?;
    match executor.take_result::<Closure>(ctx) {
        Ok(Ok(closure)) => Ok(Some(closure)),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}
