/// First identifier of the four internal edge reservers.
///
/// Reservers are engine-owned rather than configured, so they take identifiers
/// from the top of the range where no Lua window surface can reach them.
const RESERVE_LAYER_BASE: u64 = u64::MAX - 3;

/// Wayland identifier of the layer surface backing one Lua window surface.
///
/// Window surface identifiers start at zero, which the shell's own surface
/// already owns, so every configured layer surface sits one above its Lua id.
fn window_layer_id(id: u64) -> u64 {
    id.wrapping_add(1)
}

/// Inverse of [`window_layer_id`], or `None` for engine-owned surfaces.
fn window_surface_id(layer: u64) -> Option<u64> {
    (layer != PRIMARY_LAYER && layer < RESERVE_LAYER_BASE).then(|| layer - 1)
}

/// Builds the configuration for one single-edge reserver surface.
///
/// The surface draws nothing and accepts no input. Its whole purpose is the
/// exclusive zone: a layer surface anchored to exactly one edge leaves the
/// compositor no doubt about which edge to shrink, which is what keeps tiled
/// windows out from under a frame drawn on all four edges at once.
fn reserve_bar_config(edge: &str, thickness: u32, output: &str) -> BarConfig {
    BarConfig {
        namespace: format!("mold-reserve-{edge}"),
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

/// Opens one reserver surface per edge that `mold.surface.reserve` names.
fn open_reserve_layers(
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
        client
            .open_layer(id, reserve_bar_config(edge, thickness, output))
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
enum LayerUpdate {
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
fn layer_update(current: Option<&LayerSurfaceConfig>, next: &LayerSurfaceConfig) -> LayerUpdate {
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
fn sync_layer_surfaces(
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
        current.root = surface.root;
        current.updates_enabled = surface.updates_enabled;
        current.layout = None;
    }
    Ok(resumed)
}

/// Applies one compositor configure to a configured layer surface.
fn layer_surface_configure(
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
    let (physical_width, physical_height) =
        auxiliary_physical_size(surface.width, surface.height, scale);
    if let Some(renderer) = &mut surface.renderer {
        renderer
            .backend_mut()
            .resize(physical_width, physical_height);
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
fn layer_surface_scale(client: &LayerClient, state: &mut SurfaceEventState, layer: u64) {
    let Some(id) = window_surface_id(layer) else {
        return;
    };
    let Some(surface) = state.layer_surfaces.get_mut(&id) else {
        return;
    };
    let scale = client.layer_scale_120(layer).unwrap_or(120);
    if let Some(renderer) = &mut surface.renderer {
        let (width, height) = auxiliary_physical_size(surface.width, surface.height, scale);
        renderer.backend_mut().resize(width, height);
    }
}

/// Paints one configured layer surface when the compositor permits a frame.
fn layer_surface_frame(
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
    surface.needs_paint = false;
    paint_layer_surface(runtime, client, surface)
}

/// Drops one configured layer surface the compositor closed.
fn layer_surface_closed(
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
