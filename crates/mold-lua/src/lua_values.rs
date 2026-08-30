fn string_table<'gc>(ctx: Context<'gc>, values: impl IntoIterator<Item = String>) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (index, value) in values.into_iter().enumerate() {
        table
            .set(ctx, index as i64 + 1, value)
            .expect("string table accepts integer keys");
    }
    table
}

fn desktop_entry_table<'gc>(ctx: Context<'gc>, entry: &DesktopEntry) -> Table<'gc> {
    let value = Table::new(&ctx);
    value.set_field(ctx, "id", entry.id.as_str());
    value.set_field(ctx, "name", entry.name.as_str());
    value.set_field(ctx, "generic_name", entry.generic_name.as_str());
    value.set_field(ctx, "startup_class", entry.startup_class.as_str());
    value.set_field(ctx, "no_display", entry.no_display);
    value.set_field(ctx, "comment", entry.comment.as_str());
    value.set_field(ctx, "icon", entry.icon.as_str());
    value.set_field(ctx, "exec", entry.exec.as_str());
    value.set_field(ctx, "command", string_table(ctx, entry.command.clone()));
    value.set_field(ctx, "working_directory", entry.working_directory.as_str());
    value.set_field(ctx, "run_in_terminal", entry.run_in_terminal);
    value.set_field(
        ctx,
        "categories",
        string_table(ctx, entry.categories.clone()),
    );
    value.set_field(ctx, "keywords", string_table(ctx, entry.keywords.clone()));
    let actions = Table::new(&ctx);
    for (index, action) in entry.actions.iter().enumerate() {
        let item = Table::new(&ctx);
        item.set_field(ctx, "id", action.id.as_str());
        item.set_field(ctx, "name", action.name.as_str());
        item.set_field(ctx, "icon", action.icon.as_str());
        item.set_field(ctx, "exec", action.exec.as_str());
        item.set_field(ctx, "command", string_table(ctx, action.command.clone()));
        actions
            .set(ctx, index as i64 + 1, item)
            .expect("desktop action table accepts integer keys");
    }
    value.set_field(ctx, "actions", actions);
    value
}

fn greetd_response<'gc>(ctx: Context<'gc>, response: GreetdResponse) -> Table<'gc> {
    let value = Table::new(&ctx);
    match response {
        GreetdResponse::Success => {
            value.set_field(ctx, "type", "success");
        }
        GreetdResponse::AuthMessage { kind, message } => {
            value.set_field(ctx, "type", "auth_message");
            value.set_field(
                ctx,
                "auth_message_type",
                match kind {
                    AuthMessageType::Visible => "visible",
                    AuthMessageType::Secret => "secret",
                    AuthMessageType::Info => "info",
                    AuthMessageType::Error => "error",
                },
            );
            value.set_field(ctx, "auth_message", message.as_str());
        }
        GreetdResponse::Error {
            authentication,
            description,
        } => {
            value.set_field(ctx, "type", "error");
            value.set_field(ctx, "authentication", authentication);
            value.set_field(ctx, "description", description.as_str());
        }
    }
    value
}

fn track_clock_dependency(state: &Rc<RefCell<ReactiveState>>, enabled: bool) {
    if !enabled {
        return;
    }
    let mut state = state.borrow_mut();
    let clock = state.clock;
    if let Some(active) = &mut state.active {
        active.reads.insert(clock);
    }
}

fn local_time_table<'gc>(ctx: Context<'gc>) -> Table<'gc> {
    let now = jiff::Zoned::now();
    let numeric = now.strftime("%Y\t%m\t%d\t%H\t%M\t%S\t%u").to_string();
    let parts = numeric
        .split('\t')
        .map(|value| value.parse::<i64>().unwrap_or(0))
        .collect::<Vec<_>>();
    let value = Table::new(&ctx);
    for (field, index) in [
        ("year", 0),
        ("month", 1),
        ("day", 2),
        ("hours", 3),
        ("minutes", 4),
        ("seconds", 5),
        ("weekday", 6),
    ] {
        value.set_field(ctx, field, parts.get(index).copied().unwrap_or(0));
    }
    value.set_field(ctx, "date", now.strftime("%F").to_string());
    value.set_field(ctx, "time", now.strftime("%T").to_string());
    value.set_field(ctx, "month_name", now.strftime("%B").to_string());
    value.set_field(ctx, "weekday_name", now.strftime("%A").to_string());
    value.set_field(ctx, "timezone", now.strftime("%Z").to_string());
    value
}

fn bounded_timeout(milliseconds: i64) -> Result<Duration, String> {
    u64::try_from(milliseconds)
        .ok()
        .filter(|milliseconds| *milliseconds <= 5_000)
        .map(Duration::from_millis)
        .ok_or_else(|| "timeout must be between 0 and 5000 milliseconds".to_owned())
}

fn parse_easing<'gc>(ctx: Context<'gc>, value: LuaValue<'gc>) -> Result<Easing, String> {
    match value {
        LuaValue::Nil => Ok(Easing::Linear),
        LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
            "linear" => Ok(Easing::Linear),
            "in_quad" => Ok(Easing::InQuad),
            "out_quad" => Ok(Easing::OutQuad),
            "in_out_quad" => Ok(Easing::InOutQuad),
            "in_cubic" => Ok(Easing::InCubic),
            "out_cubic" => Ok(Easing::OutCubic),
            "in_out_cubic" => Ok(Easing::InOutCubic),
            "in_quart" => Ok(Easing::InQuart),
            "out_quart" => Ok(Easing::OutQuart),
            "in_out_quart" => Ok(Easing::InOutQuart),
            "in_quint" => Ok(Easing::InQuint),
            "out_quint" => Ok(Easing::OutQuint),
            "in_out_quint" => Ok(Easing::InOutQuint),
            "in_sine" => Ok(Easing::InSine),
            "out_sine" => Ok(Easing::OutSine),
            "in_out_sine" => Ok(Easing::InOutSine),
            "in_expo" => Ok(Easing::InExpo),
            "out_expo" => Ok(Easing::OutExpo),
            "in_out_expo" => Ok(Easing::InOutExpo),
            "in_circ" => Ok(Easing::InCirc),
            "out_circ" => Ok(Easing::OutCirc),
            "in_out_circ" => Ok(Easing::InOutCirc),
            "in_back" => Ok(Easing::InBack),
            "out_back" => Ok(Easing::OutBack),
            "in_out_back" => Ok(Easing::InOutBack),
            "in_bounce" => Ok(Easing::InBounce),
            "out_bounce" => Ok(Easing::OutBounce),
            "in_out_bounce" => Ok(Easing::InOutBounce),
            name => Err(format!("unknown easing `{name}`")),
        },
        LuaValue::Table(value) => {
            let read = |field| match value.get_value(ctx, field) {
                LuaValue::Integer(value) => Ok(value as f64),
                LuaValue::Number(value) if value.is_finite() => Ok(value),
                _ => Err(format!("easing {field} must be a finite number")),
            };
            let x1 = read("x1")?;
            let x2 = read("x2")?;
            if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                return Err("easing x1 and x2 must be between 0 and 1".into());
            }
            Ok(Easing::CubicBezier {
                x1,
                y1: read("y1")?,
                x2,
                y2: read("y2")?,
            })
        }
        _ => Err("easing must be a string or cubic Bezier table".to_owned()),
    }
}

