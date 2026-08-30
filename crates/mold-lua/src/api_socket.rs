fn install_socket_api<'gc>(ctx: Context<'gc>, mold: Table<'gc>) {
    let socket_send = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (socket, bytes): (UserRef<SocketToken>, String) = stack.consume(ctx)?;
        if bytes.len() > 64 * 1024 {
            return Err(HostError("socket send exceeds 64 KiB".to_owned()).into());
        }
        if let Some(stream) = socket.state.borrow_mut().socket.as_mut() {
            stream
                .send(bytes.as_bytes())
                .map_err(|error| HostError(error.to_string()))?;
        }
        Ok(CallbackReturn::Return)
    });
    let socket_flush = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let socket: UserRef<SocketToken> = stack.consume(ctx)?;
        if let Some(stream) = socket.state.borrow_mut().socket.as_mut() {
            stream
                .flush()
                .map_err(|error| HostError(error.to_string()))?;
        }
        Ok(CallbackReturn::Return)
    });
    let socket_connected = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let socket: UserRef<SocketToken> = stack.consume(ctx)?;
        stack.replace(ctx, socket.state.borrow().socket.is_some());
        Ok(CallbackReturn::Return)
    });
    let socket_set_connected = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (socket, connected): (UserRef<SocketToken>, bool) = stack.consume(ctx)?;
        let mut state = socket.state.borrow_mut();
        if connected && state.socket.is_none() {
            if state.path.is_empty() {
                stack.replace(ctx, false);
                return Ok(CallbackReturn::Return);
            }
            state.socket =
                Some(Socket::connect(&state.path).map_err(|error| HostError(error.to_string()))?);
        } else if !connected && let Some(stream) = state.socket.take() {
            let _ = stream.shutdown();
        }
        stack.replace(ctx, state.socket.is_some());
        Ok(CallbackReturn::Return)
    });
    let socket_close = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let socket: UserRef<SocketToken> = stack.consume(ctx)?;
        if let Some(stream) = socket.state.borrow_mut().socket.take() {
            let _ = stream.shutdown();
        }
        Ok(CallbackReturn::Return)
    });
    let socket_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let socket: UserRef<SocketToken> = stack.consume(ctx)?;
        stack.replace(ctx, socket.state.borrow().path.as_str());
        Ok(CallbackReturn::Return)
    });
    let socket_set_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (socket, path): (UserRef<SocketToken>, String) = stack.consume(ctx)?;
        let mut state = socket.state.borrow_mut();
        if state.socket.is_some() {
            stack.replace(ctx, false);
        } else {
            state.path = path;
            stack.replace(ctx, true);
        }
        Ok(CallbackReturn::Return)
    });
    let socket_receive = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (socket, maximum, timeout_ms): (UserRef<SocketToken>, i64, i64) = stack.consume(ctx)?;
        let maximum = usize::try_from(maximum)
            .ok()
            .filter(|maximum| (1..=64 * 1024).contains(maximum))
            .ok_or_else(|| HostError("socket receive limit must be 1..65536".to_owned()))?;
        let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
        let mut bytes = vec![0; maximum];
        let mut state = socket.state.borrow_mut();
        let Some(stream) = state.socket.as_mut() else {
            stack.replace(ctx, LuaValue::Nil);
            return Ok(CallbackReturn::Return);
        };
        match stream.receive_timeout(&mut bytes, timeout) {
            Ok(read) => {
                bytes.truncate(read);
                stack.replace(ctx, String::from_utf8_lossy(&bytes).as_ref());
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                stack.replace(ctx, LuaValue::Nil);
            }
            Err(error) => return Err(HostError(error.to_string()).into()),
        }
        Ok(CallbackReturn::Return)
    });
    let socket_methods = Table::new(&ctx);
    socket_methods.set_field(ctx, "send", socket_send);
    socket_methods.set_field(ctx, "flush", socket_flush);
    socket_methods.set_field(ctx, "receive", socket_receive);
    socket_methods.set_field(ctx, "connected", socket_connected);
    socket_methods.set_field(ctx, "set_connected", socket_set_connected);
    socket_methods.set_field(ctx, "close", socket_close);
    socket_methods.set_field(ctx, "path", socket_path);
    socket_methods.set_field(ctx, "set_path", socket_set_path);
    let socket_metatable = Table::new(&ctx);
    socket_metatable.set_field(ctx, "__index", socket_methods);
    let socket_metatable = ctx.stash(socket_metatable);
    let accepted_socket_metatable = socket_metatable.clone();
    let server_accept = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let server: UserRef<SocketServerToken> = stack.consume(ctx)?;
        let state = server.state.borrow();
        let Some(listener) = state.server.as_ref() else {
            stack.replace(ctx, LuaValue::Nil);
            return Ok(CallbackReturn::Return);
        };
        let Some(socket) = listener
            .try_accept()
            .map_err(|error| HostError(error.to_string()))?
        else {
            stack.replace(ctx, LuaValue::Nil);
            return Ok(CallbackReturn::Return);
        };
        let userdata = UserData::new_static(
            &ctx,
            SocketToken {
                state: RefCell::new(SocketState {
                    path: String::new(),
                    socket: Some(socket),
                }),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&accepted_socket_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    let server_active = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let server: UserRef<SocketServerToken> = stack.consume(ctx)?;
        stack.replace(ctx, server.state.borrow().server.is_some());
        Ok(CallbackReturn::Return)
    });
    let server_set_active = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (server, active): (UserRef<SocketServerToken>, bool) = stack.consume(ctx)?;
        let mut state = server.state.borrow_mut();
        if active && state.server.is_none() {
            if state.path.is_empty() {
                stack.replace(ctx, false);
                return Ok(CallbackReturn::Return);
            }
            state.server = Some(
                SocketServer::bind(&state.path).map_err(|error| HostError(error.to_string()))?,
            );
        } else if !active {
            state.server = None;
        }
        stack.replace(ctx, state.server.is_some());
        Ok(CallbackReturn::Return)
    });
    let server_close = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let server: UserRef<SocketServerToken> = stack.consume(ctx)?;
        server.state.borrow_mut().server = None;
        Ok(CallbackReturn::Return)
    });
    let server_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let server: UserRef<SocketServerToken> = stack.consume(ctx)?;
        stack.replace(ctx, server.state.borrow().path.as_str());
        Ok(CallbackReturn::Return)
    });
    let server_set_path = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (server, path): (UserRef<SocketServerToken>, String) = stack.consume(ctx)?;
        let mut state = server.state.borrow_mut();
        if state.server.is_some() {
            stack.replace(ctx, false);
        } else {
            state.path = path;
            stack.replace(ctx, true);
        }
        Ok(CallbackReturn::Return)
    });
    let server_methods = Table::new(&ctx);
    server_methods.set_field(ctx, "accept", server_accept);
    server_methods.set_field(ctx, "active", server_active);
    server_methods.set_field(ctx, "set_active", server_set_active);
    server_methods.set_field(ctx, "close", server_close);
    server_methods.set_field(ctx, "path", server_path);
    server_methods.set_field(ctx, "set_path", server_set_path);
    let server_metatable = Table::new(&ctx);
    server_metatable.set_field(ctx, "__index", server_methods);
    let server_metatable = ctx.stash(server_metatable);
    let socket_server = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let path: String = stack.consume(ctx)?;
        let server = SocketServer::bind(&path).map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            SocketServerToken {
                state: RefCell::new(SocketServerState {
                    path,
                    server: Some(server),
                }),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&server_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "socket_server", socket_server);
    let socket = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let path: String = stack.consume(ctx)?;
        let socket = Socket::connect(&path).map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            SocketToken {
                state: RefCell::new(SocketState {
                    path,
                    socket: Some(socket),
                }),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&socket_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "socket", socket);

    let line_push = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (parser, chunk): (UserRef<LineParserToken>, String) = stack.consume(ctx)?;
        let values = parser.parser.borrow_mut().push(chunk.as_bytes());
        stack.replace(ctx, string_table(ctx, values));
        Ok(CallbackReturn::Return)
    });
    let line_finish = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let parser: UserRef<LineParserToken> = stack.consume(ctx)?;
        match parser.parser.borrow_mut().finish() {
            Some(value) => stack.replace(ctx, value),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let line_methods = Table::new(&ctx);
    line_methods.set_field(ctx, "push", line_push);
    line_methods.set_field(ctx, "finish", line_finish);
    let line_metatable = Table::new(&ctx);
    line_metatable.set_field(ctx, "__index", line_methods);
    let line_metatable = ctx.stash(line_metatable);
    let line_parser = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let userdata = UserData::new_static(
            &ctx,
            LineParserToken {
                parser: RefCell::new(LineParser::default()),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&line_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "line_parser", line_parser);

    let split_push = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (parser, chunk): (UserRef<SplitParserToken>, String) = stack.consume(ctx)?;
        let values = parser
            .parser
            .borrow_mut()
            .push(chunk.as_bytes())
            .into_iter()
            .map(|value| String::from_utf8_lossy(&value).into_owned());
        stack.replace(ctx, string_table(ctx, values));
        Ok(CallbackReturn::Return)
    });
    let split_finish = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let parser: UserRef<SplitParserToken> = stack.consume(ctx)?;
        match parser.parser.borrow_mut().finish() {
            Some(value) => stack.replace(ctx, String::from_utf8_lossy(&value).as_ref()),
            None => stack.replace(ctx, LuaValue::Nil),
        }
        Ok(CallbackReturn::Return)
    });
    let split_delimiter = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let parser: UserRef<SplitParserToken> = stack.consume(ctx)?;
        stack.replace(
            ctx,
            String::from_utf8_lossy(parser.parser.borrow().delimiter()).as_ref(),
        );
        Ok(CallbackReturn::Return)
    });
    let split_set_delimiter = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (parser, delimiter): (UserRef<SplitParserToken>, String) = stack.consume(ctx)?;
        let values = parser
            .parser
            .borrow_mut()
            .set_delimiter(delimiter.into_bytes())
            .into_iter()
            .map(|value| String::from_utf8_lossy(&value).into_owned());
        stack.replace(ctx, string_table(ctx, values));
        Ok(CallbackReturn::Return)
    });
    let split_methods = Table::new(&ctx);
    split_methods.set_field(ctx, "push", split_push);
    split_methods.set_field(ctx, "finish", split_finish);
    split_methods.set_field(ctx, "delimiter", split_delimiter);
    split_methods.set_field(ctx, "set_delimiter", split_set_delimiter);
    let split_metatable = Table::new(&ctx);
    split_metatable.set_field(ctx, "__index", split_methods);
    let split_metatable = ctx.stash(split_metatable);
    let split_parser = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let delimiter: String = stack.consume(ctx)?;
        let parser = SplitParser::new(delimiter.into_bytes())
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            SplitParserToken {
                parser: RefCell::new(parser),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&split_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "split_parser", split_parser);

    let collector_push = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (collector, chunk): (UserRef<StreamCollectorToken>, String) = stack.consume(ctx)?;
        let changed = collector
            .collector
            .borrow_mut()
            .push(chunk.as_bytes())
            .map_err(|error| HostError(error.to_string()))?;
        stack.replace(ctx, changed);
        Ok(CallbackReturn::Return)
    });
    let collector_finish = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        stack.replace(ctx, collector.collector.borrow_mut().finish());
        Ok(CallbackReturn::Return)
    });
    let collector_text = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        stack.replace(ctx, collector.collector.borrow().text());
        Ok(CallbackReturn::Return)
    });
    let collector_data = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        stack.replace(ctx, ctx.intern(collector.collector.borrow().data()));
        Ok(CallbackReturn::Return)
    });
    let collector_waiting = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        stack.replace(ctx, collector.collector.borrow().wait_for_end());
        Ok(CallbackReturn::Return)
    });
    let collector_set_waiting = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let (collector, waiting): (UserRef<StreamCollectorToken>, bool) = stack.consume(ctx)?;
        collector.collector.borrow_mut().set_wait_for_end(waiting);
        Ok(CallbackReturn::Return)
    });
    let collector_finished = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        stack.replace(ctx, collector.collector.borrow().finished());
        Ok(CallbackReturn::Return)
    });
    let collector_reset = Callback::from_fn(&ctx, |ctx, _, mut stack| {
        let collector: UserRef<StreamCollectorToken> = stack.consume(ctx)?;
        collector.collector.borrow_mut().reset();
        Ok(CallbackReturn::Return)
    });
    let collector_methods = Table::new(&ctx);
    collector_methods.set_field(ctx, "push", collector_push);
    collector_methods.set_field(ctx, "finish", collector_finish);
    collector_methods.set_field(ctx, "text", collector_text);
    collector_methods.set_field(ctx, "data", collector_data);
    collector_methods.set_field(ctx, "wait_for_end", collector_waiting);
    collector_methods.set_field(ctx, "set_wait_for_end", collector_set_waiting);
    collector_methods.set_field(ctx, "finished", collector_finished);
    collector_methods.set_field(ctx, "reset", collector_reset);
    let collector_metatable = Table::new(&ctx);
    collector_metatable.set_field(ctx, "__index", collector_methods);
    let collector_metatable = ctx.stash(collector_metatable);
    let stream_collector = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let maximum = match options.get_value(ctx, "maximum_bytes") {
            LuaValue::Nil => 1024 * 1024,
            LuaValue::Integer(value) => usize::try_from(value)
                .ok()
                .filter(|value| (1..=16 * 1024 * 1024).contains(value))
                .ok_or_else(|| {
                    HostError("stream collector maximum_bytes must be 1..16777216".into())
                })?,
            _ => {
                return Err(
                    HostError("stream collector maximum_bytes must be an integer".into()).into(),
                );
            }
        };
        let wait_for_end = table_bool(ctx, options, "wait_for_end", true).map_err(HostError)?;
        let collector = StreamCollector::new(maximum, wait_for_end)
            .map_err(|error| HostError(error.to_string()))?;
        let userdata = UserData::new_static(
            &ctx,
            StreamCollectorToken {
                collector: RefCell::new(collector),
            },
        );
        userdata.set_metatable(ctx, Some(ctx.fetch(&collector_metatable)));
        stack.replace(ctx, userdata);
        Ok(CallbackReturn::Return)
    });
    mold.set_field(ctx, "stream_collector", stream_collector);
}

