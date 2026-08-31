use luna::{Callback, CallbackReturn, Context, Table, UserRef};
use std::cell::RefCell;
use std::rc::Rc;

use morf_layout::Geometry;
use morf_scene::NodeHandle;

use crate::{runtime_helpers::*, scene_bindings::*, state::*, surface_types::*};

pub(crate) fn checked_window_node(
    state: &ReactiveState,
    surface: u64,
    node: NodeHandle,
) -> Result<NodeHandle, HostError> {
    let surface = state
        .window_surfaces
        .get(&surface)
        .ok_or_else(|| HostError("window surface is stale".into()))?;
    if !scene_node_in_subtree(&state.scene, surface.root, node) {
        return Err(HostError(
            "item does not belong to the window surface".into(),
        ));
    }
    Ok(node)
}

pub(crate) fn point_table(ctx: Context<'_>, point: (f64, f64)) -> Table<'_> {
    let result = Table::new(&ctx);
    result.set_field(ctx, "x", point.0);
    result.set_field(ctx, "y", point.1);
    result
}

pub(crate) fn geometry_table(ctx: Context<'_>, geometry: Geometry) -> Table<'_> {
    let result = point_table(ctx, (geometry.x, geometry.y));
    result.set_field(ctx, "width", geometry.width);
    result.set_field(ctx, "height", geometry.height);
    result
}

pub(crate) fn window_item_position_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, node): (UserRef<WindowSurfaceToken>, UserRef<NodeToken>) =
            stack.consume(ctx)?;
        let state = state.borrow();
        let node = checked_window_node(&state, surface.id, node.handle)?;
        let point = state
            .transform_tracker
            .map_from_node(&state.scene, node, 0.0, 0.0)
            .map_err(|error| HostError(error.to_string()))?
            .ok_or_else(|| HostError("item has not been laid out".into()))?;
        stack.replace(ctx, point_table(ctx, point));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn window_item_rect_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, node): (UserRef<WindowSurfaceToken>, UserRef<NodeToken>) =
            stack.consume(ctx)?;
        let state = state.borrow();
        let node = checked_window_node(&state, surface.id, node.handle)?;
        let size = state
            .transform_tracker
            .geometry(node)
            .ok_or_else(|| HostError("item has not been laid out".into()))?;
        let geometry = state
            .transform_tracker
            .map_rect_from_node(
                &state.scene,
                node,
                Geometry {
                    x: 0.0,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                },
            )
            .map_err(|error| HostError(error.to_string()))?
            .ok_or_else(|| HostError("item has not been laid out".into()))?;
        stack.replace(ctx, geometry_table(ctx, geometry));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn window_map_from_item_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, node, x, y): (UserRef<WindowSurfaceToken>, UserRef<NodeToken>, f64, f64) =
            stack.consume(ctx)?;
        if !x.is_finite() || !y.is_finite() {
            return Err(HostError("mapped point must be finite".into()).into());
        }
        let state = state.borrow();
        let node = checked_window_node(&state, surface.id, node.handle)?;
        let point = state
            .transform_tracker
            .map_from_node(&state.scene, node, x, y)
            .map_err(|error| HostError(error.to_string()))?
            .ok_or_else(|| HostError("item has not been laid out".into()))?;
        stack.replace(ctx, point_table(ctx, point));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn window_map_rect_from_item_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, node, x, y, width, height): WindowMapRectArgs<'gc> = stack.consume(ctx)?;
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
        {
            return Err(HostError("mapped rectangle must be finite and nonnegative".into()).into());
        }
        let state = state.borrow();
        let node = checked_window_node(&state, surface.id, node.handle)?;
        let geometry = state
            .transform_tracker
            .map_rect_from_node(
                &state.scene,
                node,
                Geometry {
                    x,
                    y,
                    width,
                    height,
                },
            )
            .map_err(|error| HostError(error.to_string()))?
            .ok_or_else(|| HostError("item has not been laid out".into()))?;
        stack.replace(ctx, geometry_table(ctx, geometry));
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn floating_state_method<'gc>(
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
            let WindowSurfaceKind::Floating(config) = &mut surface.kind else {
                return Err(
                    HostError(format!("{property} is only valid for floating windows")).into(),
                );
            };
            let current = match property {
                "minimized" => &mut config.minimized,
                "maximized" => &mut config.maximized,
                "fullscreen" => &mut config.fullscreen,
                _ => unreachable!(),
            };
            let changed = value.is_some_and(|value| *current != value);
            if let Some(value) = value {
                *current = value;
            }
            (*current, changed)
        };
        if changed {
            state.window_surfaces_changed = true;
        }
        stack.replace(ctx, current);
        Ok(CallbackReturn::Return)
    })
}

pub(crate) fn floating_string_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    property: &'static str,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, value): (UserRef<WindowSurfaceToken>, Option<String>) = stack.consume(ctx)?;
        if value
            .as_ref()
            .is_some_and(|value| value.len() > 4_096 || value.as_bytes().contains(&0))
        {
            return Err(HostError(format!("floating {property} is invalid")).into());
        }
        let mut state = state.borrow_mut();
        let (current, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Floating(config) = &mut surface.kind else {
                return Err(
                    HostError(format!("{property} is only valid for floating windows")).into(),
                );
            };
            let current = match property {
                "title" => &mut config.title,
                "app_id" => &mut config.app_id,
                _ => unreachable!(),
            };
            let changed = value.as_ref().is_some_and(|value| current != value);
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

pub(crate) fn floating_size_method<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    property: &'static str,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, width, height): (UserRef<WindowSurfaceToken>, Option<i64>, Option<i64>) =
            stack.consume(ctx)?;
        if width.is_some() != height.is_some() {
            return Err(HostError("floating size requires both width and height".into()).into());
        }
        let values = match (width, height) {
            (Some(width), Some(height)) => Some((
                u32::try_from(width).map_err(|_| HostError("width must fit u32".into()))?,
                u32::try_from(height).map_err(|_| HostError("height must fit u32".into()))?,
            )),
            _ => None,
        };
        if values.is_some_and(|(width, height)| {
            width > 16_384
                || height > 16_384
                || (property != "maximum_size" && (width == 0 || height == 0))
        }) {
            return Err(HostError("floating size is outside 1..16384".into()).into());
        }
        let mut state = state.borrow_mut();
        let (width, height, changed) = {
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            let WindowSurfaceKind::Floating(config) = &mut surface.kind else {
                return Err(
                    HostError(format!("{property} is only valid for floating windows")).into(),
                );
            };
            let before = match property {
                "size" => (config.width, config.height),
                "minimum_size" => (config.minimum_width, config.minimum_height),
                "maximum_size" => (
                    config.maximum_width.unwrap_or_default(),
                    config.maximum_height.unwrap_or_default(),
                ),
                _ => unreachable!(),
            };
            if let Some((width, height)) = values {
                match property {
                    "size" => (config.width, config.height) = (width, height),
                    "minimum_size" => {
                        if config.maximum_width.is_some_and(|maximum| width > maximum)
                            || config
                                .maximum_height
                                .is_some_and(|maximum| height > maximum)
                        {
                            return Err(HostError("floating minimum exceeds maximum".into()).into());
                        }
                        (config.minimum_width, config.minimum_height) = (width, height);
                    }
                    "maximum_size" => {
                        if (width != 0 && width < config.minimum_width)
                            || (height != 0 && height < config.minimum_height)
                        {
                            return Err(HostError(
                                "floating maximum is smaller than minimum".into(),
                            )
                            .into());
                        }
                        config.maximum_width = (width != 0).then_some(width);
                        config.maximum_height = (height != 0).then_some(height);
                    }
                    _ => unreachable!(),
                }
            }
            let after = match property {
                "size" => (config.width, config.height),
                "minimum_size" => (config.minimum_width, config.minimum_height),
                "maximum_size" => (
                    config.maximum_width.unwrap_or_default(),
                    config.maximum_height.unwrap_or_default(),
                ),
                _ => unreachable!(),
            };
            (after.0, after.1, before != after)
        };
        state.window_surfaces_changed |= changed;
        let result = Table::new(&ctx);
        result.set_field(ctx, "width", i64::from(width));
        result.set_field(ctx, "height", i64::from(height));
        stack.replace(ctx, result);
        Ok(CallbackReturn::Return)
    })
}
