//! Owning a bus name from a configuration.
//!
//! The client half of D-Bus has been reachable from Lua for a long time:
//! `morf.dbus.proxy` calls anything on the bus. This is the other half, and it
//! is the difference between reading the session and being part of it. A
//! notification server, a tray watcher, a polkit agent, an MPRIS player — every
//! one of those is a name plus a handful of methods, and none of them can be
//! written while a configuration can only make calls.
//!
//! The engine already had all of it. `morf_io::DbusService` owns a name, hands
//! out arriving calls and answers them; it was written, tested, and reachable
//! from nowhere. What was missing was this file.

use luna::{
    Callback, CallbackReturn, Closure, Context, Table, UserData, UserRef, Value as LuaValue,
};
use morf_io::{Bus, DbusService, NameOutcome};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::HostError, serialization::lua_to_dbus, state::*};

/// Installs `morf.dbus.serve` and the methods on what it returns.
///
/// `dbus` is the table `morf.dbus`, so this adds to the client API rather than
/// standing beside it: one table, both halves.
pub(crate) fn install_dbus_serve_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    dbus: Table<'gc>,
) {
    let service_name = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let service: UserRef<DbusServiceToken> = stack.consume(ctx)?;
        let name = service.service.borrow().name().to_owned();
        stack.replace(ctx, name);
        Ok(CallbackReturn::Return)
    });
    // Replying is a separate call rather than the handler's return value,
    // because a method need not be answered on the turn it arrives. A
    // configuration that has to read a file or wait for a user before it can
    // answer holds the id and replies later; the caller waits, which is what it
    // was going to do anyway.
    let service_reply = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (service, id, value): (UserRef<DbusServiceToken>, i64, LuaValue) =
            stack.consume(ctx)?;
        let value = lua_to_dbus(ctx, value, 0).map_err(HostError)?;
        service
            .service
            .borrow_mut()
            .reply(call_id(id)?, &value)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let service_reply_error = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (service, id, name, message): (UserRef<DbusServiceToken>, i64, String, String) =
            stack.consume(ctx)?;
        service
            .service
            .borrow_mut()
            .reply_error(call_id(id)?, &name, &message)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let service_emit = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (service, path, interface, member, value): (
            UserRef<DbusServiceToken>,
            String,
            String,
            String,
            LuaValue,
        ) = stack.consume(ctx)?;
        let value = lua_to_dbus(ctx, value, 0).map_err(HostError)?;
        service
            .service
            .borrow()
            .emit(&path, &interface, &member, &value)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let on_call_state = Rc::clone(&state);
    let service_on_call = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (service, callback): (UserRef<DbusServiceToken>, Closure) = stack.consume(ctx)?;
        let mut state = on_call_state.borrow_mut();
        if state.dbus_services.len() >= MAX_DBUS_SERVICES {
            return Err(HostError("D-Bus service limit reached".into()).into());
        }
        // One handler per service. Registering a second replaces the first
        // rather than fanning out, because two handlers answering one call
        // means one of them replies to a call the other already answered.
        let existing = state
            .dbus_services
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.service, &service.service));
        let entry = PendingDbusService {
            service: Rc::clone(&service.service),
            callback: ctx.stash(callback),
        };
        match existing {
            Some(index) => state.dbus_services[index] = entry,
            None => state.dbus_services.push(entry),
        }
        Ok(CallbackReturn::Return)
    });
    let close_state = Rc::clone(&state);
    let service_close = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let service: UserRef<DbusServiceToken> = stack.consume(ctx)?;
        // Dropping the registration is what releases the name: the entry here
        // and the token hold the only two references, and `DbusService::drop`
        // hands the name to whoever is queued behind us.
        close_state
            .borrow_mut()
            .dbus_services
            .retain(|entry| !Rc::ptr_eq(&entry.service, &service.service));
        Ok(CallbackReturn::Return)
    });

    let service_methods = Table::new(&ctx);
    service_methods.set_field(ctx, "name", service_name);
    service_methods.set_field(ctx, "on_call", service_on_call);
    service_methods.set_field(ctx, "reply", service_reply);
    service_methods.set_field(ctx, "reply_error", service_reply_error);
    service_methods.set_field(ctx, "emit", service_emit);
    service_methods.set_field(ctx, "close", service_close);
    let service_metatable = Table::new(&ctx);
    service_metatable.set_field(ctx, "__index", service_methods);
    let service_metatable = ctx.stash(service_metatable);

    let serve = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (bus, name, path, replace): (String, String, String, Option<bool>) =
            stack.consume(ctx)?;
        let bus = match bus.as_str() {
            "session" => Bus::Session,
            "system" => Bus::System,
            _ => return Err(HostError(format!("unknown D-Bus bus `{bus}`")).into()),
        };
        // Replacing by default. A shell that cannot be restarted without the
        // user first killing whatever holds its name is a shell nobody
        // restarts, and every one of these names is held by a shell.
        let (service, outcome) = DbusService::own(bus, &name, &path, replace.unwrap_or(true))
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            DbusServiceToken {
                service: Rc::new(RefCell::new(service)),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&service_metatable)));
        // Two values, and the second is the one that matters. Taking a name is
        // allowed to fail without being an error — somebody else runs the
        // notification server — and a configuration that ignores this reads as
        // working right up until nothing is ever sent to it.
        stack.replace(
            ctx,
            (
                userdata,
                match outcome {
                    NameOutcome::Owned => "owned",
                    NameOutcome::Taken => "taken",
                    NameOutcome::Queued => "queued",
                },
            ),
        );
        Ok(CallbackReturn::Return)
    });
    dbus.set_field(ctx, "serve", serve);
}

/// How many names one configuration may hold.
///
/// A shell owns a handful — notifications, a tray watcher, its own control
/// interface. A configuration asking for hundreds has a loop in it.
const MAX_DBUS_SERVICES: usize = 32;

/// Narrows a Lua integer to the call id the service handed out.
///
/// Ids are opaque and only ever come from a call table, so anything that is not
/// a positive integer is a configuration replying to something it invented.
fn call_id(id: i64) -> Result<u64, HostError> {
    u64::try_from(id).map_err(|_| HostError(format!("`{id}` is not a D-Bus call id")))
}
