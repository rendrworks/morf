use luna::{Context, Table, Value as LuaValue};

use mold_region::{
    Operation as RegionOperation, Rect as RegionRect, Region, Shape as RegionShape, ShapeParams,
};
use mold_scene::NodeHandle;

use crate::{state::*, surface_types::*, table_menu::*};

pub(crate) fn window_root<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<NodeHandle, String> {
    let LuaValue::UserData(root) = options.get_value(ctx, "root") else {
        return Err("window root must be a mold node".into());
    };
    root.downcast_static::<NodeToken>()
        .map(|root| root.handle)
        .map_err(|_| "window root must be a mold node".into())
}

pub(crate) fn window_u32<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: u32,
) -> Result<u32, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => u32::try_from(value)
            .ok()
            .filter(|value| (1..=16_384).contains(value))
            .ok_or_else(|| format!("{field} must be 1..16384")),
        _ => Err(format!("{field} must be an integer")),
    }
}

pub(crate) fn window_i32<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: i32,
) -> Result<i32, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => {
            i32::try_from(value).map_err(|_| format!("{field} is outside the signed 32-bit range"))
        }
        _ => Err(format!("{field} must be an integer")),
    }
}

pub(crate) fn window_optional_u32<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
) -> Result<Option<u32>, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(value) => u32::try_from(value)
            .ok()
            .filter(|value| (1..=16_384).contains(value))
            .map(Some)
            .ok_or_else(|| format!("{field} must be 1..16384")),
        _ => Err(format!("{field} must be an integer")),
    }
}

pub(crate) fn window_optional_i32<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
) -> Result<Option<i32>, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(value) => i32::try_from(value)
            .map(Some)
            .map_err(|_| format!("{field} is outside the signed 32-bit range")),
        _ => Err(format!("{field} must be an integer")),
    }
}

pub(crate) fn popup_position(value: &str) -> bool {
    matches!(
        value,
        "none"
            | "top"
            | "bottom"
            | "left"
            | "right"
            | "top_left"
            | "top_right"
            | "bottom_left"
            | "bottom_right"
    )
}

pub(crate) fn window_parent(value: LuaValue<'_>, field: &str) -> Result<Option<u64>, String> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::UserData(parent) => parent
            .downcast_static::<WindowSurfaceToken>()
            .map(|parent| Some(parent.id))
            .map_err(|_| format!("{field} must be a mold window surface")),
        _ => Err(format!("{field} must be a mold window surface")),
    }
}

pub(crate) fn parse_popup_surface<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<
    (
        NodeHandle,
        bool,
        PopupSurfaceConfig,
        Option<PopupNodeAnchor>,
    ),
    String,
> {
    let root = window_root(ctx, options)?;
    let visible = table_bool(ctx, options, "visible", false)?;
    let anchor = match options.get_value(ctx, "anchor") {
        LuaValue::Nil => None,
        LuaValue::Table(anchor) => Some(anchor),
        _ => return Err("popup anchor must be a table".into()),
    };
    let parent = match options.get_value(ctx, "parent") {
        LuaValue::Nil => anchor.map_or(Ok(None), |anchor| {
            window_parent(anchor.get_value(ctx, "window"), "popup anchor window")
        })?,
        value => window_parent(value, "popup parent")?,
    };
    let anchor_x = anchor.map_or(Ok(0), |anchor| window_i32(ctx, anchor, "x", 0))?;
    let anchor_y = anchor.map_or(Ok(0), |anchor| window_i32(ctx, anchor, "y", 0))?;
    let anchor_width = anchor.map_or(Ok(1), |anchor| window_i32(ctx, anchor, "width", 1))?;
    let anchor_height = anchor.map_or(Ok(1), |anchor| window_i32(ctx, anchor, "height", 1))?;
    if anchor_width <= 0 || anchor_height <= 0 {
        return Err("popup anchor width and height must be positive".into());
    }
    let anchor_edge = table_string(ctx, options, "anchor_edge", "bottom_left")?;
    let gravity = table_string(ctx, options, "gravity", "bottom_right")?;
    if !popup_position(&anchor_edge) || !popup_position(&gravity) {
        return Err("popup anchor_edge and gravity must be valid positions".into());
    }
    let constraints = match options.get_value(ctx, "constraints") {
        LuaValue::Nil => PopupConstraintConfig::default(),
        LuaValue::Table(constraints) => PopupConstraintConfig {
            slide_x: table_bool(ctx, constraints, "slide_x", true)?,
            slide_y: table_bool(ctx, constraints, "slide_y", true)?,
            flip_x: table_bool(ctx, constraints, "flip_x", true)?,
            flip_y: table_bool(ctx, constraints, "flip_y", true)?,
            resize_x: table_bool(ctx, constraints, "resize_x", false)?,
            resize_y: table_bool(ctx, constraints, "resize_y", false)?,
        },
        _ => return Err("popup constraints must be a table".into()),
    };
    let node_anchor = if let Some(anchor) = anchor {
        match anchor.get_value(ctx, "node") {
            LuaValue::Nil => None,
            LuaValue::UserData(node) => {
                let node = node
                    .downcast_static::<NodeToken>()
                    .map_err(|_| "popup anchor node must be a mold node".to_owned())?
                    .handle;
                let margin = window_i32(ctx, anchor, "margin", 0)?;
                Some(PopupNodeAnchor {
                    node,
                    x: window_i32(ctx, anchor, "x", 0)?,
                    y: window_i32(ctx, anchor, "y", 0)?,
                    width: window_optional_i32(ctx, anchor, "width")?,
                    height: window_optional_i32(ctx, anchor, "height")?,
                    margin_top: window_i32(ctx, anchor, "margin_top", margin)?,
                    margin_right: window_i32(ctx, anchor, "margin_right", margin)?,
                    margin_bottom: window_i32(ctx, anchor, "margin_bottom", margin)?,
                    margin_left: window_i32(ctx, anchor, "margin_left", margin)?,
                })
            }
            _ => return Err("popup anchor node must be a mold node".into()),
        }
    } else {
        None
    };
    Ok((
        root,
        visible,
        PopupSurfaceConfig {
            parent,
            anchor_x,
            anchor_y,
            anchor_width,
            anchor_height,
            width: window_u32(ctx, options, "width", 1)?,
            height: window_u32(ctx, options, "height", 1)?,
            anchor_edge,
            gravity,
            offset_x: window_i32(ctx, options, "offset_x", 0)?,
            offset_y: window_i32(ctx, options, "offset_y", 0)?,
            constraints,
            grab_focus: table_bool(ctx, options, "grab_focus", false)?,
        },
        node_anchor,
    ))
}

pub(crate) fn parse_floating_surface<'gc>(
    ctx: Context<'gc>,
    options: Table<'gc>,
) -> Result<(NodeHandle, bool, FloatingSurfaceConfig), String> {
    let root = window_root(ctx, options)?;
    let visible = table_bool(ctx, options, "visible", false)?;
    let parent = window_parent(options.get_value(ctx, "parent"), "floating parent")?;
    let title = table_string(ctx, options, "title", "mold")?;
    let app_id = table_string(ctx, options, "app_id", "mold")?;
    if title.len() > 4096 || app_id.len() > 4096 || app_id.contains('\0') || title.contains('\0') {
        return Err("floating title and app_id must be at most 4096 bytes without NUL".into());
    }
    let minimum_width = window_u32(ctx, options, "minimum_width", 1)?;
    let minimum_height = window_u32(ctx, options, "minimum_height", 1)?;
    let maximum_width = window_optional_u32(ctx, options, "maximum_width")?;
    let maximum_height = window_optional_u32(ctx, options, "maximum_height")?;
    if maximum_width.is_some_and(|maximum| maximum < minimum_width)
        || maximum_height.is_some_and(|maximum| maximum < minimum_height)
    {
        return Err("floating maximum size cannot be smaller than its minimum size".into());
    }
    Ok((
        root,
        visible,
        FloatingSurfaceConfig {
            parent,
            width: window_u32(ctx, options, "width", 640)?,
            height: window_u32(ctx, options, "height", 480)?,
            minimum_width,
            minimum_height,
            maximum_width,
            maximum_height,
            title,
            app_id,
            minimized: table_bool(ctx, options, "minimized", false)?,
            maximized: table_bool(ctx, options, "maximized", false)?,
            fullscreen: table_bool(ctx, options, "fullscreen", false)?,
        },
    ))
}

pub(crate) fn parse_region<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    depth: usize,
) -> Result<Region, String> {
    if depth >= 64 {
        return Err("region nesting exceeds 64 levels".into());
    }
    let integer = |field: &str, default: i32| match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => {
            i32::try_from(value).map_err(|_| format!("region {field} must fit i32"))
        }
        _ => Err(format!("region {field} must be an integer")),
    };
    let x = integer("x", 0)?;
    let y = integer("y", 0)?;
    let width = integer("width", 0)?;
    let height = integer("height", 0)?;
    if width < 0 || height < 0 {
        return Err("region width and height cannot be negative".into());
    }
    let number = |field: &str, default: f32| match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => Ok(value as f32),
        LuaValue::Number(value) => Ok(value as f32),
        _ => Err(format!("region {field} must be a number")),
    };
    // Radii are fractional now, because the renderer's are: a corner and a
    // clickable corner are the same number, and rounding one of them to a whole
    // pixel is what made them two.
    let radius = number("radius", 0.0)?;
    let corner = |field: &str| {
        let value = number(field, radius)?;
        (value >= 0.0)
            .then_some(value)
            .ok_or_else(|| format!("region {field} cannot be negative"))
    };
    // The same vocabulary the renderer draws with, so a star-shaped node can be
    // given a star-shaped clickable area instead of the rectangle around it.
    let shape = match table.get_value(ctx, "shape") {
        LuaValue::Nil => RegionShape::Box,
        LuaValue::String(value) => RegionShape::parse(&value.display_lossy().to_string())
            .ok_or_else(|| format!("region shape {} is not a shape", value.display_lossy()))?,
        _ => return Err("region shape must be a string".into()),
    };
    let defaults = ShapeParams::default();
    let params = ShapeParams {
        radii: [
            corner("top_left_radius")?,
            corner("top_right_radius")?,
            corner("bottom_right_radius")?,
            corner("bottom_left_radius")?,
        ],
        points: number("points", defaults.points)?,
        inner_radius: number("inner_radius", defaults.inner_radius)?,
        thickness: number("thickness", defaults.thickness)?,
        angle: number("angle", defaults.angle)?,
    };
    let operation = match table.get_value(ctx, "intersection") {
        LuaValue::Nil => RegionOperation::Union,
        LuaValue::String(value) => RegionOperation::parse(&value.display_lossy().to_string())
            .ok_or_else(|| {
                format!(
                    "region intersection {} is not an operation",
                    value.display_lossy()
                )
            })?,
        _ => return Err("region intersection must be a string".into()),
    };
    let mut children = Vec::new();
    match table.get_value(ctx, "regions") {
        LuaValue::Nil => {}
        LuaValue::Table(values) => {
            let mut ordered = Vec::new();
            for (key, value) in values.iter(ctx) {
                let LuaValue::Integer(index) = key else {
                    return Err("region child keys must be integers".into());
                };
                let LuaValue::Table(value) = value else {
                    return Err("region children must be tables".into());
                };
                ordered.push((index, value));
            }
            if ordered.len() > 256 {
                return Err("region exceeds 256 direct children".into());
            }
            ordered.sort_by_key(|(index, _)| *index);
            for (offset, (index, value)) in ordered.into_iter().enumerate() {
                if index != offset as i64 + 1 {
                    return Err("region children must be a dense sequence".into());
                }
                children.push(parse_region(ctx, value, depth + 1)?);
            }
        }
        _ => return Err("region regions must be a table".into()),
    }
    Ok(Region {
        rect: RegionRect {
            x,
            y,
            width,
            height,
        },
        shape,
        params,
        operation,
        children,
    })
}

pub(crate) fn region_to_lua<'gc>(ctx: Context<'gc>, region: &Region) -> Table<'gc> {
    let table = Table::new(&ctx);
    table.set_field(ctx, "x", i64::from(region.rect.x));
    table.set_field(ctx, "y", i64::from(region.rect.y));
    table.set_field(ctx, "width", i64::from(region.rect.width));
    table.set_field(ctx, "height", i64::from(region.rect.height));
    table.set_field(ctx, "intersection", region.operation.name());
    table.set_field(ctx, "shape", region.shape.name());
    // Every family's parameters, read back whatever the shape is: a config that
    // round-trips a region gets the same region, and which of these a given
    // family reads is the distance function's business, not this table's.
    let [top_left, top_right, bottom_right, bottom_left] = region.params.radii;
    table.set_field(ctx, "top_left_radius", f64::from(top_left));
    table.set_field(ctx, "top_right_radius", f64::from(top_right));
    table.set_field(ctx, "bottom_right_radius", f64::from(bottom_right));
    table.set_field(ctx, "bottom_left_radius", f64::from(bottom_left));
    table.set_field(ctx, "points", f64::from(region.params.points));
    table.set_field(ctx, "inner_radius", f64::from(region.params.inner_radius));
    table.set_field(ctx, "thickness", f64::from(region.params.thickness));
    table.set_field(ctx, "angle", f64::from(region.params.angle));
    let children = Table::new(&ctx);
    for (index, child) in region.children.iter().enumerate() {
        children
            .set(ctx, index as i64 + 1, region_to_lua(ctx, child))
            .expect("region child list accepts integer keys");
    }
    table.set_field(ctx, "regions", children);
    table
}
