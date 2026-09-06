use luna::{Callback, CallbackReturn, Context, Table, UserData, UserRef};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    layer_parse::*, process_helpers::*, runtime_helpers::*, scene_bindings::*, state::*,
    surface_types::*, table_menu::*, window_geometry::*, window_methods::*, window_parse::*,
};

pub(crate) fn install_module_api<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    morf: Table<'gc>,
) -> (Table<'gc>, Table<'gc>, Table<'gc>) {
    let core = Table::new(&ctx);
    for name in [
        "env",
        "font_families",
        "process_id",
        "executable",
        "args",
        "options",
        "operands",
        "version",
        "instance_id",
        "shell_id",
        "app_id",
        "launch_time_ms",
        "shell_dir",
        "shell_path",
        "config_path",
        "data_dir",
        "data_path",
        "state_dir",
        "state_path",
        "cache_dir",
        "cache_path",
        "has_version",
        "reload",
        "on_reload_completed",
        "on_reload_failed",
        "watch_files",
        "working_directory",
        "elapsed_timer",
        "system_clock",
        "easing_curve",
        "color_quantizer",
        "icon_path",
        "has_icon",
        "exec_detached",
        "signal",
        "theme",
        "prefers",
        "reloadable",
        "persistent",
        "scope",
        "retainable",
        "retain_lock",
        "transform_watcher",
        "effect",
        "clock",
        "timer",
        "screens",
        "variants",
        "list_model",
        "virtual_list",
        "sync_view",
        "flickable",
        "transition_parent",
        "desktop_entries",
        "session_paths",
        "menu",
    ] {
        core.set(ctx, name, morf.get_value(ctx, name))
            .expect("core module accepts native fields");
    }
    let io = Table::new(&ctx);
    for name in [
        "process",
        "process_view",
        "file",
        "file_view",
        "socket_server",
        "socket",
        "line_parser",
        "split_parser",
        "stream_collector",
        "json",
    ] {
        io.set(ctx, name, morf.get_value(ctx, name))
            .expect("IO module accepts native fields");
    }
    let region = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let options: Table = stack.consume(ctx)?;
        let region = parse_region(ctx, options, 0).map_err(HostError)?;
        stack.replace(ctx, region_to_lua(ctx, &region));
        Ok(CallbackReturn::Return)
    });
    let window_visible = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
            let visible = state
                .borrow()
                .window_surfaces
                .get(&surface.id)
                .map(|surface| surface.visible)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            stack.replace(ctx, visible);
            Ok(CallbackReturn::Return)
        }
    });
    let window_open = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
            let mut state = state.borrow_mut();
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            if !surface.visible {
                surface.visible = true;
                state.window_surfaces_changed = true;
            }
            Ok(CallbackReturn::Return)
        }
    });
    let window_close = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
            let mut state = state.borrow_mut();
            let surface = state
                .window_surfaces
                .get_mut(&surface.id)
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            if surface.visible {
                surface.visible = false;
                state.window_surfaces_changed = true;
            }
            Ok(CallbackReturn::Return)
        }
    });
    let window_kind = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        move |ctx, _, mut stack| {
            let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
            let kind = state
                .borrow()
                .window_surfaces
                .get(&surface.id)
                .map(|surface| match surface.kind {
                    WindowSurfaceKind::Popup(_) => "popup",
                    WindowSurfaceKind::Floating(_) => "floating",
                    WindowSurfaceKind::Layer(_) => "layer",
                })
                .ok_or_else(|| HostError("window surface is stale".into()))?;
            stack.replace(ctx, kind);
            Ok(CallbackReturn::Return)
        }
    });
    let window_methods = Table::new(&ctx);
    window_methods.set_field(ctx, "visible", window_visible);
    // One spelling. `set_visible(node, bool)` said exactly what these two say
    // and wrote the same field; a configuration should not have to know which
    // of two names the engine prefers.
    window_methods.set_field(ctx, "open", window_open);
    window_methods.set_field(ctx, "close", window_close);
    window_methods.set_field(ctx, "kind", window_kind);
    window_methods.set_field(
        ctx,
        "updates_enabled",
        window_updates_enabled_method(ctx, Rc::clone(&state)),
    );
    for property in ["minimized", "maximized", "fullscreen"] {
        window_methods.set_field(
            ctx,
            property,
            floating_state_method(ctx, Rc::clone(&state), property),
        );
    }
    for property in ["title", "app_id"] {
        window_methods.set_field(
            ctx,
            property,
            floating_string_method(ctx, Rc::clone(&state), property),
        );
    }
    window_methods.set_field(ctx, "size", window_size_method(ctx, Rc::clone(&state)));
    for property in ["minimum_size", "maximum_size"] {
        window_methods.set_field(
            ctx,
            property,
            floating_size_method(ctx, Rc::clone(&state), property),
        );
    }
    window_methods.set_field(
        ctx,
        "grab_focus",
        popup_bool_method(ctx, Rc::clone(&state), "grab_focus"),
    );
    for property in ["anchor_edge", "gravity"] {
        window_methods.set_field(
            ctx,
            property,
            popup_string_method(ctx, Rc::clone(&state), property),
        );
    }
    window_methods.set_field(
        ctx,
        "anchor_rect",
        popup_anchor_rect_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(ctx, "offset", popup_offset_method(ctx, Rc::clone(&state)));
    window_methods.set_field(
        ctx,
        "constraints",
        popup_constraints_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "parent_id",
        window_parent_id_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "set_parent",
        window_set_parent_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "item_position",
        window_item_position_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "item_rect",
        window_item_rect_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "map_from_item",
        window_map_from_item_method(ctx, Rc::clone(&state)),
    );
    window_methods.set_field(
        ctx,
        "map_rect_from_item",
        window_map_rect_from_item_method(ctx, Rc::clone(&state)),
    );
    let move_state = Rc::clone(&state);
    let start_system_move = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let surface: UserRef<WindowSurfaceToken> = stack.consume(ctx)?;
        let mut state = move_state.borrow_mut();
        let valid = state
            .window_surfaces
            .get(&surface.id)
            .is_some_and(|surface| {
                surface.visible && matches!(surface.kind, WindowSurfaceKind::Floating(_))
            });
        if valid {
            state
                .window_surface_actions
                .push(WindowSurfaceAction::Move { id: surface.id });
        }
        stack.replace(ctx, valid);
        Ok(CallbackReturn::Return)
    });
    window_methods.set_field(ctx, "start_system_move", start_system_move);
    let resize_state = Rc::clone(&state);
    let start_system_resize = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (surface, edge): (UserRef<WindowSurfaceToken>, String) = stack.consume(ctx)?;
        if !matches!(
            edge.as_str(),
            "top"
                | "bottom"
                | "left"
                | "right"
                | "top_left"
                | "top_right"
                | "bottom_left"
                | "bottom_right"
        ) {
            return Err(HostError("invalid floating resize edge".into()).into());
        }
        let mut state = resize_state.borrow_mut();
        let valid = state
            .window_surfaces
            .get(&surface.id)
            .is_some_and(|surface| {
                surface.visible && matches!(surface.kind, WindowSurfaceKind::Floating(_))
            });
        if valid {
            state
                .window_surface_actions
                .push(WindowSurfaceAction::Resize {
                    id: surface.id,
                    edge,
                });
        }
        stack.replace(ctx, valid);
        Ok(CallbackReturn::Return)
    });
    window_methods.set_field(ctx, "start_system_resize", start_system_resize);
    let window_metatable = Table::new(&ctx);
    window_metatable.set_field(ctx, "__index", window_methods);
    let window_metatable = ctx.stash(window_metatable);
    let popup_surface = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let window_metatable = window_metatable.clone();
        move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let updates_enabled =
                table_bool(ctx, options, "updates_enabled", true).map_err(HostError)?;
            let (root, visible, config, node_anchor) =
                parse_popup_surface(ctx, options).map_err(HostError)?;
            {
                let state = state.borrow();
                state
                    .scene
                    .element(root)
                    .map_err(|error| HostError(error.to_string()))?;
                if let Some(parent) = config.parent {
                    let parent = state
                        .window_surfaces
                        .get(&parent)
                        .ok_or_else(|| HostError("popup parent is stale".into()))?;
                    if !matches!(parent.kind, WindowSurfaceKind::Floating(_)) {
                        return Err(
                            HostError("popup parent must be a floating surface".into()).into()
                        );
                    }
                    if let Some(anchor) = &node_anchor
                        && !scene_node_in_subtree(&state.scene, parent.root, anchor.node)
                    {
                        return Err(HostError(
                            "popup anchor node must belong to its parent surface".into(),
                        )
                        .into());
                    }
                }
            }
            let id = {
                let mut state = state.borrow_mut();
                let id = register_window_surface(
                    &mut state,
                    root,
                    visible,
                    updates_enabled,
                    WindowSurfaceKind::Popup(config),
                );
                if let Some(anchor) = node_anchor {
                    state
                        .scene
                        .element(anchor.node)
                        .map_err(|error| HostError(error.to_string()))?;
                    state.popup_node_anchors.insert(id, anchor);
                }
                id
            };
            let userdata = UserData::new_static(&ctx, WindowSurfaceToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&window_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    let floating_surface = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let window_metatable = window_metatable.clone();
        move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let updates_enabled =
                table_bool(ctx, options, "updates_enabled", true).map_err(HostError)?;
            let (root, visible, config) =
                parse_floating_surface(ctx, options).map_err(HostError)?;
            {
                let state = state.borrow();
                state
                    .scene
                    .element(root)
                    .map_err(|error| HostError(error.to_string()))?;
                if let Some(parent) = config.parent {
                    let parent = state
                        .window_surfaces
                        .get(&parent)
                        .ok_or_else(|| HostError("floating parent is stale".into()))?;
                    if !matches!(parent.kind, WindowSurfaceKind::Floating(_)) {
                        return Err(
                            HostError("floating parent must be a floating surface".into()).into(),
                        );
                    }
                }
            }
            let id = register_window_surface(
                &mut state.borrow_mut(),
                root,
                visible,
                updates_enabled,
                WindowSurfaceKind::Floating(config),
            );
            let userdata = UserData::new_static(&ctx, WindowSurfaceToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&window_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    let layer_surface = Callback::from_fn(&ctx, {
        let state = Rc::clone(&state);
        let window_metatable = window_metatable.clone();
        move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let updates_enabled =
                table_bool(ctx, options, "updates_enabled", true).map_err(HostError)?;
            let (root, visible, config) = parse_layer_surface(ctx, options).map_err(HostError)?;
            {
                let state = state.borrow();
                state
                    .scene
                    .element(root)
                    .map_err(|error| HostError(error.to_string()))?;
            }
            let id = register_window_surface(
                &mut state.borrow_mut(),
                root,
                visible,
                updates_enabled,
                WindowSurfaceKind::Layer(config),
            );
            let userdata = UserData::new_static(&ctx, WindowSurfaceToken { id });
            userdata.set_metatable(ctx, Some(ctx.fetch(&window_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        }
    });
    let window = Table::new(&ctx);
    window.set_field(ctx, "layer_surface", morf.get_value(ctx, "surface"));
    window.set_field(ctx, "region", region);
    window.set_field(ctx, "popup", popup_surface);
    window.set_field(ctx, "floating", floating_surface);
    window.set_field(ctx, "layer", layer_surface);
    morf.set_field(ctx, "core", core);
    morf.set_field(ctx, "io", io);
    morf.set_field(ctx, "window", window);
    (core, io, window)
}
