use luna::{
    Callback, CallbackReturn, Closure, Context, Table, UserData, UserRef, Value as LuaValue,
};
use morf_io::Timer as IoTimer;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::{lua_values::*, scene_bindings::*, state::*, table_menu::*};

pub(crate) fn install_time_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let elapsed = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        stack.replace(ctx, timer.started.borrow().elapsed().as_secs_f64());
        Ok(CallbackReturn::Return)
    });
    let elapsed_ms = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        let milliseconds = timer.started.borrow().elapsed().as_millis();
        stack.replace(ctx, i64::try_from(milliseconds).unwrap_or(i64::MAX));
        Ok(CallbackReturn::Return)
    });
    let elapsed_ns = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        let nanoseconds = timer.started.borrow().elapsed().as_nanos();
        stack.replace(ctx, i64::try_from(nanoseconds).unwrap_or(i64::MAX));
        Ok(CallbackReturn::Return)
    });
    let restart = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        let now = Instant::now();
        let elapsed = now.duration_since(*timer.started.borrow());
        *timer.started.borrow_mut() = now;
        stack.replace(ctx, elapsed.as_secs_f64());
        Ok(CallbackReturn::Return)
    });
    let restart_ms = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        let now = Instant::now();
        let elapsed = now.duration_since(*timer.started.borrow());
        *timer.started.borrow_mut() = now;
        stack.replace(ctx, i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
        Ok(CallbackReturn::Return)
    });
    let restart_ns = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer: UserRef<ElapsedTimerToken> = stack.consume(ctx)?;
        let now = Instant::now();
        let elapsed = now.duration_since(*timer.started.borrow());
        *timer.started.borrow_mut() = now;
        stack.replace(ctx, i64::try_from(elapsed.as_nanos()).unwrap_or(i64::MAX));
        Ok(CallbackReturn::Return)
    });
    let elapsed_methods = Table::new(&ctx);
    elapsed_methods.set_field(ctx, "elapsed", elapsed);
    elapsed_methods.set_field(ctx, "elapsed_ms", elapsed_ms);
    elapsed_methods.set_field(ctx, "elapsed_ns", elapsed_ns);
    elapsed_methods.set_field(ctx, "restart", restart);
    elapsed_methods.set_field(ctx, "restart_ms", restart_ms);
    elapsed_methods.set_field(ctx, "restart_ns", restart_ns);
    let elapsed_metatable = Table::new(&ctx);
    elapsed_metatable.set_field(ctx, "__index", elapsed_methods);
    let elapsed_metatable = ctx.stash(elapsed_metatable);
    let elapsed_timer = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let timer = UserData::new_static(
            &ctx,
            ElapsedTimerToken {
                started: RefCell::new(Instant::now()),
            },
        );
        timer.set_metatable(ctx, Some(ctx.fetch(&elapsed_metatable)));
        stack.replace(ctx, timer);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "elapsed_timer", elapsed_timer);
    let system_clock_snapshot = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let clock: UserRef<SystemClockToken> = stack.consume(ctx)?;
            track_clock_dependency(&state, clock.enabled.get());
            stack.replace(ctx, local_time_table(ctx));
            Ok(CallbackReturn::Return)
        }
    });
    let system_clock_format = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let (clock, format): (UserRef<SystemClockToken>, String) = stack.consume(ctx)?;
            if format.len() > 256 || format.as_bytes().contains(&0) {
                return Err(HostError("clock format exceeds 256 bytes".into()).into());
            }
            track_clock_dependency(&state, clock.enabled.get());
            stack.replace(ctx, jiff::Zoned::now().strftime(&format).to_string());
            Ok(CallbackReturn::Return)
        }
    });
    let system_clock_enabled = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let clock: UserRef<SystemClockToken> = stack.consume(ctx)?;
        stack.replace(ctx, clock.enabled.get());
        Ok(CallbackReturn::Return)
    });
    let system_clock_set_enabled = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (clock, enabled): (UserRef<SystemClockToken>, bool) = stack.consume(ctx)?;
        clock.enabled.set(enabled);
        Ok(CallbackReturn::Return)
    });
    let system_clock_precision = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let clock: UserRef<SystemClockToken> = stack.consume(ctx)?;
        stack.replace(ctx, clock.precision.borrow().as_str());
        Ok(CallbackReturn::Return)
    });
    let system_clock_set_precision = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (clock, precision): (UserRef<SystemClockToken>, String) = stack.consume(ctx)?;
        if !matches!(precision.as_str(), "hours" | "minutes" | "seconds") {
            return Err(
                HostError("clock precision must be hours, minutes, or seconds".into()).into(),
            );
        }
        *clock.precision.borrow_mut() = precision;
        Ok(CallbackReturn::Return)
    });
    let system_clock_methods = Table::new(&ctx);
    system_clock_methods.set_field(ctx, "snapshot", system_clock_snapshot);
    system_clock_methods.set_field(ctx, "format", system_clock_format);
    system_clock_methods.set_field(ctx, "enabled", system_clock_enabled);
    system_clock_methods.set_field(ctx, "set_enabled", system_clock_set_enabled);
    system_clock_methods.set_field(ctx, "precision", system_clock_precision);
    system_clock_methods.set_field(ctx, "set_precision", system_clock_set_precision);
    let system_clock_metatable = Table::new(&ctx);
    system_clock_metatable.set_field(ctx, "__index", system_clock_methods);
    let system_clock_metatable = ctx.stash(system_clock_metatable);
    let system_clock = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: LuaValue = stack.consume(ctx)?;
        let (enabled, precision) = match options {
            LuaValue::Nil => (true, "seconds".to_owned()),
            LuaValue::Table(options) => {
                let enabled = match options.get_value(ctx, "enabled") {
                    LuaValue::Nil => true,
                    LuaValue::Boolean(value) => value,
                    _ => return Err(HostError("clock enabled must be boolean".into()).into()),
                };
                let precision = match options.get_value(ctx, "precision") {
                    LuaValue::Nil => "seconds".to_owned(),
                    LuaValue::String(value) => value.display_lossy().to_string(),
                    _ => {
                        return Err(HostError("clock precision must be a string".into()).into());
                    }
                };
                if !matches!(precision.as_str(), "hours" | "minutes" | "seconds") {
                    return Err(HostError(
                        "clock precision must be hours, minutes, or seconds".into(),
                    )
                    .into());
                }
                (enabled, precision)
            }
            _ => return Err(HostError("system_clock options must be a table".into()).into()),
        };
        let clock = UserData::new_static(
            &ctx,
            SystemClockToken {
                enabled: Cell::new(enabled),
                precision: RefCell::new(precision),
            },
        );
        clock.set_metatable(ctx, Some(ctx.fetch(&system_clock_metatable)));
        stack.replace(ctx, clock);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "system_clock", system_clock);
    let easing_value = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (curve, progress): (UserRef<EasingCurveToken>, f64) = stack.consume(ctx)?;
        if !progress.is_finite() {
            return Err(HostError("easing progress must be finite".into()).into());
        }
        stack.replace(ctx, curve.easing.value_at(progress));
        Ok(CallbackReturn::Return)
    });
    /// Reads a Lua number of either kind, if it is one and it is finite.
    pub(crate) fn lua_finite_number(value: LuaValue<'_>) -> Option<f64> {
        match value {
            LuaValue::Integer(value) => Some(value as f64),
            LuaValue::Number(value) if value.is_finite() => Some(value),
            _ => None,
        }
    }

    let easing_interpolate = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (curve, progress, start, end): (UserRef<EasingCurveToken>, f64, LuaValue, LuaValue) =
            stack.consume(ctx)?;
        if !progress.is_finite() {
            return Err(HostError("easing interpolation progress must be finite".into()).into());
        }
        // Lua does not distinguish an integer from a float the way this match
        // did: `interpolate(0.5, 0, 1.0)` writes one of each, and matching the
        // two kinds pairwise rejected exactly that — the most ordinary way to
        // write "from nothing to one".
        if let (Some(start), Some(end)) = (lua_finite_number(start), lua_finite_number(end)) {
            stack.replace(ctx, curve.easing.interpolate(progress, start, end));
            return Ok(CallbackReturn::Return);
        }
        match (start, end) {
            (LuaValue::Table(start), LuaValue::Table(end)) => {
                let fields = if !matches!(start.get_value(ctx, "width"), LuaValue::Nil)
                    || !matches!(end.get_value(ctx, "width"), LuaValue::Nil)
                {
                    &["x", "y", "width", "height"][..]
                } else {
                    &["x", "y"][..]
                };
                let result = Table::new(&ctx);
                for field in fields {
                    let start = table_required_number(ctx, start, field).map_err(HostError)?;
                    let end = table_required_number(ctx, end, field).map_err(HostError)?;
                    result.set_field(ctx, field, curve.easing.interpolate(progress, start, end));
                }
                stack.replace(ctx, result);
            }
            _ => {
                return Err(HostError(
                    "easing interpolation needs two numbers, points, or rectangles".into(),
                )
                .into());
            }
        }
        Ok(CallbackReturn::Return)
    });
    let easing_methods = Table::new(&ctx);
    easing_methods.set_field(ctx, "value_at", easing_value);
    easing_methods.set_field(ctx, "interpolate", easing_interpolate);
    let easing_metatable = Table::new(&ctx);
    easing_metatable.set_field(ctx, "__index", easing_methods);
    let easing_metatable = ctx.stash(easing_metatable);
    let easing_curve = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let value: LuaValue = stack.consume(ctx)?;
        let easing = parse_easing(ctx, value).map_err(HostError)?;
        let curve = UserData::new_static(&ctx, EasingCurveToken { easing });
        curve.set_metatable(ctx, Some(ctx.fetch(&easing_metatable)));
        stack.replace(ctx, curve);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "easing_curve", easing_curve);
}

/// Installs `morf.timer`, which returns a handle that can stop it.
///
/// Here rather than beside the other host services, because a timer is a
/// clock: it belongs with the elapsed timer and the system clock it shares
/// this file with. And because a timer with no handle cannot be stopped,
/// which is how a helper polled every twenty milliseconds went on being polled
/// after the helper was gone.
pub(crate) fn install_timer_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let cancel_state = Rc::clone(&state);
    let timer_cancel = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let token: UserRef<TimerToken> = stack.consume(ctx)?;
        let mut state = cancel_state.borrow_mut();
        let before = state.timers.len();
        state.timers.retain(|timer| timer.id != token.id);
        // Whether there was anything to stop: a second cancel, or a cancel of
        // a one-shot that already fired, is not an error but is worth knowing.
        stack.replace(ctx, state.timers.len() != before);
        Ok(CallbackReturn::Return)
    });
    let active_state = Rc::clone(&state);
    let timer_active = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let token: UserRef<TimerToken> = stack.consume(ctx)?;
        let active = active_state
            .borrow()
            .timers
            .iter()
            .any(|timer| timer.id == token.id);
        stack.replace(ctx, active);
        Ok(CallbackReturn::Return)
    });
    let timer_methods = Table::new(&ctx);
    timer_methods.set_field(ctx, "cancel", timer_cancel);
    timer_methods.set_field(ctx, "active", timer_active);
    let timer_metatable = Table::new(&ctx);
    timer_metatable.set_field(ctx, "__index", timer_methods);
    let timer_metatable = ctx.stash(timer_metatable);

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
        let mut state = timer_state.borrow_mut();
        let id = state.next_timer_id();
        state.timers.push(PendingTimer {
            id,
            timer,
            callback: ctx.stash(callback),
            repeat,
            interval,
            node: None,
        });
        drop(state);
        let userdata = UserData::new_static(&ctx, TimerToken { id });
        userdata.set_metatable(ctx, Some(ctx.fetch(&timer_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "timer", timer);
}
