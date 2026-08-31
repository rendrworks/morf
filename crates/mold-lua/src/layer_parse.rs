use luna::{Context, Table, Value as LuaValue};

use mold_scene::NodeHandle;

use crate::{surface_types::*, table_menu::*, window_parse::*};

/// The largest a surface may be on either axis.
///
/// Chosen to match what `window:size()` has always enforced; the point is that
/// one number now answers the question for every door onto a surface size.
pub(crate) const MAX_SURFACE_EXTENT: u32 = 16_384;

/// Assigns one setting and reports whether the value actually moved.
///
/// A layer surface is reconfigured from the change flag this returns, so an
/// assignment that writes the value already there must not raise it: Lua
/// re-assigns the whole surface table on every binding run, and a flag set by
/// an unchanged value would re-issue the geometry on every frame.
pub(crate) fn assign_layer_setting<T: PartialEq>(field: &mut T, value: T) -> bool {
    let changed = *field != value;
    *field = value;
    changed
}

/// Reads a numeric layer setting that an animation may deliver as a float.
///
/// `mold.surface.margin_left = x` is exactly the assignment a slide animation
/// makes, and an interpolated value arrives as a Lua number rather than an
/// integer. Rounding it keeps the setting animatable; rejecting it would make
/// the geometry mutable in name only.
pub(crate) fn layer_setting_number(value: LuaValue<'_>, key: &str) -> Result<i64, String> {
    match value {
        LuaValue::Integer(value) => Ok(value),
        LuaValue::Number(value) if value.is_finite() => Ok(value.round() as i64),
        _ => Err(format!("surface {key} must be a number")),
    }
}

/// Applies one validated layer-surface setting to a configuration.
///
/// `mold.surface.<key> = value` and `window.layer { <key> = value }` are the
/// same settings on the same struct, so they share one validator rather than
/// drifting apart. The returned flag reports whether the configuration moved.
pub(crate) fn apply_layer_setting<'gc>(
    ctx: Context<'gc>,
    config: &mut LayerSurfaceConfig,
    key: &str,
    value: LuaValue<'gc>,
) -> Result<bool, String> {
    match key {
        "namespace" => {
            let LuaValue::String(value) = value else {
                return Err("surface namespace must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if value.is_empty() || value.len() > 128 {
                return Err("surface namespace must contain 1 to 128 bytes".into());
            }
            Ok(assign_layer_setting(&mut config.namespace, value))
        }
        // Both axes, one rule: zero means "as wide as the compositor makes it",
        // which is what an edge-to-edge bar wants, and anything else has to be
        // a size a surface can actually be. Width used to accept any `u32` and
        // height to demand a positive one — two rules for two halves of one
        // size, with a third at `window:size()`.
        "width" | "height" => {
            let value = layer_setting_number(value, key)?;
            let value = u32::try_from(value).map_err(|_| format!("surface {key} must fit u32"))?;
            if value > MAX_SURFACE_EXTENT {
                return Err(format!(
                    "surface {key} must be 0 (compositor-sized) or at most {MAX_SURFACE_EXTENT}"
                ));
            }
            if key == "height" {
                return Ok(assign_layer_setting(&mut config.height, value));
            }
            Ok(assign_layer_setting(&mut config.width, value))
        }
        "exclusive_zone" | "margin_top" | "margin_right" | "margin_bottom" | "margin_left" => {
            let value = layer_setting_number(value, key)?;
            let value = i32::try_from(value).map_err(|_| format!("surface {key} must fit i32"))?;
            Ok(match key {
                "exclusive_zone" => assign_layer_setting(&mut config.exclusive_zone, value),
                "margin_top" => assign_layer_setting(&mut config.margin_top, value),
                "margin_right" => assign_layer_setting(&mut config.margin_right, value),
                "margin_bottom" => assign_layer_setting(&mut config.margin_bottom, value),
                "margin_left" => assign_layer_setting(&mut config.margin_left, value),
                _ => unreachable!(),
            })
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
            Ok(assign_layer_setting(
                &mut config.anchors,
                SurfaceAnchors {
                    top: read("top")?,
                    right: read("right")?,
                    bottom: read("bottom")?,
                    left: read("left")?,
                },
            ))
        }
        "layer" => {
            let LuaValue::String(value) = value else {
                return Err("surface layer must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if !matches!(value.as_str(), "background" | "bottom" | "top" | "overlay") {
                return Err("surface layer must be background, bottom, top, or overlay".into());
            }
            Ok(assign_layer_setting(&mut config.layer, value))
        }
        "keyboard_focus" => {
            let LuaValue::String(value) = value else {
                return Err("surface keyboard_focus must be a string".into());
            };
            let value = value.display_lossy().to_string();
            if !matches!(value.as_str(), "none" | "exclusive" | "on_demand") {
                return Err("surface keyboard_focus must be none, exclusive, or on_demand".into());
            }
            Ok(assign_layer_setting(&mut config.keyboard_focus, value))
        }
        "mask" => {
            let regions = match value {
                LuaValue::Nil => None,
                LuaValue::Table(value) => Some(vec![parse_region(ctx, value, 0)?]),
                _ => return Err("surface mask must be a region table".into()),
            };
            Ok(assign_layer_setting(&mut config.input_regions, regions))
        }
        "reserve" => Ok(assign_layer_setting(
            &mut config.reserve,
            parse_surface_reserve(ctx, value)?,
        )),
        _ => Err(format!("unknown surface setting `{key}`")),
    }
}

/// Reads the per-edge thicknesses of `mold.surface.reserve`.
pub(crate) fn parse_surface_reserve<'gc>(
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
pub(crate) fn parse_layer_surface<'gc>(
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
pub(crate) fn reserve_to_lua<'gc>(ctx: Context<'gc>, reserve: SurfaceReserve) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (name, thickness) in reserve.edges() {
        table.set_field(ctx, name, i64::from(thickness));
    }
    table
}
