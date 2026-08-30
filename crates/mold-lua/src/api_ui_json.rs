fn install_ui_json_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    mold: Table<'gc>,
    limits: Limits,
) -> (Table<'gc>, Table<'gc>) {
    let ui = Table::new(&ctx);
    for (name, element) in [
        ("Item", Element::Item),
        ("Inset", Element::Inset),
        ("Rect", Element::Rect),
        ("ClipRect", Element::ClipRect),
        ("Text", Element::Text),
        ("Image", Element::Image),
        ("Icon", Element::Icon),
        ("Shape", Element::Shape),
        ("Sdf", Element::Sdf),
        ("SdfShape", Element::SdfShape),
        ("MouseArea", Element::MouseArea),
        ("Row", Element::Row),
        ("Column", Element::Column),
        ("Grid", Element::Grid),
        ("RowLayout", Element::RowLayout),
        ("ColumnLayout", Element::ColumnLayout),
        ("GridLayout", Element::GridLayout),
    ] {
        ui.set_field(
            ctx,
            name,
            element_constructor(ctx, Rc::clone(&state), limits, element),
        );
    }
    ui.set_field(
        ctx,
        "Repeater",
        view_constructor(ctx, Rc::clone(&state), limits, ViewKind::Repeater),
    );
    ui.set_field(
        ctx,
        "ListView",
        view_constructor(ctx, Rc::clone(&state), limits, ViewKind::List),
    );
    ui.set_field(
        ctx,
        "GridView",
        view_constructor(ctx, Rc::clone(&state), limits, ViewKind::Grid),
    );
    ui.set_field(
        ctx,
        "Flickable",
        element_constructor(ctx, Rc::clone(&state), limits, Element::Flickable),
    );
    ui.set_field(
        ctx,
        "Loader",
        loader_constructor(ctx, Rc::clone(&state), limits),
    );
    ui.set_field(
        ctx,
        "Timer",
        timer_constructor(ctx, Rc::clone(&state), limits),
    );
    let reparent_state = Rc::clone(&state);
    let reparent = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (child, parent): (UserRef<NodeToken>, LuaValue) = stack.consume(ctx)?;
        let parent = match parent {
            LuaValue::Nil => None,
            LuaValue::UserData(parent) => Some(
                parent
                    .downcast_static::<NodeToken>()
                    .map_err(|_| HostError("parent must be a mold node or nil".into()))?
                    .handle,
            ),
            _ => return Err(HostError("parent must be a mold node or nil".into()).into()),
        };
        reparent_state
            .borrow_mut()
            .scene
            .reparent(child.handle, parent)
            .map_err(|error| HostError(error.to_string()))?;
        Ok(CallbackReturn::Return)
    });
    ui.set_field(ctx, "reparent", reparent);
    for kind in ["spring", "smoothed"] {
        ui.set_field(
            ctx,
            kind,
            Callback::from_fn(&ctx, move |ctx, _, mut stack| {
                let options: Table = stack.consume(ctx)?;
                options.set_field(ctx, "kind", kind);
                stack.replace(ctx, options);
                Ok(CallbackReturn::Return)
            }),
        );
    }
    mold.set_field(ctx, "ui", ui);
    let json_array_metatable = Table::new(&ctx);
    json_array_metatable.set_field(ctx, "__json_kind", "array");
    json_array_metatable.set_field(ctx, "__metatable", "mold.io.json");
    let json_object_metatable = Table::new(&ctx);
    json_object_metatable.set_field(ctx, "__json_kind", "object");
    json_object_metatable.set_field(ctx, "__metatable", "mold.io.json");
    let json_null_metatable = Table::new(&ctx);
    json_null_metatable.set_field(ctx, "__metatable", "mold.io.json");
    let json_null = UserData::new_static(&ctx, JsonNullToken);
    json_null.set_metatable(ctx, Some(json_null_metatable));
    let array_metatable = ctx.stash(json_array_metatable);
    let object_metatable = ctx.stash(json_object_metatable);
    let null = ctx.stash(json_null);
    let json_decode = Callback::from_fn(&ctx, {
        let array_metatable = array_metatable.clone();
        let object_metatable = object_metatable.clone();
        let null = null.clone();
        move |ctx, _, mut stack| {
            let source: String = stack.consume(ctx)?;
            if source.len() > 1024 * 1024 {
                return Err(HostError("JSON input exceeds 1 MiB".into()).into());
            }
            let value = serde_json::from_str::<serde_json::Value>(&source)
                .map_err(|error| HostError(error.to_string()))?;
            let mut entries = 0;
            let value = json_to_lua(
                ctx,
                &value,
                ctx.fetch(&array_metatable),
                ctx.fetch(&object_metatable),
                ctx.fetch(&null),
                0,
                &mut entries,
            )
            .map_err(HostError)?;
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    let json_encode = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (value, pretty): (LuaValue, LuaValue) = stack.consume(ctx)?;
        let pretty = match pretty {
            LuaValue::Nil => false,
            LuaValue::Boolean(value) => value,
            _ => return Err(HostError("JSON pretty flag must be boolean".into()).into()),
        };
        let mut entries = 0;
        let value = lua_to_json(ctx, value, 0, &mut entries).map_err(HostError)?;
        let encoded = if pretty {
            serde_json::to_string_pretty(&value)
        } else {
            serde_json::to_string(&value)
        }
        .map_err(|error| HostError(error.to_string()))?;
        if encoded.len() > 1024 * 1024 {
            return Err(HostError("JSON output exceeds 1 MiB".into()).into());
        }
        stack.replace(ctx, encoded);
        Ok(CallbackReturn::Return)
    });
    let json_array = Callback::from_fn(&ctx, {
        let array_metatable = array_metatable.clone();
        move |ctx, _, mut stack| {
            let value: Table = stack.consume(ctx)?;
            value.set_metatable(ctx, Some(ctx.fetch(&array_metatable)));
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    let json_object = Callback::from_fn(&ctx, {
        let object_metatable = object_metatable.clone();
        move |ctx, _, mut stack| {
            let value: Table = stack.consume(ctx)?;
            value.set_metatable(ctx, Some(ctx.fetch(&object_metatable)));
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    let json_file_read = Callback::from_fn(&ctx, {
        let array_metatable = array_metatable.clone();
        let object_metatable = object_metatable.clone();
        let null = null.clone();
        move |ctx, _, mut stack| {
            let file: UserRef<FileDocumentToken> = stack.consume(ctx)?;
            let file = file.file.borrow();
            let data = file
                .data()
                .ok_or_else(|| HostError("JSON file view is not loaded".into()))?;
            let value = serde_json::from_slice::<serde_json::Value>(data)
                .map_err(|error| HostError(error.to_string()))?;
            let mut entries = 0;
            let value = json_to_lua(
                ctx,
                &value,
                ctx.fetch(&array_metatable),
                ctx.fetch(&object_metatable),
                ctx.fetch(&null),
                0,
                &mut entries,
            )
            .map_err(HostError)?;
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        }
    });
    let json_file_write = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (file, value, pretty): (UserRef<FileDocumentToken>, LuaValue, LuaValue) =
            stack.consume(ctx)?;
        let pretty = match pretty {
            LuaValue::Nil => true,
            LuaValue::Boolean(value) => value,
            _ => return Err(HostError("JSON pretty flag must be boolean".into()).into()),
        };
        let mut entries = 0;
        let value = lua_to_json(ctx, value, 0, &mut entries).map_err(HostError)?;
        let mut encoded = if pretty {
            serde_json::to_vec_pretty(&value)
        } else {
            serde_json::to_vec(&value)
        }
        .map_err(|error| HostError(error.to_string()))?;
        if pretty {
            encoded.push(b'\n');
        }
        stack.replace(ctx, file.file.borrow_mut().set_data(&encoded));
        Ok(CallbackReturn::Return)
    });
    let json = Table::new(&ctx);
    json.set_field(ctx, "decode", json_decode);
    json.set_field(ctx, "encode", json_encode);
    json.set_field(ctx, "array", json_array);
    json.set_field(ctx, "object", json_object);
    json.set_field(ctx, "null", json_null);
    json.set_field(ctx, "read_file", json_file_read);
    json.set_field(ctx, "write_file", json_file_write);
    mold.set_field(ctx, "json", json);
    (ui, json)
}
