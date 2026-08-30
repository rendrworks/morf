/// Applies one validated layer-surface setting to a configuration.
///
/// `mold.surface.<key> = value` and `window.layer { <key> = value }` are the
/// same settings on the same struct, so they share one validator rather than
/// drifting apart.
fn apply_layer_setting<'gc>(
    ctx: Context<'gc>,
    config: &mut LayerSurfaceConfig,
    key: &str,
    value: LuaValue<'gc>,
) -> Result<(), String> {
    match key {
        "namespace" => {
            let LuaValue::String(value) = value else {
                return Err("surface namespace must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if value.is_empty() || value.len() > 128 {
                return Err("surface namespace must contain 1 to 128 bytes".into());
            }
            config.namespace = value;
        }
        "width" | "height" => {
            let LuaValue::Integer(value) = value else {
                return Err(format!("surface {key} must be an integer"));
            };
            let value = u32::try_from(value).map_err(|_| format!("surface {key} must fit u32"))?;
            if key == "height" && value == 0 {
                return Err("surface height must be positive".into());
            }
            if key == "width" {
                config.width = value;
            } else {
                config.height = value;
            }
        }
        "exclusive_zone" | "margin_top" | "margin_right" | "margin_bottom" | "margin_left" => {
            let LuaValue::Integer(value) = value else {
                return Err(format!("surface {key} must be an integer"));
            };
            let value = i32::try_from(value).map_err(|_| format!("surface {key} must fit i32"))?;
            match key {
                "exclusive_zone" => config.exclusive_zone = value,
                "margin_top" => config.margin_top = value,
                "margin_right" => config.margin_right = value,
                "margin_bottom" => config.margin_bottom = value,
                "margin_left" => config.margin_left = value,
                _ => unreachable!(),
            }
        }
        "anchors" => {
            let LuaValue::Table(value) = value else {
                return Err("surface anchors must be a table".into());
            };
            let read = |name| match value.get_value(ctx, name) {
                LuaValue::Nil => Ok(false),
                LuaValue::Boolean(value) => Ok(value),
                _ => Err(format!("surface anchor {name} must be boolean")),
            };
            config.anchors = SurfaceAnchors {
                top: read("top")?,
                right: read("right")?,
                bottom: read("bottom")?,
                left: read("left")?,
            };
        }
        "layer" => {
            let LuaValue::String(value) = value else {
                return Err("surface layer must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if !matches!(value.as_str(), "background" | "bottom" | "top" | "overlay") {
                return Err("surface layer must be background, bottom, top, or overlay".into());
            }
            config.layer = value;
        }
        "keyboard_focus" => {
            let LuaValue::String(value) = value else {
                return Err("surface keyboard_focus must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if !matches!(value.as_str(), "none" | "exclusive" | "on_demand") {
                return Err("surface keyboard_focus must be none, exclusive, or on_demand".into());
            }
            config.keyboard_focus = value;
        }
        "mask" => {
            config.input_regions = match value {
                LuaValue::Nil => None,
                LuaValue::Table(value) => Some(vec![parse_region(ctx, value, 0)?]),
                _ => return Err("surface mask must be a region table".into()),
            };
        }
        "reserve" => config.reserve = parse_surface_reserve(ctx, value)?,
        _ => return Err(format!("unknown surface setting `{key}`")),
    }
    Ok(())
}

/// Reads the per-edge thicknesses of `mold.surface.reserve`.
fn parse_surface_reserve<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
) -> Result<SurfaceReserve, String> {
    let value = match value {
        LuaValue::Nil => return Ok(SurfaceReserve::default()),
        LuaValue::Table(value) => value,
        _ => return Err("surface reserve must be a table".into()),
    };
    for (key, _) in value.iter(ctx) {
        let LuaValue::String(key) = key else {
            return Err("surface reserve keys must be strings".into());
        };
        if !matches!(
            key.display_lossy().to_string().as_str(),
            "top" | "right" | "bottom" | "left"
        ) {
            return Err("surface reserve accepts only top, right, bottom, and left".into());
        }
    }
    let edge = |name: &str| match value.get_value(ctx, name) {
        LuaValue::Nil => Ok(0),
        LuaValue::Integer(thickness) => u32::try_from(thickness)
            .ok()
            .filter(|thickness| *thickness <= 16_384)
            .ok_or_else(|| format!("surface reserve {name} must be 0..16384")),
        _ => Err(format!("surface reserve {name} must be an integer")),
    };
    Ok(SurfaceReserve {
        top: edge("top")?,
        right: edge("right")?,
        bottom: edge("bottom")?,
        left: edge("left")?,
    })
}

/// Reads a `window.layer { ... }` table into a scene root and its configuration.
///
/// An additional layer surface defaults to reserving nothing: unlike the shell's
/// own surface it is normally decoration drawn outside the usable area, and a
/// surface that claims an exclusive zone by accident moves every tiled window.
fn parse_layer_surface<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<(NodeHandle, bool, LayerSurfaceConfig), String> {
    let root = window_root(ctx, options)?;
    let visible = table_bool(ctx, options, "visible", false)?;
    let mut config = LayerSurfaceConfig {
        exclusive_zone: 0,
        ..LayerSurfaceConfig::default()
    };
    let mut settings = Vec::new();
    for (key, value) in options.iter(ctx) {
        let LuaValue::String(key) = key else {
            return Err("layer surface settings must have string keys".into());
        };
        let key = key.display_lossy().to_string();
        if matches!(key.as_str(), "root" | "visible" | "updates_enabled") {
            continue;
        }
        if key == "reserve" {
            return Err("reserve is only available on mold.surface".into());
        }
        settings.push((key, value));
    }
    settings.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in settings {
        apply_layer_setting(ctx, &mut config, &key, value)?;
    }
    Ok((root, visible, config))
}

/// Renders a reserve configuration back to Lua.
fn reserve_to_lua<'gc>(ctx: Context<'gc>, reserve: SurfaceReserve) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (name, thickness) in reserve.edges() {
        table.set_field(ctx, name, i64::from(thickness));
    }
    table
}
