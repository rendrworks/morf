//! Running a `ui.Layout` container's functions from inside a layout pass.
//!
//! The layout pass holds the scene borrowed; the functions run in Lua
//! beside it with a fuel budget, take and return plain numbers, and must
//! not write to nodes or signals -- a write from inside layout is refused
//! with a message rather than allowed to corrupt the pass it is part of.

use std::collections::HashMap;

use luna::{Context, Executor, Lua, StashedClosure, Table, Value as LuaValue, Variadic};
use morf_layout::{CustomLayout, Geometry, Layout, Size, TextMeasurer};
use morf_scene::NodeHandle;

use crate::{reactive_execute::*, state::*, types::*};

pub(crate) struct LuaLayoutHost<'a> {
    pub(crate) lua: &'a mut Lua,
    pub(crate) layouts: &'a HashMap<NodeHandle, CustomLayoutFns>,
    pub(crate) limits: Limits,
}

fn size_table<'gc>(ctx: Context<'gc>, size: Size) -> Table<'gc> {
    let table = Table::new(&ctx);
    table.set_field(ctx, "width", size.width);
    table.set_field(ctx, "height", size.height);
    table
}

fn children_table<'gc>(ctx: Context<'gc>, children: &[Size]) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (index, child) in children.iter().enumerate() {
        table
            .set(ctx, index as i64 + 1, size_table(ctx, *child))
            .expect("children table accepts integer keys");
    }
    table
}

fn number<'gc>(ctx: Context<'gc>, table: Table<'gc>, key: &str) -> Option<f64> {
    match table.get_value(ctx, key) {
        LuaValue::Integer(value) => Some(value as f64),
        LuaValue::Number(value) => Some(value),
        _ => None,
    }
}

fn call<'gc>(
    ctx: Context<'gc>,
    function: &StashedClosure,
    args: Vec<LuaValue<'gc>>,
    limits: Limits,
    what: &str,
) -> Result<Vec<LuaValue<'gc>>, String> {
    let executor = Executor::start(ctx, ctx.fetch(function).into(), Variadic(args));
    drive_executor(ctx, executor, limits, limits.effect_fuel, what)?;
    match executor.take_result::<Variadic<Vec<LuaValue>>>(ctx) {
        Ok(Ok(Variadic(values))) => Ok(values),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

impl CustomLayout for LuaLayoutHost<'_> {
    fn measure(
        &mut self,
        node: NodeHandle,
        available: Size,
        children: &[Size],
    ) -> Result<Size, String> {
        let functions = self
            .layouts
            .get(&node)
            .ok_or("Layout node has no functions")?
            .clone();
        let limits = self.limits;
        self.lua.enter(|ctx| {
            let args = vec![
                LuaValue::Table(size_table(ctx, available)),
                LuaValue::Table(children_table(ctx, children)),
            ];
            let values = call(ctx, &functions.measure, args, limits, "Layout measure")?;
            let dimension = |value: Option<&LuaValue>| match value {
                Some(LuaValue::Integer(value)) => Ok(*value as f64),
                Some(LuaValue::Number(value)) if value.is_finite() => Ok(*value),
                _ => Err("Layout measure must return width, height".to_owned()),
            };
            Ok(Size {
                width: dimension(values.first())?.max(0.0),
                height: dimension(values.get(1))?.max(0.0),
            })
        })
    }

    fn place(
        &mut self,
        node: NodeHandle,
        bounds: Size,
        children: &[Size],
    ) -> Result<Vec<Geometry>, String> {
        let functions = self
            .layouts
            .get(&node)
            .ok_or("Layout node has no functions")?
            .clone();
        let limits = self.limits;
        self.lua.enter(|ctx| {
            let args = vec![
                LuaValue::Table(size_table(ctx, bounds)),
                LuaValue::Table(children_table(ctx, children)),
            ];
            let values = call(ctx, &functions.place, args, limits, "Layout place")?;
            let Some(LuaValue::Table(placements)) = values.first() else {
                return Err("Layout place must return a list of placements".to_owned());
            };
            let mut out = Vec::with_capacity(children.len());
            for (index, child) in children.iter().enumerate() {
                let entry = match placements.get_value(ctx, index as i64 + 1) {
                    LuaValue::Table(entry) => entry,
                    LuaValue::Nil => {
                        out.push(Geometry {
                            x: 0.0,
                            y: 0.0,
                            width: child.width,
                            height: child.height,
                        });
                        continue;
                    }
                    _ => return Err(format!("Layout placement {} must be a table", index + 1)),
                };
                out.push(Geometry {
                    x: number(ctx, entry, "x").unwrap_or(0.0),
                    y: number(ctx, entry, "y").unwrap_or(0.0),
                    width: number(ctx, entry, "width").unwrap_or(child.width).max(0.0),
                    height: number(ctx, entry, "height")
                        .unwrap_or(child.height)
                        .max(0.0),
                });
            }
            Ok(out)
        })
    }
}

impl Runtime {
    /// Lays out a surface, running any `ui.Layout` container's functions.
    ///
    /// The scene stays borrowed for the whole pass, so those functions may
    /// read nothing from it and write nothing to it; they are given numbers
    /// and return numbers.
    pub fn compute_layout(
        &mut self,
        root: NodeHandle,
        available: Size,
        text: &mut impl TextMeasurer,
    ) -> Result<Layout, String> {
        let reactive = self.reactive.borrow();
        let mut host = LuaLayoutHost {
            lua: &mut self.lua,
            layouts: &reactive.custom_layouts,
            limits: self.limits,
        };
        Layout::compute_with(&reactive.scene, root, available, text, &mut host)
            .map_err(|error| error.to_string())
    }
}
