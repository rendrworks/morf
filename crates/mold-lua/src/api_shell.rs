fn install_shell_api<'gc>(ctx: Context<'gc>, state: Rc<RefCell<ReactiveState>>, mold: Table<'gc>) {
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
    mold.set_field(ctx, "surface", surface);
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
    mold.set_field(ctx, "env", env);
    mold.set_field(ctx, "process_id", i64::from(std::process::id()));
    mold.set_field(ctx, "version", env!("CARGO_PKG_VERSION"));
    let launched = launch_time_ms();
    mold.set_field(
        ctx,
        "launch_time_ms",
        i64::try_from(launched).unwrap_or(i64::MAX),
    );
    mold.set_field(
        ctx,
        "instance_id",
        format!("{}-{launched}", std::process::id()),
    );
    mold.set_field(
        ctx,
        "app_id",
        std::env::var("MOLD_APP_ID").unwrap_or_else(|_| "mold".to_owned()),
    );
    let shell_id_state = Rc::clone(&state);
    let shell_id = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let id = shell_storage_key(&shell_id_state.borrow().shell_root);
        stack.replace(ctx, id);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "shell_id", shell_id);
    let shell_dir_state = Rc::clone(&state);
    let shell_dir = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let root = shell_dir_state.borrow().shell_root.clone();
        stack.replace(ctx, root.to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "shell_dir", shell_dir);
    let shell_root_state = Rc::clone(&state);
    let shell_path = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let relative: String = stack.consume(ctx)?;
        let path =
            rooted_path(&shell_root_state.borrow().shell_root, &relative).map_err(HostError)?;
        stack.replace(ctx, path.to_string_lossy().as_ref());
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "shell_path", shell_path);
    mold.set_field(ctx, "config_path", mold.get_value(ctx, "shell_path"));
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
        mold.set(ctx, directory_name, directory)
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
        mold.set(ctx, path_name, path)
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
    mold.set_field(ctx, "has_version", has_version);
    let reload_state = Rc::clone(&state);
    let reload = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let hard: Option<bool> = stack.consume(ctx)?;
        let mut state = reload_state.borrow_mut();
        state.reload_request = Some(state.reload_request.unwrap_or(false) || hard.unwrap_or(false));
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "reload", reload);
    let completed_state = Rc::clone(&state);
    let on_reload_completed = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        completed_state
            .borrow_mut()
            .reload_completed_callbacks
            .push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "on_reload_completed", on_reload_completed);
    let failed_state = Rc::clone(&state);
    let on_reload_failed = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let callback: Closure = stack.consume(ctx)?;
        failed_state
            .borrow_mut()
            .reload_failed_callbacks
            .push(ctx.stash(callback));
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "on_reload_failed", on_reload_failed);
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
    mold.set_field(ctx, "watch_files", watch_files);
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
    mold.set_field(ctx, "working_directory", working_directory);
}
