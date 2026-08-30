fn install_process_api<'gc>(ctx: Context<'gc>, mold: Table<'gc>) {
    let process_write = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, bytes): (UserRef<ProcessToken>, String) = stack.consume(ctx)?;
        process
            .process
            .borrow_mut()
            .write(bytes.as_bytes())
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let process_close = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessToken> = stack.consume(ctx)?;
        process.process.borrow_mut().close_stdin();
        Ok(CallbackReturn::Return)
    });
    let process_kill = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessToken> = stack.consume(ctx)?;
        process
            .process
            .borrow_mut()
            .kill()
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let process_next = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessToken> = stack.consume(ctx)?;
        let event = process
            .process
            .borrow_mut()
            .next_event(Duration::ZERO)
            .map_err(|error| HostError(error.to_string()))?;
        let Some(event) = event else {
            stack.replace(ctx, LuaValue::Nil);
            return Ok(CallbackReturn::Return);
        };
        let value = Table::new(&ctx);
        match event {
            ProcessEvent::Stdout(bytes) => {
                value.set_field(ctx, "kind", "stdout");
                value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
            }
            ProcessEvent::Stderr(bytes) => {
                value.set_field(ctx, "kind", "stderr");
                value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
            }
            ProcessEvent::Exit(status) => {
                value.set_field(ctx, "kind", "exit");
                value.set_field(ctx, "success", status.success());
                value.set_field(
                    ctx,
                    "code",
                    status
                        .code()
                        .map_or(LuaValue::Nil, |code| LuaValue::Integer(code as i64)),
                );
            }
        }
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let process_methods = Table::new(&ctx);
    process_methods.set_field(ctx, "write", process_write);
    process_methods.set_field(ctx, "close_stdin", process_close);
    process_methods.set_field(ctx, "kill", process_kill);
    process_methods.set_field(ctx, "next", process_next);
    let process_metatable = Table::new(&ctx);
    process_metatable.set_field(ctx, "__index", process_methods);
    let process_metatable = ctx.stash(process_metatable);
    let process = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (program, args): (String, Table) = stack.consume(ctx)?;
        let args = table_string_array(ctx, args, 64).map_err(HostError)?;
        let process =
            Process::spawn(program, args).map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            ProcessToken {
                process: RefCell::new(process),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&process_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "process", process);

    let process_view_start = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        let mut state = process.state.borrow_mut();
        if state.process.is_some() {
            stack.replace(ctx, false);
            return Ok(CallbackReturn::Return);
        }
        let child =
            Process::spawn_config(&state.config).map_err(|error| HostError(error.to_string()))?;
        state.process = Some(child);
        stack.replace(ctx, true);
        Ok(CallbackReturn::Return)
    });
    let process_view_running = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        stack.replace(ctx, process.state.borrow().process.is_some());
        Ok(CallbackReturn::Return)
    });
    let process_view_id = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        match process.state.borrow().process.as_ref() {
            Some(process) => stack.replace(ctx, i64::from(process.id())),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let process_view_write = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, bytes): (UserRef<ProcessViewToken>, String) = stack.consume(ctx)?;
        let mut state = process.state.borrow_mut();
        let process = state
            .process
            .as_mut()
            .ok_or_else(|| HostError("process is not running".into()))?;
        process
            .write(bytes.as_bytes())
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let process_view_close = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        if let Some(process) = process.state.borrow_mut().process.as_mut() {
            process.close_stdin();
        }
        Ok(CallbackReturn::Return)
    });
    let process_view_kill = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        let mut state = process.state.borrow_mut();
        let process = state
            .process
            .as_mut()
            .ok_or_else(|| HostError("process is not running".into()))?;
        process
            .kill()
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let process_view_signal = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, signal): (UserRef<ProcessViewToken>, i64) = stack.consume(ctx)?;
        let signal = i32::try_from(signal).map_err(|_| HostError("signal must be 1..64".into()))?;
        let state = process.state.borrow();
        let process = state
            .process
            .as_ref()
            .ok_or_else(|| HostError("process is not running".into()))?;
        process
            .signal(signal)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    let process_view_command = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        let command = string_table(ctx, process.state.borrow().config.command.clone());
        stack.replace(ctx, command);
        Ok(CallbackReturn::Return)
    });
    let process_view_set_command = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, command): (UserRef<ProcessViewToken>, Table) = stack.consume(ctx)?;
        let command = table_string_array(ctx, command, 64).map_err(HostError)?;
        if command.is_empty() {
            return Err(HostError("process_view command cannot be empty".into()).into());
        }
        update_process_view_config(&process, |config| config.command = command)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let process_view_environment = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        let values = Table::new(&ctx);
        let environment = process.state.borrow().config.environment.clone();
        for (name, value) in environment {
            values.set(
                ctx,
                ctx.intern(name.as_bytes()),
                ctx.intern(value.as_bytes()),
            )?;
        }
        stack.replace(ctx, values);
        Ok(CallbackReturn::Return)
    });
    let process_view_set_environment = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, environment): (UserRef<ProcessViewToken>, Table) = stack.consume(ctx)?;
        let environment = table_string_map(ctx, environment, 256).map_err(HostError)?;
        update_process_view_config(&process, |config| config.environment = environment)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let process_view_working_directory = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        match &process.state.borrow().config.working_directory {
            Some(directory) => stack.replace(ctx, directory.to_string_lossy().as_ref()),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let process_view_set_working_directory = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, directory): (UserRef<ProcessViewToken>, LuaValue) = stack.consume(ctx)?;
        let directory = match directory {
            LuaValue::Nil => None,
            LuaValue::String(directory) => {
                let directory = directory.display_lossy().to_string();
                if directory.is_empty()
                    || directory.len() > 4_096
                    || directory.as_bytes().contains(&0)
                {
                    return Err(HostError("working directory is invalid".into()).into());
                }
                Some(PathBuf::from(directory))
            }
            _ => {
                return Err(HostError("working directory must be a string or nil".into()).into());
            }
        };
        update_process_view_config(&process, |config| config.working_directory = directory)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let process_view_clear_environment = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let process: UserRef<ProcessViewToken> = stack.consume(ctx)?;
        stack.replace(ctx, process.state.borrow().config.clear_environment);
        Ok(CallbackReturn::Return)
    });
    let process_view_set_clear_environment = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, clear): (UserRef<ProcessViewToken>, bool) = stack.consume(ctx)?;
        update_process_view_config(&process, |config| config.clear_environment = clear)
            .map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    let process_view_next = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (process, timeout_ms): (UserRef<ProcessViewToken>, i64) = stack.consume(ctx)?;
        let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
        let mut state = process.state.borrow_mut();
        let child = state
            .process
            .as_mut()
            .ok_or_else(|| HostError("process is not running".into()))?;
        let event = child
            .next_event(timeout)
            .map_err(|error| HostError(error.to_string()))?;
        let Some(event) = event else {
            stack.replace(ctx, LuaValue::Nil);
            return Ok(CallbackReturn::Return);
        };
        let value = Table::new(&ctx);
        match event {
            ProcessEvent::Stdout(bytes) => {
                value.set_field(ctx, "kind", "stdout");
                value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
            }
            ProcessEvent::Stderr(bytes) => {
                value.set_field(ctx, "kind", "stderr");
                value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
            }
            ProcessEvent::Exit(status) => {
                value.set_field(ctx, "kind", "exit");
                value.set_field(ctx, "success", status.success());
                value.set_field(
                    ctx,
                    "code",
                    status
                        .code()
                        .map_or(LuaValue::Nil, |code| LuaValue::Integer(code as i64)),
                );
                state.process = None;
            }
        }
        stack.replace(ctx, value);
        Ok(CallbackReturn::Return)
    });
    let process_view_methods = Table::new(&ctx);
    process_view_methods.set_field(ctx, "start", process_view_start);
    process_view_methods.set_field(ctx, "running", process_view_running);
    process_view_methods.set_field(ctx, "process_id", process_view_id);
    process_view_methods.set_field(ctx, "write", process_view_write);
    process_view_methods.set_field(ctx, "close_stdin", process_view_close);
    process_view_methods.set_field(ctx, "kill", process_view_kill);
    process_view_methods.set_field(ctx, "signal", process_view_signal);
    process_view_methods.set_field(ctx, "command", process_view_command);
    process_view_methods.set_field(ctx, "set_command", process_view_set_command);
    process_view_methods.set_field(ctx, "environment", process_view_environment);
    process_view_methods.set_field(ctx, "set_environment", process_view_set_environment);
    process_view_methods.set_field(ctx, "working_directory", process_view_working_directory);
    process_view_methods.set_field(
        ctx,
        "set_working_directory",
        process_view_set_working_directory,
    );
    process_view_methods.set_field(ctx, "clear_environment", process_view_clear_environment);
    process_view_methods.set_field(
        ctx,
        "set_clear_environment",
        process_view_set_clear_environment,
    );
    process_view_methods.set_field(ctx, "next", process_view_next);
    let process_view_metatable = Table::new(&ctx);
    process_view_metatable.set_field(ctx, "__index", process_view_methods);
    let process_view_metatable = ctx.stash(process_view_metatable);
    let process_view = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let command = match options.get_value(ctx, "command") {
            LuaValue::Table(command) => table_string_array(ctx, command, 64).map_err(HostError)?,
            _ => return Err(HostError("process_view command must be a table".into()).into()),
        };
        if command.is_empty() {
            return Err(HostError("process_view command cannot be empty".into()).into());
        }
        let environment = match options.get_value(ctx, "environment") {
            LuaValue::Nil => BTreeMap::new(),
            LuaValue::Table(environment) => {
                table_string_map(ctx, environment, 256).map_err(HostError)?
            }
            _ => {
                return Err(HostError("process_view environment must be a table".into()).into());
            }
        };
        let clear_environment = match options.get_value(ctx, "clear_environment") {
            LuaValue::Nil => false,
            LuaValue::Boolean(value) => value,
            _ => {
                return Err(
                    HostError("process_view clear_environment must be boolean".into()).into(),
                );
            }
        };
        let working_directory = match options.get_value(ctx, "working_directory") {
            LuaValue::Nil => None,
            LuaValue::String(value) => Some(PathBuf::from(value.display_lossy().to_string())),
            _ => {
                return Err(
                    HostError("process_view working_directory must be a string".into()).into(),
                );
            }
        };
        let running = match options.get_value(ctx, "running") {
            LuaValue::Nil => false,
            LuaValue::Boolean(value) => value,
            _ => {
                return Err(HostError("process_view running must be boolean".into()).into());
            }
        };
        let config = ProcessConfig {
            command,
            environment,
            clear_environment,
            working_directory,
        };
        let process = if running {
            Some(Process::spawn_config(&config).map_err(|error| HostError(error.to_string()))?)
        } else {
            None
        };
        let userdata = UserData::new_static(
            &ctx,
            ProcessViewToken {
                state: RefCell::new(ProcessViewState { config, process }),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&process_view_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "process_view", process_view);
}

