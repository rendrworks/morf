use luna::{Context, Table, Value as LuaValue};
use morf_image::{ImageRect as QuantizeRect, quantize_colors};
use morf_io::{Process, ProcessConfig};
use morf_scene::NodeHandle;
use std::path::PathBuf;

use crate::{state::*, surface_types::*};

pub(crate) fn update_process_view_config(
    process: &ProcessViewToken,
    update: impl FnOnce(&mut ProcessConfig),
) -> Result<(), String> {
    let mut state = process.state.borrow_mut();
    let mut config = state.config.clone();
    update(&mut config);
    let replacement = state
        .process
        .is_some()
        .then(|| Process::spawn_config(&config))
        .transpose()
        .map_err(|error| error.to_string())?;
    state.config = config;
    if let Some(replacement) = replacement {
        state.process = Some(replacement);
    }
    Ok(())
}

pub(crate) fn parse_quantizer_options<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<(PathBuf, u8, Option<QuantizeRect>, u32), String> {
    let source = match options.get_value(ctx, "source") {
        LuaValue::String(value) => PathBuf::from(value.display_lossy().to_string()),
        _ => return Err("color_quantize source must be a string".into()),
    };
    let depth = match options.get_value(ctx, "depth") {
        LuaValue::Nil => 3,
        LuaValue::Integer(value) => u8::try_from(value)
            .ok()
            .filter(|value| *value <= 8)
            .ok_or_else(|| "color_quantize depth must be 0..8".to_owned())?,
        _ => return Err("color_quantize depth must be an integer".into()),
    };
    let rescale_size = match options.get_value(ctx, "rescale_size") {
        LuaValue::Nil => 64,
        LuaValue::Integer(value) => u32::try_from(value)
            .ok()
            .filter(|value| *value <= 512)
            .ok_or_else(|| "color_quantize rescale_size must be 0..512".to_owned())?,
        _ => return Err("color_quantize rescale_size must be an integer".into()),
    };
    let crop = parse_quantizer_rect(ctx, options.get_value(ctx, "rect"))?;
    Ok((source, depth, crop, rescale_size))
}

pub(crate) fn parse_quantizer_rect<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
) -> Result<Option<QuantizeRect>, String> {
    let LuaValue::Table(rect) = value else {
        return match value {
            LuaValue::Nil => Ok(None),
            _ => Err("color_quantize rect must be a table or nil".into()),
        };
    };
    let read = |field| match rect.get_value(ctx, field) {
        LuaValue::Integer(value) => u32::try_from(value)
            .map_err(|_| format!("color_quantize rect {field} must be nonnegative")),
        _ => Err(format!("color_quantize rect {field} must be an integer")),
    };
    let rect = QuantizeRect {
        x: read("x")?,
        y: read("y")?,
        width: read("width")?,
        height: read("height")?,
    };
    if rect.width == 0 || rect.height == 0 {
        return Err("color_quantize rect size must be positive".into());
    }
    Ok(Some(rect))
}

pub(crate) fn quantizer_colors_to_lua<'gc>(ctx: Context<'gc>, colors: &[[u8; 4]]) -> Table<'gc> {
    let values = Table::new(&ctx);
    for (index, color) in colors.iter().enumerate() {
        let encoded = if color[3] == 255 {
            format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                color[0], color[1], color[2], color[3]
            )
        };
        values
            .set(ctx, index as i64 + 1, encoded)
            .expect("color table accepts indexed strings");
    }
    values
}

pub(crate) fn update_color_quantizer(
    quantizer: &ColorQuantizerToken,
    update: impl FnOnce(&mut ColorQuantizerState),
) -> Result<(), String> {
    let mut state = quantizer.state.borrow_mut();
    let mut next = state.clone();
    update(&mut next);
    next.colors = quantize_colors(&next.source, next.depth, next.crop, next.rescale_size)
        .map_err(|error| error.to_string())?;
    *state = next;
    Ok(())
}

/// Registers one window surface and returns the id it was given.
///
/// The id allocation, the insert and the change flag were written out once per
/// surface kind — three copies of the same six lines, differing only in which
/// `WindowSurfaceKind` they wrapped — so adding a field to the record meant
/// finding all three.
pub(crate) fn register_window_surface(
    state: &mut ReactiveState,
    root: NodeHandle,
    visible: bool,
    updates_enabled: bool,
    kind: WindowSurfaceKind,
) -> u64 {
    let id = state.next_window_surface;
    state.next_window_surface = state.next_window_surface.wrapping_add(1);
    state.window_surfaces.insert(
        id,
        WindowSurfaceConfig {
            id,
            root,
            visible,
            updates_enabled,
            kind,
        },
    );
    state.window_surfaces_changed = true;
    id
}
