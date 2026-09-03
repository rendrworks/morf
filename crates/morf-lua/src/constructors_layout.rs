//! `ui.Layout { measure = fn, place = fn, ...children }`: a container laid
//! out by the configuration's own two functions.
//!
//! `measure(available, children)` returns the container's width and
//! height; `place(bounds, children)` returns one `{ x, y, width, height }`
//! per child. `available` and `bounds` are `{ width, height }`, `children`
//! a list of `{ width, height }` in tree order, already measured -- once,
//! which is the rule that keeps deep trees cheap. AwesomeWM's `fit` and
//! `layout`, SwiftUI's `sizeThatFits` and `placeSubviews`, in ten lines of
//! Lua for the left-centre-right every bar draws by hand.

use luna::{Callback, CallbackReturn, Context, Function, Table, Value as LuaValue};
use morf_scene::Element;
use std::cell::RefCell;
use std::rc::Rc;

use crate::{configure::*, scene_bindings::*, state::*, types::*};

pub(crate) fn layout_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let clean = Table::new(&ctx);
        let mut measure = None;
        let mut place = None;
        for (key, value) in properties.iter(ctx) {
            let name = match key {
                LuaValue::String(name) => name.display_lossy().to_string(),
                _ => {
                    clean.set(ctx, key, value)?;
                    continue;
                }
            };
            match name.as_str() {
                "measure" | "place" => {
                    let LuaValue::Function(Function::Closure(function)) = value else {
                        return Err(HostError(format!("Layout `{name}` must be a function")).into());
                    };
                    if name == "measure" {
                        measure = Some(ctx.stash(function));
                    } else {
                        place = Some(ctx.stash(function));
                    }
                }
                _ => {
                    clean.set(ctx, key, value)?;
                }
            }
        }
        let (Some(measure), Some(place)) = (measure, place) else {
            return Err(HostError("Layout needs both `measure` and `place`".into()).into());
        };
        let node = create_node(&state, Element::Custom);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        state
            .borrow_mut()
            .custom_layouts
            .insert(node, CustomLayoutFns { measure, place });
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}
