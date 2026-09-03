use luna::{Callback, CallbackReturn, Closure, Context, Table, Value as LuaValue};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    layer_parse::*, scene_bindings::*, state::*, table_menu::*, types::*, window_parse::*,
};

pub(crate) fn install_shell_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) {
    let surface_read_state = Rc::clone(&state);
    let surface_index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (_surface, key): (Table, String) = stack.consume(ctx)?;
        let config = &surface_read_state.borrow().layer_surface;
        let value = match key.as_str() {
            "namespace" => LuaValue::String(ctx.intern(config.namespace.as_bytes())),
            "width" => LuaValue::Integer(i64::from(config.width)),
            "height" => LuaValue::Integer(i64::from(config.height)),
            "exclusive_zone" => LuaValue::Integer(i64::from(config.exclusive_zone)),
            "margin_top" => LuaValue::Integer(i64::from(config.margin_top)),
            "margin_right" => LuaValue::Integer(i64::from(config.margin_right)),
            "margin_bottom" => LuaValue::Integer(i64::from(config.margin_bottom)),
            "margin_left" => LuaValue::Integer(i64::from(config.margin_left)),
            "layer" => LuaValue::String(ctx.intern(config.layer.as_bytes())),
            "keyboard_focus" => LuaValue::String(ctx.intern(config.keyboard_focus.as_bytes())),
            "opaque" => LuaValue::Boolean(config.opaque),
            "anchors" => {
                let anchors = Table::new(&ctx);
                anchors.set_field(ctx, "top", config.anchors.top);
                anchors.set_field(ctx, "right", config.anchors.right);
                anchors.set_field(ctx, "bottom", config.anchors.bottom);
                anchors.set_field(ctx, "left", config.anchors.left);
                LuaValue::Table(anchors)
            }
            "mask" => config
                .input_regions
                .as_ref()
                .map_or(LuaValue::Nil, |regions| {
                    let values = Table::new(&ctx);
                    for (index, region) in regions.iter().enumerate() {
                        values
                            .set(ctx, index as i64 + 1, region_to_lua(ctx, region))
                            .expect("region list accepts integer keys");
                    }
                    LuaValue::Table(values)
                }),
            "reserve" => LuaValue::Table(reserve_to_lua(ctx, config.reserve)),
            _ => LuaValue::Nil,
        };
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let surface_write_state = Rc::clone(&state);
    let surface_new_index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (_surface, key, value): (Table, String, LuaValue) = stack.consume(ctx)?;
        let mut state = surface_write_state.borrow_mut();
        let changed =
            apply_layer_setting(ctx, &mut state.layer_surface, &key, value).map_err(HostError)?;
        state.layer_surface_changed |= changed;
        Ok(CallbackReturn::Return)
    });
    let surface_metatable = Table::new(&ctx);
    surface_metatable.set_field(ctx, "__index", surface_index);
    surface_metatable.set_field(ctx, "__newindex", surface_new_index);
    let surface = Table::new(&ctx);
    surface.set_metatable(ctx, Some(surface_metatable));
    morf.set_field(ctx, "surface", surface);
    let env = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let name: String = stack.consume(ctx)?;
        if name.is_empty() || name.len() > 256 || name.as_bytes().contains(&0) {
            return Err(HostError("environment variable name is invalid".into()).into());
        }
        match std::env::var_os(name) {
            Some(value) => stack.replace(ctx, value.to_string_lossy().as_ref()),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "env", env);
    // What faces this machine has, so a configuration that offers a choice of
    // font can offer the real ones rather than a list of names guessed by
    // whoever wrote it. A call rather than a table: working the answer out
    // means scanning the font directories, and most configurations never ask.
    let font_families = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let names = morf_text::installed_families();
        let table = Table::new(&ctx);
        for (index, name) in names.iter().enumerate() {
            table.set(ctx, index as i64 + 1, name.as_str())?;
        }
        stack.replace(ctx, table);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "font_families", font_families);
    // What this configuration was started with. Three views of the same words:
    // `args` is what was typed, in order and unaltered; `options` is the flags
    // resolved into names and values; `operands` is what was left over. A
    // configuration that wants to read the line itself has the first, and one
    // that wants an answer has the other two.
    let given = crate::arguments::given();
    let list_of = |items: &[String]| {
        let table = Table::new(&ctx);
        for (index, word) in items.iter().enumerate() {
            table
                .set(ctx, index as i64 + 1, word.as_str())
                .expect("a table accepts integer keys");
        }
        table
    };
    let words = list_of(given.words());
    morf.set_field(ctx, "args", words);
    let options = Table::new(&ctx);
    for (name, values) in given.options() {
        // One value is that value; several are a list, because a repeated
        // option keeps what it was given and only the configuration knows
        // whether the first, the last or all of them was meant.
        if let [only] = values.as_slice() {
            match only.text() {
                Some(text) => options.set_field(ctx, name.as_str(), text),
                None => options.set_field(ctx, name.as_str(), true),
            };
            continue;
        }
        let list = Table::new(&ctx);
        for (index, value) in values.iter().enumerate() {
            let slot = index as i64 + 1;
            match value.text() {
                Some(text) => list.set(ctx, slot, text),
                None => list.set(ctx, slot, true),
            }
            .expect("a table accepts integer keys");
        }
        options.set_field(ctx, name.as_str(), list);
    }
    morf.set_field(ctx, "options", options);
    morf.set_field(ctx, "operands", list_of(given.operands()));
    morf.set_field(ctx, "process_id", i64::from(std::process::id()));
    // The binary that is running, so a configuration can start another of
    // itself. `"morf"` only works when morf is on `PATH`, which it is not when
    // it is being run out of a build directory — and a greeter that cannot open
    // its on-screen keyboard because of that is a machine nobody can log into.
    if let Ok(executable) = std::env::current_exe() {
        morf.set_field(ctx, "executable", executable.to_string_lossy().as_ref());
    }
    morf.set_field(ctx, "version", env!("CARGO_PKG_VERSION"));
    let launched = launch_time_ms();
    morf.set_field(
        ctx,
        "launch_time_ms",
        i64::try_from(launched).unwrap_or(i64::MAX),
    );
    morf.set_field(
        ctx,
        "instance_id",
        format!("{}-{launched}", std::process::id()),
    );
    morf.set_field(
        ctx,
        "app_id",
        std::env::var("MORF_APP_ID").unwrap_or_else(|_| "morf".to_owned()),
    );
    let shell_id_state = Rc::clone(&state);
    let shell_id = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let id = shell_storage_key(&shell_id_state.borrow().shell_root);
        stack.replace(ctx, id);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shell_id", shell_id);
    let shell_dir_state = Rc::clone(&state);
    let shell_dir = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let root = shell_dir_state.borrow().shell_root.clone();
        stack.replace(ctx, root.to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shell_dir", shell_dir);
    let shell_root_state = Rc::clone(&state);
    let shell_path = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let relative: String = stack.consume(ctx)?;
        let path =
            rooted_path(&shell_root_state.borrow().shell_root, &relative).map_err(HostError)?;
        stack.replace(ctx, path.to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shell_path", shell_path);
    morf.set_field(ctx, "config_path", morf.get_value(ctx, "shell_path"));
    for (directory_name, path_name, kind) in [
        ("data_dir", "data_path", StorageKind::Data),
        ("state_dir", "state_path", StorageKind::State),
        ("cache_dir", "cache_path", StorageKind::Cache),
    ] {
        let directory_state = Rc::clone(&state);
        let directory = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let root =
                shell_storage_dir(&directory_state.borrow().shell_root, kind).map_err(HostError)?;
            stack.replace(ctx, root.to_string_lossy().as_ref());
            Ok(CallbackReturn::Return)
        });
        morf.set(ctx, directory_name, directory)
            .expect("core path directory accepts a native callback");
        let path_state = Rc::clone(&state);
        let path = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let relative: String = stack.consume(ctx)?;
            let root =
                shell_storage_dir(&path_state.borrow().shell_root, kind).map_err(HostError)?;
            let path = rooted_path(&root, &relative).map_err(HostError)?;
            stack.replace(ctx, path.to_string_lossy().as_ref());
            Ok(CallbackReturn::Return)
        });
        morf.set(ctx, path_name, path)
            .expect("core path resolver accepts a native callback");
    }
    let has_version = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (major, minor, features): (i64, i64, Option<Table>) = stack.consume(ctx)?;
        let current = env!("CARGO_PKG_VERSION")
            .split('.')
            .take(2)
            .map(|part| part.parse::<i64>().unwrap_or(0))
            .collect::<Vec<_>>();
        let available = current.first().copied().unwrap_or(0) > major
            || current.first().copied().unwrap_or(0) == major
                && current.get(1).copied().unwrap_or(0) >= minor;
        let features_available = features.is_none_or(|features| {
            table_string_array(ctx, features, 64).is_ok_and(|features| {
                features.iter().all(|feature| {
                    matches!(
                        feature.as_str(),
                        "wayland"
                            | "vulkan"
                            | "gles"
                            | "lua"
                            | "ipc"
                            | "session-lock"
                            | "screencopy"
                            | "virtual-keyboard"
                            | "input-method"
                            | "text-input"
                    )
                })
            })
        });
        stack.replace(ctx, available && features_available);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "has_version", has_version);
    let reload_state = Rc::clone(&state);
    let reload = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let hard: Option<bool> = stack.consume(ctx)?;
        let mut state = reload_state.borrow_mut();
        state.reload_request = Some(state.reload_request.unwrap_or(false) || hard.unwrap_or(false));
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "reload", reload);
    let completed_state = Rc::clone(&state);
    let on_reload_completed = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        completed_state
            .borrow_mut()
            .reload_completed_callbacks
            .push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "on_reload_completed", on_reload_completed);
    let failed_state = Rc::clone(&state);
    let on_reload_failed = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        failed_state
            .borrow_mut()
            .reload_failed_callbacks
            .push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "on_reload_failed", on_reload_failed);
    let watch_state = Rc::clone(&state);
    let watch_files = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let value: Option<bool> = stack.consume(ctx)?;
        let mut state = watch_state.borrow_mut();
        if let Some(value) = value
            && state.watch_files != value
        {
            state.watch_files = value;
            state.watch_files_changed = true;
        }
        stack.replace(ctx, state.watch_files);
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "watch_files", watch_files);
    let quit_state = Rc::clone(&state);
    // Asking to stop, rather than stopping. The call returns and the rest of
    // the handler runs; the shell goes down at the top of the next frame, once
    // the supervisor has seen the request and taken every output down with it.
    // Exiting from inside a Lua callback would unwind the runtime that is
    // running the callback.
    let quit = Callback::from_fn(&ctx, move |_, _, _| {
        quit_state.borrow_mut().quit_requested = true;
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "quit", quit);
    let working_directory = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let path: Option<String> = stack.consume(ctx)?;
        if let Some(path) = path {
            if path.is_empty() || path.len() > 4_096 || path.as_bytes().contains(&0) {
                return Err(HostError("working directory path is invalid".into()).into());
            }
            std::env::set_current_dir(&path).map_err(|error| HostError(error.to_string()))?;
        }
        let current = std::env::current_dir().map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, current.to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "working_directory", working_directory);
}
