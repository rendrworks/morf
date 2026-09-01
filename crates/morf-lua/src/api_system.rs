use luna::{
    Callback, CallbackReturn, Closure, Context, Table, UserData, UserRef, Value as LuaValue,
};
use morf_io::{Bus, DbusProxy};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use morf_services::{
    GreetdClient, PamAuthenticator, PipeWire, StatusNotifierHost, UdevMonitor, XkbKeymap,
};

use crate::{lua_values::*, scene_bindings::*, serialization::*, state::*, table_menu::*};

pub(crate) fn install_system_service_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let dbus_get = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (proxy, property): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
        let value = proxy.proxy.get_value(&property).map_err(HostError)?;
        stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let dbus_call = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (proxy, method): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
        let value = proxy.proxy.call_value(&method).map_err(HostError)?;
        stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let dbus_call_with = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (proxy, method, argument): (UserRef<DbusToken>, String, LuaValue) =
            stack.consume(ctx)?;
        let argument = lua_to_dbus(ctx, argument, 0).map_err(HostError)?;
        let value = proxy
            .proxy
            .call_value_with(&method, &argument)
            .map_err(HostError)?;
        stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let dbus_set = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (proxy, property, value): (UserRef<DbusToken>, String, LuaValue) =
            stack.consume(ctx)?;
        let value = lua_to_dbus(ctx, value, 0).map_err(HostError)?;
        proxy
            .proxy
            .set_value(&property, &value)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let dbus_signal_state = Rc::clone(&state);
    let dbus_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (proxy, signal, callback): (UserRef<DbusToken>, String, Closure) =
            stack.consume(ctx)?;
        let signal = proxy
            .proxy
            .subscribe(signal)
            .map_err(|error| HostError(error.to_string()))?;
        dbus_signal_state
            .borrow_mut()
            .dbus_signals
            .push(PendingDbusSignal {
                signal,
                callback: ctx.stash(callback),
            });
        Ok(CallbackReturn::Return)
    });
    let dbus_introspect = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let proxy: UserRef<DbusToken> = stack.consume(ctx)?;
        let xml = proxy
            .proxy
            .introspect()
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, xml);
        Ok(CallbackReturn::Return)
    });
    let dbus_methods = Table::new(&ctx);
    dbus_methods.set_field(ctx, "get", dbus_get);
    dbus_methods.set_field(ctx, "call", dbus_call);
    dbus_methods.set_field(ctx, "call_with", dbus_call_with);
    dbus_methods.set_field(ctx, "set", dbus_set);
    dbus_methods.set_field(ctx, "subscribe", dbus_subscribe);
    dbus_methods.set_field(ctx, "introspect", dbus_introspect);
    let dbus_metatable = Table::new(&ctx);
    dbus_metatable.set_field(ctx, "__index", dbus_methods);
    let dbus_metatable = ctx.stash(dbus_metatable);
    let dbus_proxy = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (bus, destination, path, interface): (String, String, String, String) =
            stack.consume(ctx)?;
        let bus = match bus.as_str() {
            "session" => Bus::Session,
            "system" => Bus::System,
            _ => return Err(HostError(format!("unknown D-Bus bus `{bus}`")).into()),
        };
        let proxy = DbusProxy::connect(bus, destination, path, interface)
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(&ctx, DbusToken { proxy });
        userdata.set_metatable(ctx, Some(ctx.fetch(&dbus_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    let dbus = Table::new(&ctx);
    dbus.set_field(ctx, "proxy", dbus_proxy);
    morf.set_field(ctx, "dbus", dbus);

    let pipewire_nodes = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let pipewire: UserRef<PipeWireToken> = stack.consume(ctx)?;
        let nodes = Table::new(&ctx);
        for (index, node) in pipewire.service.nodes().into_iter().enumerate() {
            let value = Table::new(&ctx);
            value.set_field(ctx, "id", node.id as i64);
            value.set_field(
                ctx,
                "serial",
                node.serial
                    .and_then(|value| i64::try_from(value).ok())
                    .map_or(LuaValue::Nil, LuaValue::Integer),
            );
            value.set_field(ctx, "name", node.name.as_str());
            value.set_field(ctx, "description", node.description.as_str());
            value.set_field(ctx, "media_class", node.media_class.as_str());
            nodes
                .set(ctx, index as i64 + 1, value)
                .expect("PipeWire node table accepts integer keys");
        }
        stack.replace(ctx, nodes);
        Ok(CallbackReturn::Return)
    });
    let pipewire_volume = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (pipewire, id): (UserRef<PipeWireToken>, i64) = stack.consume(ctx)?;
        let id = u32::try_from(id).map_err(|_| HostError("invalid PipeWire node id".into()))?;
        let volume = pipewire
            .service
            .volume(id)
            .map_err(|error| HostError(error.to_string()))?;
        let value = Table::new(&ctx);
        let channels = Table::new(&ctx);
        for (index, channel) in volume.channels.iter().enumerate() {
            channels
                .set(ctx, index as i64 + 1, *channel as f64)
                .expect("PipeWire channel table accepts integer keys");
        }
        value.set_field(ctx, "channels", channels);
        value.set_field(ctx, "level", volume.average() as f64);
        value.set_field(ctx, "muted", volume.muted);
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let pipewire_set_volume = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (pipewire, id, level, muted): (UserRef<PipeWireToken>, i64, f64, bool) =
            stack.consume(ctx)?;
        let id = u32::try_from(id).map_err(|_| HostError("invalid PipeWire node id".into()))?;
        if !level.is_finite() || level < 0.0 || level > f32::MAX as f64 {
            return Err(HostError("PipeWire volume must be finite and non-negative".into()).into());
        }
        let current = pipewire
            .service
            .volume(id)
            .map_err(|error| HostError(error.to_string()))?;
        let channels = vec![level as f32; current.channels.len().max(1)];
        pipewire
            .service
            .set_volume(id, &channels, muted)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let pipewire_methods = Table::new(&ctx);
    pipewire_methods.set_field(ctx, "nodes", pipewire_nodes);
    pipewire_methods.set_field(ctx, "volume", pipewire_volume);
    pipewire_methods.set_field(ctx, "set_volume", pipewire_set_volume);
    let pipewire_metatable = Table::new(&ctx);
    pipewire_metatable.set_field(ctx, "__index", pipewire_methods);
    let pipewire_metatable = ctx.stash(pipewire_metatable);
    let pipewire_connect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let service = PipeWire::connect().map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(&ctx, PipeWireToken { service });
        userdata.set_metatable(ctx, Some(ctx.fetch(&pipewire_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    let pipewire = Table::new(&ctx);
    pipewire.set_field(ctx, "connect", pipewire_connect);
    morf.set_field(ctx, "pipewire", pipewire);

    let udev_state = Rc::clone(&state);
    let udev_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (subsystem, callback): (Option<String>, Closure) = stack.consume(ctx)?;
        let monitor = UdevMonitor::new(subsystem).map_err(|error| HostError(error.to_string()))?;
        udev_state.borrow_mut().udev_monitors.push(PendingUdev {
            monitor,
            callback: ctx.stash(callback),
        });
        Ok(CallbackReturn::Return)
    });
    let udev = Table::new(&ctx);
    udev.set_field(ctx, "subscribe", udev_subscribe);
    morf.set_field(ctx, "udev", udev);

    let status_notifier_state = Rc::clone(&state);
    let status_notifier_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        // `subscribe(handler)` uses the vendor-neutral watcher name;
        // `subscribe(handler, { "org.freedesktop", "..." })` names the ones this
        // session actually has. The engine ships no desktop environment's
        // prefix of its own — which watcher answers is a fact about the machine,
        // and the configuration is the thing that knows it.
        let (callback, watchers): (Closure, Option<Table>) = stack.consume(ctx)?;
        let names: Vec<String> = match watchers {
            Some(table) => (1..=table.length(&ctx))
                .filter_map(|index| match table.get_value(ctx, index) {
                    LuaValue::String(name) => Some(name.display_lossy().to_string()),
                    _ => None,
                })
                .collect(),
            None => vec![StatusNotifierHost::DEFAULT_NAMESPACE.to_owned()],
        };
        if names.is_empty() {
            return Err(HostError("status notifier needs at least one watcher name".into()).into());
        }
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let host = StatusNotifierHost::connect_to(&borrowed)
            .map_err(|error| HostError(error.to_string()))?;
        let mut state = status_notifier_state.borrow_mut();
        if state.status_notifiers.len() >= 4 {
            return Err(HostError("status notifier subscription limit reached".into()).into());
        }
        state.status_notifiers.push(PendingStatusNotifier {
            host,
            callback: ctx.stash(callback),
        });
        Ok(CallbackReturn::Return)
    });
    let status_notifier = Table::new(&ctx);
    status_notifier.set_field(ctx, "subscribe", status_notifier_subscribe);
    morf.set_field(ctx, "status_notifier", status_notifier);

    let greetd_create = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (greetd, username): (UserRef<GreetdToken>, String) = stack.consume(ctx)?;
        let response = greetd
            .client
            .borrow_mut()
            .create_session(&username)
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, greetd_response(ctx, response));
        Ok(CallbackReturn::Return)
    });
    let greetd_respond = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (greetd, response): (UserRef<GreetdToken>, Option<String>) = stack.consume(ctx)?;
        let response = greetd
            .client
            .borrow_mut()
            .respond(response.as_deref())
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, greetd_response(ctx, response));
        Ok(CallbackReturn::Return)
    });
    let greetd_start = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (greetd, command, environment): (UserRef<GreetdToken>, Table, Table) =
            stack.consume(ctx)?;
        let command = table_string_array(ctx, command, 64).map_err(HostError)?;
        let environment = table_string_array(ctx, environment, 256).map_err(HostError)?;
        let response = greetd
            .client
            .borrow_mut()
            .start_session(&command, &environment)
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, greetd_response(ctx, response));
        Ok(CallbackReturn::Return)
    });
    let greetd_cancel = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let greetd: UserRef<GreetdToken> = stack.consume(ctx)?;
        let response = greetd
            .client
            .borrow_mut()
            .cancel_session()
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, greetd_response(ctx, response));
        Ok(CallbackReturn::Return)
    });
    let greetd_methods = Table::new(&ctx);
    greetd_methods.set_field(ctx, "create_session", greetd_create);
    greetd_methods.set_field(ctx, "respond", greetd_respond);
    greetd_methods.set_field(ctx, "start_session", greetd_start);
    greetd_methods.set_field(ctx, "cancel_session", greetd_cancel);
    let greetd_metatable = Table::new(&ctx);
    greetd_metatable.set_field(ctx, "__index", greetd_methods);
    let greetd_metatable = ctx.stash(greetd_metatable);
    let greetd_connect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let path: Option<String> = stack.consume(ctx)?;
        let timeout = Duration::from_secs(2);
        let client = match path {
            Some(path) => GreetdClient::connect(path, timeout),
            None => GreetdClient::connect_environment(timeout),
        }
        .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            GreetdToken {
                client: RefCell::new(client),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&greetd_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    let greetd = Table::new(&ctx);
    greetd.set_field(ctx, "connect", greetd_connect);
    morf.set_field(ctx, "greetd", greetd);

    let pam_state = Rc::clone(&state);
    let pam_authenticate = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (service, username, password, callback): (String, String, String, Closure) =
            stack.consume(ctx)?;
        pam_state.borrow_mut().pam_tasks.push(PendingPam {
            task: PamAuthenticator::authenticate_async(service, username, password),
            callback: ctx.stash(callback),
            unlock_on_success: false,
        });
        Ok(CallbackReturn::Return)
    });
    let pam = Table::new(&ctx);
    pam.set_field(ctx, "authenticate", pam_authenticate);
    let pam_unlock_state = Rc::clone(&state);
    let pam_authenticate_unlock = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (service, username, password, callback): (String, String, String, Closure) =
            stack.consume(ctx)?;
        pam_unlock_state.borrow_mut().pam_tasks.push(PendingPam {
            task: PamAuthenticator::authenticate_async(service, username, password),
            callback: ctx.stash(callback),
            unlock_on_success: true,
        });
        Ok(CallbackReturn::Return)
    });
    pam.set_field(ctx, "authenticate_unlock", pam_authenticate_unlock);
    morf.set_field(ctx, "pam", pam);

    let xkb_compile = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let rules = table_string(ctx, options, "rules", "").map_err(HostError)?;
        let model = table_string(ctx, options, "model", "pc105").map_err(HostError)?;
        let layout = table_string(ctx, options, "layout", "us").map_err(HostError)?;
        let variant = table_string(ctx, options, "variant", "").map_err(HostError)?;
        let xkb_options = match options.get_value(ctx, "options") {
            LuaValue::Nil => None,
            LuaValue::String(value) => Some(value.display_lossy().to_string()),
            _ => return Err(HostError("XKB options must be a string".into()).into()),
        };
        let keymap = XkbKeymap::compile(&rules, &model, &layout, &variant, xkb_options.as_deref())
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, xkb_keymap_to_lua(ctx, &keymap));
        Ok(CallbackReturn::Return)
    });
    let xkb = Table::new(&ctx);
    xkb.set_field(ctx, "compile", xkb_compile);
    morf.set_field(ctx, "xkb", xkb);
}
