use morf_lua::{
    LayerSurfaceConfig, Runtime, Toplevel, WindowSurfaceConfig, WindowSurfaceKind, Workspace,
};
use morf_render::{RenderEngine, WgpuBackend};
use morf_scene::NodeHandle;
use morf_wayland::{
    BarConfig, KeyboardFocus, LayerAnchors, LayerClient, PRIMARY_LAYER, ShellLayer, ToplevelAction,
    physical_size,
};
use std::collections::{HashMap, HashSet};

use crate::{paint::*, services::*, surfaces::*};

/// Hands every request a configuration has queued to the compositor.
///
/// One list, called at each of the three points that need it. It used to be
/// written out six times in three different subsets, and the subsets did not
/// agree: the startup copies left out clipboard requests, and the copies that
/// run *after* the event loop — the ones that exist so a request made by an
/// input handler reaches the compositor in the same frame — left out output
/// power. So a key handler that turned a display off was a frame late, every
/// time, for no reason anybody chose.
pub(crate) fn apply_service_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    apply_output_power_requests(runtime, client);
    apply_clipboard_requests(runtime, client);
    apply_screencopy_requests(runtime, client);
    apply_virtual_keyboard_requests(runtime, client);
    apply_input_method_requests(runtime, client);
    apply_text_input_requests(runtime, client);
    publish_windows(runtime, client);
    publish_workspaces(runtime, client);
    apply_workspace_activation(runtime, client);
    apply_toplevel_requests(runtime, client);
}

/// Hands the compositor's window list to the configuration, when it changed.
///
/// Only when it changed. The list is rebuilt from scratch each time — cheap for
/// a dozen windows, and cheaper than a diff that would have to decide what
/// identity means for a renamed window — but doing it every frame would rebuild
/// a dozen Lua tables sixty times a second to say nothing new.
fn publish_windows(runtime: &mut Runtime, client: &mut LayerClient) {
    if !client.take_toplevels_changed() {
        return;
    }
    let windows: Vec<Toplevel> = client
        .toplevels()
        .into_iter()
        .map(|window| Toplevel {
            identifier: window.identifier,
            title: window.title,
            app_id: window.app_id,
            activated: window.activated,
            maximized: window.maximized,
            minimized: window.minimized,
            fullscreen: window.fullscreen,
            controllable: window.controllable,
        })
        .collect();
    runtime.set_windows(&windows);
}

/// Hands the compositor's workspace list to the configuration, when it changed.
///
/// The same contract as the window list above, and rebuilt the same way for the
/// same reason.
fn publish_workspaces(runtime: &mut Runtime, client: &mut LayerClient) {
    if !client.take_workspaces_changed() {
        return;
    }
    let workspaces: Vec<Workspace> = client
        .workspaces()
        .into_iter()
        .map(|workspace| Workspace {
            key: workspace.key,
            id: workspace.id,
            name: workspace.name,
            coordinates: workspace.coordinates,
            output: workspace.output,
            active: workspace.active,
            urgent: workspace.urgent,
            hidden: workspace.hidden,
            activatable: workspace.activatable,
        })
        .collect();
    runtime.set_workspaces(&workspaces);
}

/// Acts on other windows, if the configuration asked.
fn apply_toplevel_requests(runtime: &mut Runtime, client: &mut LayerClient) {
    for request in runtime.take_toplevel_requests() {
        let action = match request.action.as_str() {
            "activate" => ToplevelAction::Activate,
            "close" => ToplevelAction::Close,
            "set_maximized" => ToplevelAction::Maximized(request.value),
            "set_minimized" => ToplevelAction::Minimized(request.value),
            "set_fullscreen" => ToplevelAction::Fullscreen(request.value),
            _ => continue,
        };
        client.control_toplevel(&request.identifier, action);
    }
}

/// Switches workspace, if the configuration asked.
fn apply_workspace_activation(runtime: &mut Runtime, client: &mut LayerClient) {
    if let Some(id) = runtime.take_workspace_activation() {
        client.activate_workspace(&id);
    }
}

/// First identifier of the four internal edge reservers.
///
/// Reservers are engine-owned rather than configured, so they take identifiers
/// from the top of the range where no Lua window surface can reach them.
pub(crate) const RESERVE_LAYER_BASE: u64 = u64::MAX - 3;

/// Wayland identifier of the layer surface backing one Lua window surface.
///
/// Window surface identifiers start at zero, which the shell's own surface
/// already owns, so every configured layer surface sits one above its Lua id.
pub(crate) fn window_layer_id(id: u64) -> u64 {
    id.wrapping_add(1)
}

/// Inverse of [`window_layer_id`], or `None` for engine-owned surfaces.
pub(crate) fn window_surface_id(layer: u64) -> Option<u64> {
    (layer != PRIMARY_LAYER && layer < RESERVE_LAYER_BASE).then(|| layer - 1)
}

/// Builds the configuration for one single-edge reserver surface.
///
/// The surface draws nothing and accepts no input. Its whole purpose is the
/// exclusive zone: a layer surface anchored to exactly one edge leaves the
/// compositor no doubt about which edge to shrink, which is what keeps tiled
/// windows out from under a frame drawn on all four edges at once.
pub(crate) fn reserve_bar_config(edge: &str, thickness: u32, output: &str) -> BarConfig {
    BarConfig {
        namespace: format!("morf-reserve-{edge}"),
        width: 1,
        height: 1,
        exclusive_zone: i32::try_from(thickness).unwrap_or(i32::MAX),
        output: Some(output.to_owned()),
        anchors: LayerAnchors {
            top: edge == "top",
            right: edge == "right",
            bottom: edge == "bottom",
            left: edge == "left",
        },
        margin_top: 0,
        margin_right: 0,
        margin_bottom: 0,
        margin_left: 0,
        layer: ShellLayer::Bottom,
        keyboard_focus: KeyboardFocus::None,
    }
}

/// Brings the reserver surfaces up to date with `morf.surface.reserve`.
///
/// A reserver holds an edge of the output away from the windows, and the only
/// thing that changes as the border animates is its exclusive zone — one of the
/// fields wlr-layer-shell lets a mapped surface change. Configured layer
/// surfaces already sort a live geometry change from a genuine rebuild for
/// exactly this reason; reservers ignored all of it and recreated the zwlr
/// surface, the wl_surface, the fractional scale and the viewport every time
/// the number moved, which is an unmap and a remap the compositor has to
/// rearrange around.
pub(crate) fn open_reserve_layers(
    client: &mut LayerClient,
    config: &LayerSurfaceConfig,
    output: &str,
) -> Result<(), String> {
    for (index, (edge, thickness)) in config.reserve.edges().into_iter().enumerate() {
        let id = RESERVE_LAYER_BASE + index as u64;
        if thickness == 0 {
            client.close_layer(id);
            continue;
        }
        let reserve = reserve_bar_config(edge, thickness, output);
        if client.layer_surface(id).is_some() {
            // Already mapped: move it rather than rebuild it.
            client
                .set_layer_geometry(id, &reserve)
                .map_err(|error| error.to_string())?;
            client.commit_layer(id);
            continue;
        }
        client
            .open_layer(id, reserve)
            .map_err(|error| error.to_string())?;
        client.set_layer_input_region(id, Some(&[]));
        // A reserver draws nothing, but it still has to map: a compositor
        // computes the output's usable area from the layer surfaces it
        // arranges, and an unmapped one is skipped, so its exclusive zone would
        // reserve nothing at all.
        client
            .map_layer_blank(id)
            .map_err(|error| error.to_string())?;
        client.commit_layer(id);
    }
    Ok(())
}

/// How a configured layer surface reaches its new configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayerUpdate {
    /// The surface already matches; the compositor hears nothing.
    None,
    /// Geometry moved, and every part of it may change on a live surface.
    Geometry,
    /// Something fixed at creation moved, so the surface has to be rebuilt.
    Recreate,
}

/// Decides how one layer surface is brought up to date.
///
/// wlr-layer-shell lets a mapped surface change its size, anchors, margins,
/// exclusive zone and keyboard interactivity, and nothing else: namespace,
/// layer and output are fixed when the surface is created. Sorting the two
/// apart is what keeps an animated margin or a growing border from destroying
/// the zwlr surface, the wl_surface, the fractional scale, the viewport and the
/// renderer once per frame — a visible unmap and remap for a geometry change
/// the protocol supports outright.
pub(crate) fn layer_update(
    current: Option<&LayerSurfaceConfig>,
    next: &LayerSurfaceConfig,
) -> LayerUpdate {
    let Some(current) = current else {
        return LayerUpdate::Recreate;
    };
    if current.namespace != next.namespace || current.layer != next.layer {
        return LayerUpdate::Recreate;
    }
    if current == next {
        LayerUpdate::None
    } else {
        LayerUpdate::Geometry
    }
}

/// Opens, updates, and closes the layer surfaces a configuration asks for.
pub(crate) fn sync_layer_surfaces(
    client: &mut LayerClient,
    output: &str,
    desired: &[&WindowSurfaceConfig],
    layers: &mut HashMap<u64, AuxiliarySurface>,
) -> Result<bool, String> {
    let mut resumed = false;
    let live = desired
        .iter()
        .map(|surface| surface.id)
        .collect::<HashSet<_>>();
    let mut stale = layers
        .keys()
        .filter(|id| !live.contains(id))
        .copied()
        .collect::<Vec<_>>();
    stale.sort_unstable_by(|a, b| b.cmp(a));
    for id in stale {
        client.close_layer(window_layer_id(id));
        layers.remove(&id);
    }
    for surface in desired {
        let id = surface.id;
        let WindowSurfaceKind::Layer(config) = &surface.kind else {
            unreachable!("only layer surfaces reach the layer sync");
        };
        let update = layer_update(
            layers
                .get(&id)
                .and_then(|current| current.layer_config.as_ref()),
            config,
        );
        if update == LayerUpdate::Recreate {
            client
                .open_layer(window_layer_id(id), runtime_bar_config(config, output)?)
                .map_err(|error| error.to_string())?;
            layers.insert(
                id,
                AuxiliarySurface {
                    id,
                    root: surface.root,
                    updates_enabled: surface.updates_enabled,
                    width: config.width.max(1),
                    height: config.height.max(1),
                    renderer: None,
                    layout: None,
                    popup_config: None,
                    floating_config: None,
                    layer_config: Some(config.clone()),
                    needs_paint: true,
                },
            );
            continue;
        }
        if update == LayerUpdate::Geometry {
            client
                .set_layer_geometry(window_layer_id(id), &runtime_bar_config(config, output)?)
                .map_err(|error| error.to_string())?;
        }
        let Some(current) = layers.get_mut(&id) else {
            continue;
        };
        if update == LayerUpdate::Geometry {
            // The mask travels in the same configuration, and it is applied
            // when the surface paints, so a geometry change owes one repaint
            // even before the compositor answers with a configure.
            current.layer_config = Some(config.clone());
            current.needs_paint = true;
            resumed = true;
        }
        resumed |= !current.updates_enabled && surface.updates_enabled;
        let moved = current.root != surface.root;
        current.root = surface.root;
        current.updates_enabled = surface.updates_enabled;
        // Only when the tree it lays out actually changed. `CachedLayout`
        // already re-checks the revision, the size and the scale, so
        // clearing it here on every sync threw away a valid layout — and
        // with an anchored popup, which re-syncs whenever its anchor moves,
        // that was every frame.
        if moved {
            current.layout = None;
        }
    }
    Ok(resumed)
}

/// Applies one compositor configure to a configured layer surface.
pub(crate) fn layer_surface_configure(
    runtime: &Runtime,
    client: &LayerClient,
    state: &mut SurfaceEventState,
    layer: u64,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let Some(id) = window_surface_id(layer) else {
        return Ok(());
    };
    let Some(surface) = state.layer_surfaces.get_mut(&id) else {
        return Ok(());
    };
    let initial = surface.renderer.is_none();
    surface.width = width.max(1);
    surface.height = height.max(1);
    let scale = client.layer_scale_120(layer).unwrap_or(120);
    let (physical_width, physical_height) = physical_size((surface.width, surface.height), scale);
    if let Some(renderer) = &mut surface.renderer {
        renderer.resize(physical_width, physical_height);
    } else {
        let target = client
            .layer_window_target(layer)
            .ok_or_else(|| "configured layer surface disappeared".to_owned())?;
        let backend = pollster::block_on(WgpuBackend::new_surface(
            target,
            physical_width,
            physical_height,
        ))
        .map_err(|error| error.to_string())?;
        surface.renderer = Some(RenderEngine::new(backend));
    }
    if initial || surface.updates_enabled {
        paint_layer_surface(runtime, client, surface)?;
    }
    Ok(())
}

/// Resizes one configured layer surface after its preferred scale changed.
pub(crate) fn layer_surface_scale(
    runtime: &Runtime,
    client: &LayerClient,
    state: &mut SurfaceEventState,
    layer: u64,
) -> Result<(), String> {
    let Some(id) = window_surface_id(layer) else {
        return Ok(());
    };
    let Some(surface) = state.layer_surfaces.get_mut(&id) else {
        return Ok(());
    };
    let scale = client.layer_scale_120(layer).unwrap_or(120);
    if let Some(renderer) = &mut surface.renderer {
        let (width, height) = physical_size((surface.width, surface.height), scale);
        renderer.resize(width, height);
    }
    // And then draw into it. Resizing the swapchain without repainting leaves
    // the surface showing whatever the old buffer held, at the new size, until
    // something unrelated happens to mark it dirty — which on a static bar can
    // be a very long time. Its two siblings, the primary-layer scale change and
    // the configure for this same surface, both repaint here.
    if surface.updates_enabled {
        paint_layer_surface(runtime, client, surface)?;
    }
    Ok(())
}

/// Paints one configured layer surface when the compositor permits a frame.
pub(crate) fn layer_surface_frame(
    runtime: &Runtime,
    client: &LayerClient,
    state: &mut SurfaceEventState,
    layer: u64,
) -> Result<(), String> {
    let Some(id) = window_surface_id(layer) else {
        return Ok(());
    };
    let Some(surface) = state
        .layer_surfaces
        .get_mut(&id)
        .filter(|surface| surface.updates_enabled && surface.needs_paint)
    else {
        return Ok(());
    };
    paint_layer_surface(runtime, client, surface)
}

/// Drops one configured layer surface the compositor closed.
pub(crate) fn layer_surface_closed(
    runtime: &mut Runtime,
    client: &mut LayerClient,
    state: &mut SurfaceEventState,
    layer: u64,
) {
    client.close_layer(layer);
    let Some(id) = window_surface_id(layer) else {
        return;
    };
    if state.layer_surfaces.remove(&id).is_some() {
        runtime.set_window_surface_visible(id, false);
    }
}

/// Routes one key press into a surface subtree, keeping its focus.
///
/// Tab moves to the next focusable node and off the end again; anything else
/// goes to whatever holds focus, or to the first thing that can take it.
///
/// This exists as a function because the lock screen had its own copy that did
/// neither — no traversal and no persistence, so every key went to the first
/// focusable node in the tree. On the one surface whose entire purpose is to
/// accept a password, a second field could not be reached at all.
///
/// Returns whether anything changed enough to want a repaint.
pub(crate) fn dispatch_key_in_subtree(
    runtime: &mut Runtime,
    root: NodeHandle,
    focused: &mut Option<NodeHandle>,
    keysym: u32,
    text: Option<&str>,
) -> bool {
    const TAB: u32 = 0xff09;
    let current = focused.filter(|node| runtime.node_in_subtree(root, *node));
    if keysym == TAB {
        *focused = runtime.next_key_target_in(root, current);
        return true;
    }
    let Some(node) = current.or_else(|| runtime.first_key_target_in(root)) else {
        return false;
    };
    *focused = Some(node);
    runtime.dispatch_key_event(node, keysym, text)
}
