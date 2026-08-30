struct ApiModules<'gc> {
    mold: Table<'gc>,
    core: Table<'gc>,
    ui: Table<'gc>,
    io: Table<'gc>,
    json: Table<'gc>,
    window: Table<'gc>,
}

fn finish_reactive_api<'gc>(
    ctx: Context<'gc>,
    modules: ApiModules<'gc>,
    module_roots: Rc<RefCell<Vec<PathBuf>>>,
    limits: Limits,
) {
    let ApiModules {
        mold,
        core,
        ui,
        io,
        json,
        window,
    } = modules;
    ctx.set_global("mold", mold);

    let loaded = Table::new(&ctx);
    loaded.set_field(ctx, "mold", mold);
    loaded.set_field(ctx, "mold.core", core);
    loaded.set_field(ctx, "mold.ui", ui);
    loaded.set_field(ctx, "mold.io", io);
    loaded.set_field(ctx, "mold.io.json", json);
    loaded.set_field(ctx, "mold.window", window);
    let package = Table::new(&ctx);
    package.set_field(ctx, "loaded", loaded);
    ctx.set_global("package", package);

    let mold = ctx.stash(mold);
    let ui = ctx.stash(ui);
    let loaded = ctx.stash(loaded);
    ctx.set_global(
        "require",
        Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let name: String = stack.consume(ctx)?;
            match name.as_str() {
                "mold" => stack.replace(ctx, ctx.fetch(&mold)),
                "mold.ui" => stack.replace(ctx, ctx.fetch(&ui)),
                _ => {
                    let loaded = ctx.fetch(&loaded);
                    let key = ctx.intern(name.as_bytes());
                    let cached = loaded.get_value(ctx, key);
                    if !matches!(cached, LuaValue::Nil) {
                        stack.replace(ctx, cached);
                        return Ok(CallbackReturn::Return);
                    }
                    loaded.set(ctx, key, true)?;
                    let source = match load_runtime_module(&module_roots.borrow(), &name) {
                        Ok(source) => source,
                        Err(error) => {
                            loaded.set(ctx, key, LuaValue::Nil)?;
                            return Err(HostError(error).into());
                        }
                    };
                    let module = match execute_module(ctx, &name, &source, limits) {
                        Ok(LuaValue::Nil) => LuaValue::Boolean(true),
                        Ok(module) => module,
                        Err(error) => {
                            loaded.set(ctx, key, LuaValue::Nil)?;
                            return Err(HostError(error).into());
                        }
                    };
                    loaded.set(ctx, key, module)?;
                    stack.replace(ctx, module);
                }
            }
            Ok(CallbackReturn::Return)
        }),
    );
}
fn install_reactive_api(
    lua: &mut Lua,
    state: Rc<RefCell<ReactiveState>>,
    module_roots: Rc<RefCell<Vec<PathBuf>>>,
    limits: Limits,
    screen: Option<&Screen>,
) {
    lua.enter(|ctx| {
        let mold = Table::new(&ctx);
        install_signal_api(ctx, Rc::clone(&state), mold, limits);
        install_retention_api(ctx, Rc::clone(&state), mold, limits);
        install_shell_api(ctx, Rc::clone(&state), mold);
        install_time_api(ctx, Rc::clone(&state), mold);
        install_image_api(ctx, mold);
        install_transform_api(ctx, Rc::clone(&state), mold);
        install_animation_api(ctx, Rc::clone(&state), mold);
        install_easing_api(ctx, mold);
        install_group_api(ctx, Rc::clone(&state), mold);
        install_host_service_api(ctx, Rc::clone(&state), mold, screen);
        install_view_api(ctx, Rc::clone(&state), mold, limits);
        install_process_api(ctx, mold);
        install_file_api(ctx, mold);
        install_socket_api(ctx, mold);
        install_system_service_api(ctx, Rc::clone(&state), mold);
        let (ui, json) = install_ui_json_api(ctx, Rc::clone(&state), mold, limits);
        install_menu_desktop_api(ctx, mold, limits);
        let (core, io, window) = install_module_api(ctx, state, mold);
        finish_reactive_api(
            ctx,
            ApiModules {
                mold,
                core,
                ui,
                io,
                json,
                window,
            },
            module_roots,
            limits,
        );
    });
}
