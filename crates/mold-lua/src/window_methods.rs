fn window_updates_enabled_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, value): (UserRef<WindowSurfaceToken>, Option<bool>) = stack.consume(ctx)?;
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let changed = value.is_some_and(|value| surface.updates_enabled != value);
            if let Some(value) = value {
                surface.updates_enabled = value;
            }
            (surface.updates_enabled, changed)
        };
        state.window_surfaces_changed |= changed;
        stack.replace(ctx, current);
        Ok(CallbackReturn::Return)
    })
}

fn window_size_method<'gc>(ctx: Context<'gc>, state: Rc<RefCell<ReactiveState>>) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, width, height): (UserRef<WindowSurfaceToken>, Option<i64>, Option<i64>) =
            stack.consume(ctx)?;
        if width.is_some() != height.is_some() {
            return Err(HostError("window size requires both width and height".into()).into());
        }
        let values = match (width, height) {
            (Some(width), Some(height)) => Some((
                u32::try_from(width).map_err(|_| HostError("width must fit u32".into()))?,
                u32::try_from(height).map_err(|_| HostError("height must fit u32".into()))?,
            )),
            _ => None,
        };
        if values.is_some_and(|(width, height)| {
            width == 0 || height == 0 || width > 16_384 || height > 16_384
        }) {
            return Err(HostError("window size is outside 1..16384".into()).into());
        }
        let mut state = state.borrow_mut();
        let (width, height, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let size = match &mut surface.kind {
                WindowSurfaceKind::Popup(config) => (&mut config.width, &mut config.height),
                WindowSurfaceKind::Floating(config) => (&mut config.width, &mut config.height),
            };
            let before = (*size.0, *size.1);
            if let Some((width, height)) = values {
                (*size.0, *size.1) = (width, height);
            }
            (*size.0, *size.1, before != (*size.0, *size.1))
        };
        state.window_surfaces_changed |= changed;
        let result = Table::new(&ctx);
        result.set_field(ctx, "width", i64::from(width));
        result.set_field(ctx, "height", i64::from(height));
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    })
}

fn popup_bool_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    property: &'static str,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, value): (UserRef<WindowSurfaceToken>, Option<bool>) = stack.consume(ctx)?;
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Popup(config) = &mut surface.kind else {
                return Err(HostError(format!("{property} is only valid for popups")).into());
            };
            let current = match property {
                "grab_focus" => &mut config.grab_focus,
                _ => unreachable!(),
            };
            let changed = value.is_some_and(|value| *current != value);
            if let Some(value) = value {
                *current = value;
            }
            (*current, changed)
        };
        state.window_surfaces_changed |= changed;
        stack.replace(ctx, current);
        Ok(CallbackReturn::Return)
    })
}

fn popup_string_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    property: &'static str,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, value): (UserRef<WindowSurfaceToken>, Option<String>) = stack.consume(ctx)?;
        if value.as_deref().is_some_and(|value| !popup_position(value)) {
            return Err(HostError(format!("invalid popup {property}")).into());
        }
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Popup(config) = &mut surface.kind else {
                return Err(HostError(format!("{property} is only valid for popups")).into());
            };
            let current = match property {
                "anchor_edge" => &mut config.anchor_edge,
                "gravity" => &mut config.gravity,
                _ => unreachable!(),
            };
            let changed = value.as_ref().is_some_and(|value| *current != *value);
            if let Some(value) = value {
                *current = value;
            }
            (current.clone(), changed)
        };
        state.window_surfaces_changed |= changed;
        stack.replace(ctx, current);
        Ok(CallbackReturn::Return)
    })
}

fn popup_anchor_rect_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, x, y, width, height): PopupAnchorArgs<'gc> = stack.consume(ctx)?;
        let supplied = [x, y, width, height]
            .iter()
            .filter(|value| value.is_some())
            .count();
        if supplied != 0 && supplied != 4 {
            return Err(
                HostError("popup anchor_rect requires x, y, width and height".into()).into(),
            );
        }
        let values = if let (Some(x), Some(y), Some(width), Some(height)) = (x, y, width, height) {
            let x = i32::try_from(x).map_err(|_| HostError("anchor x must fit i32".into()))?;
            let y = i32::try_from(y).map_err(|_| HostError("anchor y must fit i32".into()))?;
            let width = i32::try_from(width)
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| HostError("anchor width must be positive i32".into()))?;
            let height = i32::try_from(height)
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| HostError("anchor height must be positive i32".into()))?;
            Some((x, y, width, height))
        } else {
            None
        };
        let clear_node_anchor = values.is_some();
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let id = surface.id;
            let surface = state
                .window_surfaces
                .get_mut(&id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Popup(config) = &mut surface.kind else {
                return Err(HostError("anchor_rect is only valid for popups".into()).into());
            };
            let before = (
                config.anchor_x,
                config.anchor_y,
                config.anchor_width,
                config.anchor_height,
            );
            if let Some(values) = values {
                (
                    config.anchor_x,
                    config.anchor_y,
                    config.anchor_width,
                    config.anchor_height,
                ) = values;
            }
            (
                (
                    config.anchor_x,
                    config.anchor_y,
                    config.anchor_width,
                    config.anchor_height,
                ),
                before
                    != (
                        config.anchor_x,
                        config.anchor_y,
                        config.anchor_width,
                        config.anchor_height,
                    ),
            )
        };
        if clear_node_anchor {
            state.popup_node_anchors.remove(&surface.id);
        }
        state.window_surfaces_changed |= changed;
        let result = Table::new(&ctx);
        result.set_field(ctx, "x", i64::from(current.0));
        result.set_field(ctx, "y", i64::from(current.1));
        result.set_field(ctx, "width", i64::from(current.2));
        result.set_field(ctx, "height", i64::from(current.3));
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    })
}

fn popup_offset_method<'gc>(ctx: Context<'gc>, state: Rc<RefCell<ReactiveState>>) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, x, y): (UserRef<WindowSurfaceToken>, Option<i64>, Option<i64>) =
            stack.consume(ctx)?;
        if x.is_some() != y.is_some() {
            return Err(HostError("popup offset requires both x and y".into()).into());
        }
        let values = match (x, y) {
            (Some(x), Some(y)) => Some((
                i32::try_from(x).map_err(|_| HostError("offset x must fit i32".into()))?,
                i32::try_from(y).map_err(|_| HostError("offset y must fit i32".into()))?,
            )),
            _ => None,
        };
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Popup(config) = &mut surface.kind else {
                return Err(HostError("offset is only valid for popups".into()).into());
            };
            let before = (config.offset_x, config.offset_y);
            if let Some(values) = values {
                (config.offset_x, config.offset_y) = values;
            }
            (
                (config.offset_x, config.offset_y),
                before != (config.offset_x, config.offset_y),
            )
        };
        state.window_surfaces_changed |= changed;
        let result = Table::new(&ctx);
        result.set_field(ctx, "x", i64::from(current.0));
        result.set_field(ctx, "y", i64::from(current.1));
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    })
}

fn popup_constraints_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, values): (UserRef<WindowSurfaceToken>, Option<Table>) = stack.consume(ctx)?;
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Popup(config) = &mut surface.kind else {
                return Err(HostError("constraints is only valid for popups".into()).into());
            };
            let before = config.constraints;
            if let Some(values) = values {
                for (field, current) in [
                    ("slide_x", &mut config.constraints.slide_x),
                    ("slide_y", &mut config.constraints.slide_y),
                    ("flip_x", &mut config.constraints.flip_x),
                    ("flip_y", &mut config.constraints.flip_y),
                    ("resize_x", &mut config.constraints.resize_x),
                    ("resize_y", &mut config.constraints.resize_y),
                ] {
                    match values.get_value(ctx, field) {
                        LuaValue::Nil => {}
                        LuaValue::Boolean(value) => *current = value,
                        _ => {
                            return Err(HostError(format!(
                                "popup constraint {field} must be boolean"
                            ))
                            .into());
                        }
                    }
                }
            }
            (config.constraints, before != config.constraints)
        };
        state.window_surfaces_changed |= changed;
        let result = Table::new(&ctx);
        result.set_field(ctx, "slide_x", current.slide_x);
        result.set_field(ctx, "slide_y", current.slide_y);
        result.set_field(ctx, "flip_x", current.flip_x);
        result.set_field(ctx, "flip_y", current.flip_y);
        result.set_field(ctx, "resize_x", current.resize_x);
        result.set_field(ctx, "resize_y", current.resize_y);
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    })
}

fn window_parent_id_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
        let parent = state
            .borrow()
            .window_surfaces
            .get(&surface.id)
            .map(|surface| match &surface.kind {
                WindowSurfaceKind::Popup(config) => config.parent,
                WindowSurfaceKind::Floating(config) => config.parent,
            })
            .ok_or_else(|| HostError("window surface is stale".into()))?;
        stack.replace(
            ctx,
            parent.map_or(LuaValue::Nil, |id| LuaValue::Integer(id as i64)),
        );
        Ok(CallbackReturn::Return)
    })
}

fn window_set_parent_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, parent): (
            UserRef<WindowSurfaceToken>,
            Option<UserRef<WindowSurfaceToken>>,
        ) = stack.consume(ctx)?;
        let parent = parent.map(|parent| parent.id);
        if parent == Some(surface.id) {
            return Err(HostError("a window cannot parent itself".into()).into());
        }
        let mut state = state.borrow_mut();
        if let Some(parent) = parent {
            let parent_surface = state
                .window_surfaces
                .get(&parent)
                .ok_or_else(|| HostError("window parent is stale".into()))?;
            if !matches!(parent_surface.kind, WindowSurfaceKind::Floating(_)) {
                return Err(HostError("window parent must be a floating surface".into()).into());
            }
            let mut current = Some(parent);
            let mut depth = 0;
            while let Some(id) = current {
                if id == surface.id {
                    return Err(
                        HostError("window parent relationship contains a cycle".into()).into(),
                    );
                }
                depth += 1;
                if depth > 64 {
                    return Err(HostError("window parent chain exceeds 64 levels".into()).into());
                }
                current = state
                    .window_surfaces
                    .get(&id)
                    .and_then(|surface| match &surface.kind {
                        WindowSurfaceKind::Popup(config) => config.parent,
                        WindowSurfaceKind::Floating(config) => config.parent,
                    });
            }
        }
        let changed = {
            let target = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            match &mut target.kind {
                WindowSurfaceKind::Popup(config) => {
                    let changed = config.parent != parent;
                    config.parent = parent;
                    changed
                }
                WindowSurfaceKind::Floating(config) => {
                    if target.visible && config.parent != parent {
                        return Err(HostError(
                            "floating parent cannot change while the window is visible".into(),
                        )
                        .into());
                    }
                    let changed = config.parent != parent;
                    config.parent = parent;
                    changed
                }
            }
        };
        state.window_surfaces_changed |= changed;
        stack.replace(
            ctx,
            parent.map_or(LuaValue::Nil, |id| LuaValue::Integer(id as i64)),
        );
        Ok(CallbackReturn::Return)
    })
}

