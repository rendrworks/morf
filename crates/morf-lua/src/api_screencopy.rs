//! `morf.screencopy`: asking the compositor for a picture of the screen.
//!
//! Split from the host API at the line gate. Two ways to ask -- an output,
//! or one window by the identifier `morf.windows` reported -- and one way
//! to let a picture go, since nothing collects a published capture on its
//! own.

use luna::{Callback, CallbackReturn, Closure, Context, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::HostError, state::*, surface_types::*};

pub(crate) fn install_screencopy_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let screencopy_state = Rc::clone(&state);
    // `capture(include_cursor, handler, options)`: `options.gpu` asks for
    // the picture to stay on the GPU, which is the difference between a
    // thumbnail that costs two copies of the screen and one that costs none.
    let screencopy_capture = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (include_cursor, callback, options): (bool, Closure, Option<Table>) =
            stack.consume(ctx)?;
        let (gpu, name) = capture_options(ctx, options);
        let mut state = screencopy_state.borrow_mut();
        if state.screencopy_callbacks.len() >= 4 {
            return Err(HostError("screencopy request limit reached".into()).into());
        }
        let id = state.next_screencopy;
        state.next_screencopy = state.next_screencopy.wrapping_add(1);
        state.screencopy_requests.push(ScreencopyRequest {
            id,
            include_cursor,
            window: None,
            gpu,
            name: name.clone(),
        });
        if let Some(name) = name {
            state.screencopy_names.insert(id, name);
        }
        state.screencopy_callbacks.insert(id, ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let window_state = Rc::clone(&state);
    // `capture_window(identifier, handler)` — the same frame, the same handler
    // shape, one window instead of the whole output. Separate from `capture`
    // rather than an extra argument to it because the two can fail for
    // different reasons and a configuration wants to know which.
    let screencopy_window = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (identifier, callback, options): (String, Closure, Option<Table>) =
            stack.consume(ctx)?;
        let (gpu, name) = capture_options(ctx, options);
        let mut state = window_state.borrow_mut();
        if state.screencopy_callbacks.len() >= 4 {
            return Err(HostError("screencopy request limit reached".into()).into());
        }
        let id = state.next_screencopy;
        state.next_screencopy = state.next_screencopy.wrapping_add(1);
        state.screencopy_requests.push(ScreencopyRequest {
            id,
            include_cursor: false,
            window: Some(identifier),
            gpu,
            name: name.clone(),
        });
        if let Some(name) = name {
            state.screencopy_names.insert(id, name);
        }
        state.screencopy_callbacks.insert(id, ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    // `release(source)`: the picture is as large as the screen, on the GPU
    // or in memory, and nothing collects it on its own -- only the
    // configuration knows when the window it showed has gone.
    let release_state = Rc::clone(&state);
    let screencopy_release = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let source: String = stack.consume(ctx)?;
        release_state.borrow_mut().screencopy_releases.push(source);
        Ok(CallbackReturn::Return)
    });
    let screencopy = Table::new(&ctx);
    screencopy.set_field(ctx, "capture", screencopy_capture);
    screencopy.set_field(ctx, "capture_window", screencopy_window);
    screencopy.set_field(ctx, "release", screencopy_release);
    morf.set_field(ctx, "screencopy", screencopy);
}

/// What a capture's `options` table asks for: the GPU, and a name.
fn capture_options<'gc>(ctx: Context<'gc>, options: Option<Table<'gc>>) -> (bool, Option<String>) {
    let Some(options) = options else {
        return (false, None);
    };
    let gpu = matches!(options.get_value(ctx, "gpu"), LuaValue::Boolean(true));
    let name = match options.get_value(ctx, "name") {
        LuaValue::String(name) => Some(name.display_lossy().to_string()),
        _ => None,
    };
    (gpu, name)
}
