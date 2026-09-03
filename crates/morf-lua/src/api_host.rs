use luna::{Callback, CallbackReturn, Closure, Context, Function, Table, Value as LuaValue};
use morf_io::Timer as IoTimer;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::{scene_bindings::*, state::*, surface_types::*, types::*};

pub(crate) fn install_host_service_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
    screen: Option<&Screen>,
) {
    let idle_state = Rc::clone(&state);
    let idle_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        // The third argument asks for *input* idleness: time since the person
        // last touched anything, ignoring idle inhibitors. A media player
        // inhibits idle so the screen stays on, and a shell that dims its own
        // bar after a minute of no input still wants to know about the minute.
        let (milliseconds, callback, input_only): (i64, Closure, Option<bool>) =
            stack.consume(ctx)?;
        let milliseconds = u32::try_from(milliseconds)
            .map_err(|_| HostError("idle timeout must fit an unsigned 32-bit value".into()))?;
        let key = (milliseconds, input_only.unwrap_or(false));
        let mut state = idle_state.borrow_mut();
        let callback_count = state.idle_callbacks.values().map(Vec::len).sum::<usize>();
        if callback_count >= 256 {
            return Err(HostError("idle callback limit reached".into()).into());
        }
        if !state.idle_callbacks.contains_key(&key) && state.idle_callbacks.len() >= 64 {
            return Err(HostError("idle timeout limit reached".into()).into());
        }
        state
            .idle_callbacks
            .entry(key)
            .or_default()
            .push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let inhibit_state = Rc::clone(&state);
    // The other direction from `subscribe`: not "tell me when the session goes
    // idle" but "do not let it". A video player, a presentation, a long copy —
    // each wants the screen to stay awake while nobody touches the input.
    let idle_inhibit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let inhibited: bool = stack.consume(ctx)?;
        let mut state = inhibit_state.borrow_mut();
        state.idle_inhibited = inhibited;
        state.idle_inhibit_changed = true;
        Ok(CallbackReturn::Return)
    });
    let idle = Table::new(&ctx);
    idle.set_field(ctx, "subscribe", idle_subscribe);
    idle.set_field(ctx, "inhibit", idle_inhibit);
    morf.set_field(ctx, "idle", idle);
    let output_power_state = Rc::clone(&state);
    let output_power_set = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let mode: String = stack.consume(ctx)?;
        let on = match mode.as_str() {
            "off" => false,
            "on" => true,
            _ => return Err(HostError("output power mode must be `on` or `off`".into()).into()),
        };
        let mut state = output_power_state.borrow_mut();
        if state.output_power_requests.len() >= 64 {
            return Err(HostError("output power request limit reached".into()).into());
        }
        state.output_power_requests.push(on);
        Ok(CallbackReturn::Return)
    });
    let output_power = Table::new(&ctx);
    output_power.set_field(ctx, "set", output_power_set);
    morf.set_field(ctx, "output_power", output_power);
    let clipboard_set_state = Rc::clone(&state);
    let clipboard_set = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let text: String = stack.consume(ctx)?;
        if text.len() > 1_048_576 {
            return Err(HostError("clipboard text limit reached".into()).into());
        }
        let mut state = clipboard_set_state.borrow_mut();
        if state.clipboard_requests.len() >= 64 {
            return Err(HostError("clipboard request limit reached".into()).into());
        }
        state.clipboard_requests.push(text);
        Ok(CallbackReturn::Return)
    });
    let clipboard_subscribe_state = Rc::clone(&state);
    let clipboard_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        let mut state = clipboard_subscribe_state.borrow_mut();
        if state.clipboard_callbacks.len() >= 64 {
            return Err(HostError("clipboard callback limit reached".into()).into());
        }
        state.clipboard_callbacks.push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let clipboard = Table::new(&ctx);
    clipboard.set_field(ctx, "set", clipboard_set);
    clipboard.set_field(ctx, "subscribe", clipboard_subscribe);
    morf.set_field(ctx, "clipboard", clipboard);
    let screencopy_state = Rc::clone(&state);
    let screencopy_capture = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (include_cursor, callback): (bool, Closure) = stack.consume(ctx)?;
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
        });
        state.screencopy_callbacks.insert(id, ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let window_state = Rc::clone(&state);
    // `capture_window(identifier, handler)` — the same frame, the same handler
    // shape, one window instead of the whole output. Separate from `capture`
    // rather than an extra argument to it because the two can fail for
    // different reasons and a configuration wants to know which.
    let screencopy_window = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (identifier, callback): (String, Closure) = stack.consume(ctx)?;
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
        });
        state.screencopy_callbacks.insert(id, ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    let screencopy = Table::new(&ctx);
    screencopy.set_field(ctx, "capture", screencopy_capture);
    screencopy.set_field(ctx, "capture_window", screencopy_window);
    morf.set_field(ctx, "screencopy", screencopy);
    let virtual_key_state = Rc::clone(&state);
    let virtual_key = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (keycode, pressed): (i64, bool) = stack.consume(ctx)?;
        let keycode = u32::try_from(keycode)
            .map_err(|_| HostError("virtual keycode must fit an unsigned 32-bit value".into()))?;
        let mut state = virtual_key_state.borrow_mut();
        if state.virtual_keyboard_requests.len() >= 256 {
            return Err(HostError("virtual keyboard request limit reached".into()).into());
        }
        state
            .virtual_keyboard_requests
            .push(VirtualKeyboardRequest::Key { keycode, pressed });
        Ok(CallbackReturn::Return)
    });
    let virtual_modifiers_state = Rc::clone(&state);
    let virtual_modifiers = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let values: (i64, i64, i64, i64) = stack.consume(ctx)?;
        let request = VirtualKeyboardRequest::Modifiers {
            depressed: u32::try_from(values.0)
                .map_err(|_| HostError("depressed modifiers must fit u32".into()))?,
            latched: u32::try_from(values.1)
                .map_err(|_| HostError("latched modifiers must fit u32".into()))?,
            locked: u32::try_from(values.2)
                .map_err(|_| HostError("locked modifiers must fit u32".into()))?,
            group: u32::try_from(values.3)
                .map_err(|_| HostError("keyboard group must fit u32".into()))?,
        };
        let mut state = virtual_modifiers_state.borrow_mut();
        if state.virtual_keyboard_requests.len() >= 256 {
            return Err(HostError("virtual keyboard request limit reached".into()).into());
        }
        state.virtual_keyboard_requests.push(request);
        Ok(CallbackReturn::Return)
    });
    let virtual_keyboard = Table::new(&ctx);
    virtual_keyboard.set_field(ctx, "key", virtual_key);
    virtual_keyboard.set_field(ctx, "modifiers", virtual_modifiers);
    morf.set_field(ctx, "virtual_keyboard", virtual_keyboard);
    let input_method_subscribe_state = Rc::clone(&state);
    let input_method_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        let mut state = input_method_subscribe_state.borrow_mut();
        if state.input_method_callbacks.len() >= 64 {
            return Err(HostError("input method callback limit reached".into()).into());
        }
        state.input_method_callbacks.push(ctx.stash(callback));
        state.input_method_enable_requested = true;
        Ok(CallbackReturn::Return)
    });
    let input_method_commit_state = Rc::clone(&state);
    let input_method_commit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let text: String = stack.consume(ctx)?;
        if text.len() > 4_000 {
            return Err(HostError("input method text limit reached".into()).into());
        }
        let mut state = input_method_commit_state.borrow_mut();
        if state.input_method_requests.len() >= 256 {
            return Err(HostError("input method request limit reached".into()).into());
        }
        state
            .input_method_requests
            .push(InputMethodRequest::Commit(text));
        Ok(CallbackReturn::Return)
    });
    let input_method_preedit_state = Rc::clone(&state);
    let input_method_preedit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (text, begin, end): (String, i64, i64) = stack.consume(ctx)?;
        if text.len() > 4_000 {
            return Err(HostError("input method text limit reached".into()).into());
        }
        let begin = i32::try_from(begin)
            .map_err(|_| HostError("preedit cursor start must fit i32".into()))?;
        let end =
            i32::try_from(end).map_err(|_| HostError("preedit cursor end must fit i32".into()))?;
        let mut state = input_method_preedit_state.borrow_mut();
        if state.input_method_requests.len() >= 256 {
            return Err(HostError("input method request limit reached".into()).into());
        }
        state
            .input_method_requests
            .push(InputMethodRequest::Preedit { text, begin, end });
        Ok(CallbackReturn::Return)
    });
    let input_method_delete_state = Rc::clone(&state);
    let input_method_delete = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (before, after): (i64, i64) = stack.consume(ctx)?;
        let before = u32::try_from(before)
            .map_err(|_| HostError("delete before length must fit u32".into()))?;
        let after = u32::try_from(after)
            .map_err(|_| HostError("delete after length must fit u32".into()))?;
        let mut state = input_method_delete_state.borrow_mut();
        if state.input_method_requests.len() >= 256 {
            return Err(HostError("input method request limit reached".into()).into());
        }
        state
            .input_method_requests
            .push(InputMethodRequest::Delete { before, after });
        Ok(CallbackReturn::Return)
    });
    let input_method = Table::new(&ctx);
    input_method.set_field(ctx, "subscribe", input_method_subscribe);
    input_method.set_field(ctx, "commit", input_method_commit);
    input_method.set_field(ctx, "preedit", input_method_preedit);
    input_method.set_field(ctx, "delete", input_method_delete);
    morf.set_field(ctx, "input_method", input_method);
    let text_input_subscribe_state = Rc::clone(&state);
    let text_input_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        let mut state = text_input_subscribe_state.borrow_mut();
        if state.text_input_callbacks.len() >= 64 {
            return Err(HostError("text input callback limit reached".into()).into());
        }
        state.text_input_callbacks.push(ctx.stash(callback));
        state.text_input_enable_requested = true;
        Ok(CallbackReturn::Return)
    });
    let text_input_disable_state = Rc::clone(&state);
    let text_input_disable = Callback::from_fn(&ctx, move |_, _, _| {
        let mut state = text_input_disable_state.borrow_mut();
        if state.text_input_requests.len() >= 256 {
            return Err(HostError("text input request limit reached".into()).into());
        }
        state.text_input_requests.push(TextInputRequest::Disable);
        Ok(CallbackReturn::Return)
    });
    let text_input_surrounding_state = Rc::clone(&state);
    let text_input_surrounding = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (text, cursor, anchor): (String, i64, i64) = stack.consume(ctx)?;
        if text.len() > 4_000 {
            return Err(HostError("text input text limit reached".into()).into());
        }
        let cursor = i32::try_from(cursor)
            .map_err(|_| HostError("text input cursor must fit i32".into()))?;
        let anchor = i32::try_from(anchor)
            .map_err(|_| HostError("text input anchor must fit i32".into()))?;
        let mut state = text_input_surrounding_state.borrow_mut();
        if state.text_input_requests.len() >= 256 {
            return Err(HostError("text input request limit reached".into()).into());
        }
        state
            .text_input_requests
            .push(TextInputRequest::Surrounding {
                text,
                cursor,
                anchor,
            });
        Ok(CallbackReturn::Return)
    });
    let text_input_content_state = Rc::clone(&state);
    let text_input_content = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (hints, purpose): (i64, i64) = stack.consume(ctx)?;
        let hints =
            u32::try_from(hints).map_err(|_| HostError("text input hints must fit u32".into()))?;
        let purpose = u32::try_from(purpose)
            .map_err(|_| HostError("text input purpose must fit u32".into()))?;
        let mut state = text_input_content_state.borrow_mut();
        if state.text_input_requests.len() >= 256 {
            return Err(HostError("text input request limit reached".into()).into());
        }
        state
            .text_input_requests
            .push(TextInputRequest::ContentType { hints, purpose });
        Ok(CallbackReturn::Return)
    });
    let text_input_rect_state = Rc::clone(&state);
    let text_input_rect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let values: (i64, i64, i64, i64) = stack.consume(ctx)?;
        let request = TextInputRequest::CursorRect {
            x: i32::try_from(values.0).map_err(|_| HostError("cursor x must fit i32".into()))?,
            y: i32::try_from(values.1).map_err(|_| HostError("cursor y must fit i32".into()))?,
            width: i32::try_from(values.2)
                .map_err(|_| HostError("cursor width must fit i32".into()))?,
            height: i32::try_from(values.3)
                .map_err(|_| HostError("cursor height must fit i32".into()))?,
        };
        let mut state = text_input_rect_state.borrow_mut();
        if state.text_input_requests.len() >= 256 {
            return Err(HostError("text input request limit reached".into()).into());
        }
        state.text_input_requests.push(request);
        Ok(CallbackReturn::Return)
    });
    let text_input = Table::new(&ctx);
    text_input.set_field(ctx, "subscribe", text_input_subscribe);
    text_input.set_field(ctx, "disable", text_input_disable);
    text_input.set_field(ctx, "surrounding", text_input_surrounding);
    text_input.set_field(ctx, "content_type", text_input_content);
    text_input.set_field(ctx, "cursor_rect", text_input_rect);
    morf.set_field(ctx, "text_input", text_input);
    let timer_state = Rc::clone(&state);
    let timer = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (milliseconds, callback, repeat): (f64, Closure, LuaValue) = stack.consume(ctx)?;
        if !milliseconds.is_finite() || milliseconds <= 0.0 {
            return Err(HostError("timer interval must be finite and positive".into()).into());
        }
        let repeat = match repeat {
            LuaValue::Nil => true,
            LuaValue::Boolean(value) => value,
            _ => return Err(HostError("timer repeat must be boolean".into()).into()),
        };
        let timer = IoTimer::every(Duration::from_secs_f64(milliseconds / 1_000.0))
            .map_err(|error| HostError(error.to_string()))?;
        let interval = Duration::from_secs_f64(milliseconds / 1_000.0);
        timer_state.borrow_mut().timers.push(PendingTimer {
            timer,
            callback: ctx.stash(callback),
            repeat,
            interval,
            node: None,
        });
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "timer", timer);
    let ipc_register = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (_table, name, value): (Table, String, LuaValue) = stack.consume(ctx)?;
            match value {
                LuaValue::Function(Function::Closure(closure)) => {
                    state
                        .borrow_mut()
                        .ipc_handlers
                        .insert(name, ctx.stash(closure));
                }
                LuaValue::Nil => {
                    state.borrow_mut().ipc_handlers.remove(&name);
                }
                _ => {
                    return Err(
                        HostError("morf.ipc values must be functions or nil".to_owned()).into(),
                    );
                }
            }
            Ok(CallbackReturn::Return)
        }
    });
    let ipc_metatable = Table::new(&ctx);
    ipc_metatable.set_field(ctx, "__newindex", ipc_register);
    let ipc = Table::new(&ctx);
    ipc.set_metatable(ctx, Some(ipc_metatable));
    morf.set_field(ctx, "ipc", ipc);
    let screens = Table::new(&ctx);
    // `morf.screens` is ordered: index 1 is always the output this
    // configuration instance drives, and `Runtime::set_screens` appends the
    // compositor's remaining outputs after it.
    if let Some(screen) = screen {
        screens
            .set(ctx, 1, screen_entry(ctx, screen))
            .expect("screen table accepts integer keys");
    }
    morf.set_field(ctx, "screens", screens);

    // Every window the compositor reports, filled by `Runtime::set_windows` and
    // updated in place. Empty here rather than absent so a configuration can
    // hold it and watch it from the first line, before any compositor has said
    // anything — and so `#morf.windows` is a number rather than an error on a
    // compositor that does not report them at all.
    let windows = Table::new(&ctx);
    morf.set_field(ctx, "windows", windows);
    // Filled once the compositor connection is up; empty rather than absent so
    // a configuration can index it from its first line.
    morf.set_field(ctx, "capabilities", Table::new(&ctx));
    crate::api_compositor::install_compositor_api(ctx, Rc::clone(&state), morf);

    // Empty rather than absent, so a configuration can hold it and watch it
    // from its first line — and so `#morf.workspaces` is a number rather than
    // an error on a compositor that does not speak the protocol at all.
}

/// Builds the Lua table describing one output.
pub(crate) fn screen_entry<'gc>(ctx: Context<'gc>, screen: &Screen) -> Table<'gc> {
    let value = Table::new(&ctx);
    value.set_field(ctx, "id", screen.id as i64);
    value.set_field(ctx, "name", screen.name.as_str());
    value.set_field(ctx, "make", screen.make.as_str());
    value.set_field(ctx, "model", screen.model.as_str());
    value.set_field(
        ctx,
        "description",
        screen
            .description
            .as_deref()
            .map_or(LuaValue::Nil, |description| {
                LuaValue::String(ctx.intern(description.as_bytes()))
            }),
    );
    value.set_field(
        ctx,
        "x",
        screen.position.map_or(LuaValue::Nil, |position| {
            LuaValue::Integer(position.0 as i64)
        }),
    );
    value.set_field(
        ctx,
        "y",
        screen.position.map_or(LuaValue::Nil, |position| {
            LuaValue::Integer(position.1 as i64)
        }),
    );
    value.set_field(
        ctx,
        "width",
        screen
            .width
            .map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
    );
    value.set_field(
        ctx,
        "height",
        screen
            .height
            .map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
    );
    value.set_field(ctx, "scale", screen.scale as i64);
    value.set_field(ctx, "device_pixel_ratio", screen.scale as i64);
    value.set_field(ctx, "transform", screen.transform.as_str());
    let physical_width = screen.physical_size.map(|size| size.0);
    let physical_height = screen.physical_size.map(|size| size.1);
    value.set_field(
        ctx,
        "physical_width_mm",
        physical_width.map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
    );
    value.set_field(
        ctx,
        "physical_height_mm",
        physical_height.map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
    );
    let physical_density = screen_density(screen);
    value.set_field(
        ctx,
        "physical_pixel_density",
        physical_density.map_or(LuaValue::Nil, LuaValue::Number),
    );
    value.set_field(
        ctx,
        "logical_pixel_density",
        physical_density.map_or(LuaValue::Nil, |density| {
            LuaValue::Number(density / f64::from(screen.scale.max(1)))
        }),
    );
    value.set_field(ctx, "orientation", screen_orientation(screen));
    value.set_field(
        ctx,
        "primary_orientation",
        screen_primary_orientation(screen),
    );
    value.set_field(ctx, "serial_number", LuaValue::Nil);
    value
}
