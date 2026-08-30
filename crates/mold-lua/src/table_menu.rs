fn table_number<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: f64,
) -> Result<f64, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => Ok(value as f64),
        LuaValue::Number(value) if value.is_finite() => Ok(value),
        _ => Err(format!("{field} must be a finite number")),
    }
}

fn table_required_number<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
) -> Result<f64, String> {
    match table.get_value(ctx, field) {
        LuaValue::Integer(value) => Ok(value as f64),
        LuaValue::Number(value) if value.is_finite() => Ok(value),
        _ => Err(format!("{field} must be a finite number")),
    }
}

fn parse_menu_entries<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    depth: usize,
    callbacks: &mut HashMap<String, StashedClosure>,
) -> Result<Vec<MenuEntry>, String> {
    if depth >= 32 {
        return Err("menu exceeds 32 levels".into());
    }
    let mut values = Vec::new();
    for (key, value) in table.iter(ctx) {
        let LuaValue::Integer(index) = key else {
            continue;
        };
        let LuaValue::Table(value) = value else {
            return Err(format!("menu entry {index} must be a table"));
        };
        values.push((index, value));
    }
    values.sort_by_key(|(index, _)| *index);
    for (offset, (index, _)) in values.iter().enumerate() {
        if *index != offset as i64 + 1 {
            return Err("menu entries must be a dense sequence".into());
        }
    }
    let mut entries = Vec::with_capacity(values.len());
    for (_, value) in values {
        let id = table_string(ctx, value, "id", "")?;
        let separator = table_bool(ctx, value, "separator", false)?;
        let mut entry = if separator {
            MenuEntry::separator(id.clone())
        } else {
            MenuEntry::item(id.clone(), table_string(ctx, value, "text", "")?)
        };
        entry.enabled = table_bool(ctx, value, "enabled", !separator)?;
        entry.visible = table_bool(ctx, value, "visible", true)?;
        entry.icon = match value.get_value(ctx, "icon") {
            LuaValue::Nil => None,
            LuaValue::String(icon) => Some(icon.display_lossy().to_string()),
            _ => return Err(format!("menu entry `{id}` icon must be a string or nil")),
        };
        entry.button_type = match value.get_value(ctx, "button_type") {
            LuaValue::Nil => ButtonType::None,
            LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
                "none" => ButtonType::None,
                "checkbox" => ButtonType::CheckBox,
                "radio" => ButtonType::RadioButton,
                _ => return Err(format!("menu entry `{id}` has an invalid button_type")),
            },
            _ => return Err(format!("menu entry `{id}` button_type must be a string")),
        };
        entry.check_state = parse_check_state(value.get_value(ctx, "checked"))?;
        entry.radio_group = match value.get_value(ctx, "radio_group") {
            LuaValue::Nil => None,
            LuaValue::String(group) => Some(group.display_lossy().to_string()),
            _ => {
                return Err(format!(
                    "menu entry `{id}` radio_group must be a string or nil"
                ));
            }
        };
        entry.children = match value.get_value(ctx, "children") {
            LuaValue::Nil => Vec::new(),
            LuaValue::Table(children) => parse_menu_entries(ctx, children, depth + 1, callbacks)?,
            _ => return Err(format!("menu entry `{id}` children must be a table")),
        };
        match value.get_value(ctx, "on_triggered") {
            LuaValue::Nil => {}
            LuaValue::Function(Function::Closure(closure)) => {
                callbacks.insert(id, ctx.stash(closure));
            }
            _ => return Err("menu on_triggered must be a function".into()),
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_check_state(value: LuaValue<'_>) -> Result<CheckState, String> {
    match value {
        LuaValue::Nil | LuaValue::Boolean(false) => Ok(CheckState::Unchecked),
        LuaValue::Boolean(true) => Ok(CheckState::Checked),
        LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
            "unchecked" => Ok(CheckState::Unchecked),
            "partial" => Ok(CheckState::PartiallyChecked),
            "checked" => Ok(CheckState::Checked),
            _ => Err("menu checked must be boolean, partial, checked, or unchecked".into()),
        },
        _ => Err("menu checked must be boolean or a check state string".into()),
    }
}

fn menu_entries_to_lua<'gc>(ctx: Context<'gc>, entries: &[MenuEntry]) -> Table<'gc> {
    let values = Table::new(&ctx);
    for (index, entry) in entries.iter().enumerate() {
        values
            .set(ctx, index as i64 + 1, menu_entry_to_lua(ctx, entry))
            .expect("menu table accepts integer keys");
    }
    values
}

fn menu_entry_to_lua<'gc>(ctx: Context<'gc>, entry: &MenuEntry) -> Table<'gc> {
    let value = Table::new(&ctx);
    value.set_field(ctx, "id", entry.id.as_str());
    value.set_field(ctx, "separator", entry.separator);
    value.set_field(ctx, "enabled", entry.enabled);
    value.set_field(ctx, "visible", entry.visible);
    value.set_field(ctx, "text", entry.text.as_str());
    match &entry.icon {
        Some(icon) => value.set_field(ctx, "icon", icon.as_str()),
        None => value.set_field(ctx, "icon", LuaValue::Nil),
    };
    value.set_field(
        ctx,
        "button_type",
        match entry.button_type {
            ButtonType::None => "none",
            ButtonType::CheckBox => "checkbox",
            ButtonType::RadioButton => "radio",
        },
    );
    value.set_field(ctx, "check_state", check_state_name(entry.check_state));
    value.set_field(ctx, "checked", entry.check_state == CheckState::Checked);
    match &entry.radio_group {
        Some(group) => value.set_field(ctx, "radio_group", group.as_str()),
        None => value.set_field(ctx, "radio_group", LuaValue::Nil),
    };
    value.set_field(ctx, "has_children", !entry.children.is_empty());
    value.set_field(ctx, "children", menu_entries_to_lua(ctx, &entry.children));
    value
}

fn check_state_name(state: CheckState) -> &'static str {
    match state {
        CheckState::Unchecked => "unchecked",
        CheckState::PartiallyChecked => "partial",
        CheckState::Checked => "checked",
    }
}

fn table_bool<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: bool,
) -> Result<bool, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Boolean(value) => Ok(value),
        _ => Err(format!("{field} must be boolean")),
    }
}

fn optional_closure<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
) -> Result<Option<StashedClosure>, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(None),
        LuaValue::Function(Function::Closure(closure)) => Ok(Some(ctx.stash(closure))),
        _ => Err(format!("{field} must be a function or nil")),
    }
}

fn table_string<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: &str,
) -> Result<String, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default.to_owned()),
        LuaValue::String(value) => Ok(value.display_lossy().to_string()),
        _ => Err(format!("{field} must be a string")),
    }
}

fn table_string_array<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    maximum: usize,
) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for (key, value) in table.iter(ctx) {
        let LuaValue::Integer(index) = key else {
            return Err("argument list keys must be integers".to_owned());
        };
        let LuaValue::String(value) = value else {
            return Err("process arguments must be strings".to_owned());
        };
        values.push((index, value.display_lossy().to_string()));
    }
    if values.len() > maximum {
        return Err(format!("argument list exceeds {maximum} entries"));
    }
    values.sort_by_key(|(index, _)| *index);
    for (offset, (index, _)) in values.iter().enumerate() {
        if *index != offset as i64 + 1 {
            return Err("argument list must be a dense sequence".to_owned());
        }
    }
    Ok(values.into_iter().map(|(_, value)| value).collect())
}

fn table_string_map<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    maximum: usize,
) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for (key, value) in table.iter(ctx) {
        let LuaValue::String(key) = key else {
            return Err("environment keys must be strings".to_owned());
        };
        let LuaValue::String(value) = value else {
            return Err("environment values must be strings".to_owned());
        };
        let key = key.display_lossy().to_string();
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err("environment variable name is invalid".to_owned());
        }
        let value = value.display_lossy().to_string();
        if value.contains('\0') {
            return Err("environment variable value is invalid".to_owned());
        }
        values.insert(key, value);
        if values.len() > maximum {
            return Err(format!("environment exceeds {maximum} entries"));
        }
    }
    Ok(values)
}

