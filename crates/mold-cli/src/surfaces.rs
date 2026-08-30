struct AuxiliarySurface {
    id: u64,
    root: NodeHandle,
    updates_enabled: bool,
    width: u32,
    height: u32,
    renderer: Option<RenderEngine<WgpuBackend>>,
    layout: Option<Layout>,
    popup_config: Option<PopupSurfaceConfig>,
    floating_config: Option<FloatingSurfaceConfig>,
    layer_config: Option<LayerSurfaceConfig>,
    /// Whether this surface has work pending for the next frame callback.
    ///
    /// A configured layer surface is permanent decoration, so repainting it on
    /// every frame callback would keep the compositor compositing forever. It
    /// paints only when something marked it dirty, and asks for another frame
    /// only when it painted.
    needs_paint: bool,
}

/// Largest frame delta charged to animations in a single tick.
///
/// A compositor that fell behind should let motion catch up, but only so far.
/// Beyond a few dropped frames, advancing by the whole gap reads as a jump, so
/// the tick is capped and the remaining time is simply lost.
const MAX_FRAME_DELTA_MS: u32 = 100;

/// How far to advance animations for a frame callback at `time_ms`.
///
/// `previous` is the timebase carried forward from the last tick, and is absent
/// whenever the scene had settled. That absence is what keeps idle time out of
/// the clock: the compositor stops sending frame callbacks while nothing moves,
/// so the gap since the last one measures how long the shell sat still, not how
/// far motion should advance. Charging it to an animation that started in the
/// meantime makes it jump, and a long enough gap lands it on its target in a
/// single tick.
fn animation_delta(previous: Option<u32>, time_ms: u32) -> Duration {
    let elapsed = previous.map_or(0, |previous| {
        time_ms.wrapping_sub(previous).min(MAX_FRAME_DELTA_MS)
    });
    Duration::from_millis(elapsed.into())
}

struct SurfaceEventState {
    layout: Layout,
    popup_surfaces: HashMap<u64, AuxiliarySurface>,
    floating_surfaces: HashMap<u64, AuxiliarySurface>,
    layer_surfaces: HashMap<u64, AuxiliarySurface>,
    last_frame: Option<u32>,
    hovered: Option<(SurfaceRole, NodeHandle)>,
    pressed: Option<(SurfaceRole, NodeHandle, f64, f64, bool)>,
    focused: HashMap<SurfaceRole, NodeHandle>,
    touches: HashMap<i32, (SurfaceRole, NodeHandle, f64, f64)>,
}

fn popup_anchor(value: &str) -> Result<PopupAnchor, String> {
    Ok(match value {
        "none" => PopupAnchor::None,
        "top" => PopupAnchor::Top,
        "bottom" => PopupAnchor::Bottom,
        "left" => PopupAnchor::Left,
        "right" => PopupAnchor::Right,
        "top_left" => PopupAnchor::TopLeft,
        "top_right" => PopupAnchor::TopRight,
        "bottom_left" => PopupAnchor::BottomLeft,
        "bottom_right" => PopupAnchor::BottomRight,
        value => return Err(format!("invalid popup anchor `{value}`")),
    })
}

fn popup_gravity(value: &str) -> Result<PopupGravity, String> {
    Ok(match value {
        "none" => PopupGravity::None,
        "top" => PopupGravity::Top,
        "bottom" => PopupGravity::Bottom,
        "left" => PopupGravity::Left,
        "right" => PopupGravity::Right,
        "top_left" => PopupGravity::TopLeft,
        "top_right" => PopupGravity::TopRight,
        "bottom_left" => PopupGravity::BottomLeft,
        "bottom_right" => PopupGravity::BottomRight,
        value => return Err(format!("invalid popup gravity `{value}`")),
    })
}

fn window_surface_parent(surface: &WindowSurfaceConfig) -> Option<u64> {
    match &surface.kind {
        WindowSurfaceKind::Popup(config) => config.parent,
        WindowSurfaceKind::Floating(config) => config.parent,
        WindowSurfaceKind::Layer(_) => None,
    }
}

fn window_surface_effectively_visible(
    id: u64,
    surfaces: &HashMap<u64, &WindowSurfaceConfig>,
    visiting: &mut HashSet<u64>,
) -> bool {
    let Some(surface) = surfaces.get(&id) else {
        return false;
    };
    if !surface.visible || !visiting.insert(id) {
        return false;
    }
    let visible = window_surface_parent(surface)
        .is_none_or(|parent| window_surface_effectively_visible(parent, surfaces, visiting));
    visiting.remove(&id);
    visible
}

fn sync_window_surfaces(
    runtime: &Runtime,
    client: &mut LayerClient,
    popups: &mut HashMap<u64, AuxiliarySurface>,
    floatings: &mut HashMap<u64, AuxiliarySurface>,
    layers: &mut HashMap<u64, AuxiliarySurface>,
    output: &str,
) -> Result<bool, String> {
    let mut resumed = false;
    let surfaces = runtime.window_surface_configs();
    let surfaces_by_id = surfaces
        .iter()
        .map(|surface| (surface.id, surface))
        .collect::<HashMap<_, _>>();
    let mut desired_popups = surfaces
        .iter()
        .filter(|surface| {
            matches!(&surface.kind, WindowSurfaceKind::Popup(_))
                && window_surface_effectively_visible(
                    surface.id,
                    &surfaces_by_id,
                    &mut HashSet::new(),
                )
        })
        .collect::<Vec<_>>();
    let mut desired_floatings = surfaces
        .iter()
        .filter(|surface| {
            matches!(&surface.kind, WindowSurfaceKind::Floating(_))
                && window_surface_effectively_visible(
                    surface.id,
                    &surfaces_by_id,
                    &mut HashSet::new(),
                )
        })
        .collect::<Vec<_>>();
    let mut desired_layers = surfaces
        .iter()
        .filter(|surface| {
            matches!(&surface.kind, WindowSurfaceKind::Layer(_))
                && window_surface_effectively_visible(
                    surface.id,
                    &surfaces_by_id,
                    &mut HashSet::new(),
                )
        })
        .collect::<Vec<_>>();
    desired_popups.sort_by_key(|surface| surface.id);
    desired_floatings.sort_by_key(|surface| surface.id);
    desired_layers.sort_by_key(|surface| surface.id);
    let desired_popup_ids = desired_popups
        .iter()
        .map(|surface| surface.id)
        .collect::<HashSet<_>>();
    let desired_floating_ids = desired_floatings
        .iter()
        .map(|surface| surface.id)
        .collect::<HashSet<_>>();

    let mut stale_popups = popups
        .keys()
        .filter(|id| !desired_popup_ids.contains(id))
        .copied()
        .collect::<Vec<_>>();
    stale_popups.sort_unstable_by(|a, b| b.cmp(a));
    for id in stale_popups {
        client.close_popup(id);
        popups.remove(&id);
    }
    let mut stale_floatings = floatings
        .keys()
        .filter(|id| !desired_floating_ids.contains(id))
        .copied()
        .collect::<Vec<_>>();
    stale_floatings.sort_unstable_by(|a, b| b.cmp(a));
    for id in stale_floatings {
        client.close_floating(id);
        floatings.remove(&id);
    }
    resumed |= sync_layer_surfaces(client, output, &desired_layers, layers)?;
    let mut reopened = HashSet::new();
    for surface in desired_floatings {
        let id = surface.id;
        let WindowSurfaceKind::Floating(config) = &surface.kind else {
            unreachable!();
        };
        let changed = floatings
            .get(&id)
            .is_none_or(|current| current.floating_config.as_ref() != Some(config))
            || config
                .parent
                .is_some_and(|parent| reopened.contains(&parent));
        if changed {
            client.close_floating(id);
            client
                .open_floating(
                    id,
                    config.parent,
                    FloatingConfig {
                        width: config.width,
                        height: config.height,
                        minimum_width: config.minimum_width,
                        minimum_height: config.minimum_height,
                        maximum_width: config.maximum_width,
                        maximum_height: config.maximum_height,
                        title: config.title.clone(),
                        app_id: config.app_id.clone(),
                        minimized: config.minimized,
                        maximized: config.maximized,
                        fullscreen: config.fullscreen,
                    },
                )
                .map_err(|error| error.to_string())?;
            reopened.insert(id);
            floatings.insert(
                id,
                AuxiliarySurface {
                    id: surface.id,
                    root: surface.root,
                    updates_enabled: surface.updates_enabled,
                    width: config.width,
                    height: config.height,
                    renderer: None,
                    layout: None,
                    popup_config: None,
                    floating_config: Some(config.clone()),
                    layer_config: None,
                    needs_paint: true,
                },
            );
        } else if let Some(current) = floatings.get_mut(&id) {
            resumed |= !current.updates_enabled && surface.updates_enabled;
            current.root = surface.root;
            current.updates_enabled = surface.updates_enabled;
            current.width = config.width;
            current.height = config.height;
            current.layout = None;
        }
    }
    for surface in desired_popups {
        let id = surface.id;
        let WindowSurfaceKind::Popup(config) = &surface.kind else {
            unreachable!();
        };
        let changed = popups
            .get(&id)
            .is_none_or(|current| current.popup_config.as_ref() != Some(config))
            || config
                .parent
                .is_some_and(|parent| reopened.contains(&parent));
        if changed {
            client.close_popup(id);
            let parent = if let Some(parent) = config.parent {
                let parent = surfaces_by_id
                    .get(&parent)
                    .ok_or_else(|| "popup parent is stale".to_owned())?;
                match parent.kind {
                    WindowSurfaceKind::Popup(_) => SurfaceRole::Popup(parent.id),
                    WindowSurfaceKind::Floating(_) => SurfaceRole::Floating(parent.id),
                    WindowSurfaceKind::Layer(_) => SurfaceRole::Layer(window_layer_id(parent.id)),
                }
            } else {
                SurfaceRole::Layer(PRIMARY_LAYER)
            };
            client
                .open_popup(
                    id,
                    parent,
                    PopupConfig {
                        anchor: InputRect {
                            x: config.anchor_x,
                            y: config.anchor_y,
                            width: config.anchor_width,
                            height: config.anchor_height,
                        },
                        width: config.width,
                        height: config.height,
                        anchor_edge: popup_anchor(&config.anchor_edge)?,
                        gravity: popup_gravity(&config.gravity)?,
                        offset_x: config.offset_x,
                        offset_y: config.offset_y,
                        constraints: PopupConstraints {
                            slide_x: config.constraints.slide_x,
                            slide_y: config.constraints.slide_y,
                            flip_x: config.constraints.flip_x,
                            flip_y: config.constraints.flip_y,
                            resize_x: config.constraints.resize_x,
                            resize_y: config.constraints.resize_y,
                        },
                        grab_focus: config.grab_focus,
                    },
                )
                .map_err(|error| error.to_string())?;
            reopened.insert(id);
            popups.insert(
                id,
                AuxiliarySurface {
                    id: surface.id,
                    root: surface.root,
                    updates_enabled: surface.updates_enabled,
                    width: config.width,
                    height: config.height,
                    renderer: None,
                    layout: None,
                    popup_config: Some(config.clone()),
                    floating_config: None,
                    layer_config: None,
                    needs_paint: true,
                },
            );
        } else if let Some(current) = popups.get_mut(&id) {
            resumed |= !current.updates_enabled && surface.updates_enabled;
            current.root = surface.root;
            current.updates_enabled = surface.updates_enabled;
            current.width = config.width;
            current.height = config.height;
            current.layout = None;
        }
    }
    Ok(resumed)
}

fn auxiliary_physical_size(width: u32, height: u32, scale_120: u32) -> (u32, u32) {
    let physical = |value: u32| {
        u32::try_from((u64::from(value) * u64::from(scale_120)).div_ceil(120)).unwrap_or(u32::MAX)
    };
    (physical(width), physical(height))
}

fn surface_layout<'a>(
    surface: SurfaceRole,
    layer: &'a Layout,
    popups: &'a HashMap<u64, AuxiliarySurface>,
    floatings: &'a HashMap<u64, AuxiliarySurface>,
    layers: &'a HashMap<u64, AuxiliarySurface>,
) -> Option<&'a Layout> {
    match surface {
        SurfaceRole::Layer(PRIMARY_LAYER) => Some(layer),
        SurfaceRole::Layer(id) => layers.get(&window_surface_id(id)?)?.layout.as_ref(),
        SurfaceRole::Popup(id) => popups.get(&id)?.layout.as_ref(),
        SurfaceRole::Floating(id) => floatings.get(&id)?.layout.as_ref(),
    }
}

fn surface_root(
    surface: SurfaceRole,
    layer: NodeHandle,
    popups: &HashMap<u64, AuxiliarySurface>,
    floatings: &HashMap<u64, AuxiliarySurface>,
    layers: &HashMap<u64, AuxiliarySurface>,
) -> Option<NodeHandle> {
    match surface {
        SurfaceRole::Layer(PRIMARY_LAYER) => Some(layer),
        SurfaceRole::Layer(id) => layers
            .get(&window_surface_id(id)?)
            .map(|surface| surface.root),
        SurfaceRole::Popup(id) => popups.get(&id).map(|surface| surface.root),
        SurfaceRole::Floating(id) => floatings.get(&id).map(|surface| surface.root),
    }
}

fn primary_surface_root(runtime: &Runtime) -> Result<NodeHandle, String> {
    let roots = runtime.scene().roots();
    let mut window_roots = HashSet::new();
    for surface in runtime.window_surface_configs() {
        if !window_roots.insert(surface.root) {
            return Err("a scene root cannot back multiple window surfaces".into());
        }
        if runtime
            .scene()
            .parent(surface.root)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("window surface roots must be top-level scene nodes".into());
        }
    }
    let primary = roots
        .into_iter()
        .filter(|root| !window_roots.contains(root))
        .collect::<Vec<_>>();
    if primary.len() != 1 {
        return Err("configuration must create exactly one primary surface root".into());
    }
    Ok(primary[0])
}

fn runtime_bar_config(surface: &LayerSurfaceConfig, output: &str) -> Result<BarConfig, String> {
    let layer = match surface.layer.as_str() {
        "background" => ShellLayer::Background,
        "bottom" => ShellLayer::Bottom,
        "top" => ShellLayer::Top,
        "overlay" => ShellLayer::Overlay,
        value => return Err(format!("unsupported layer surface layer `{value}`")),
    };
    let keyboard_focus = match surface.keyboard_focus.as_str() {
        "none" => KeyboardFocus::None,
        "exclusive" => KeyboardFocus::Exclusive,
        "on_demand" => KeyboardFocus::OnDemand,
        value => return Err(format!("unsupported keyboard focus policy `{value}`")),
    };
    Ok(BarConfig {
        namespace: surface.namespace.clone(),
        width: surface.width,
        height: surface.height,
        exclusive_zone: surface.exclusive_zone,
        output: Some(output.to_owned()),
        anchors: LayerAnchors {
            top: surface.anchors.top,
            right: surface.anchors.right,
            bottom: surface.anchors.bottom,
            left: surface.anchors.left,
        },
        margin_top: surface.margin_top,
        margin_right: surface.margin_right,
        margin_bottom: surface.margin_bottom,
        margin_left: surface.margin_left,
        layer,
        keyboard_focus,
    })
}

fn connect_runtime_surface(runtime: &Runtime, output: &str) -> Result<LayerClient, String> {
    let config = runtime.layer_surface_config();
    let mut client =
        LayerClient::connect(runtime_bar_config(&config, output)?).map_err(|e| e.to_string())?;
    open_reserve_layers(&mut client, &config, output)?;
    loop {
        client.dispatch().map_err(|error| error.to_string())?;
        while let Some(event) = client.next_event() {
            match event {
                LayerEvent::Configure { id, .. } if id == PRIMARY_LAYER => return Ok(client),
                LayerEvent::Closed { id } if id == PRIMARY_LAYER => {
                    return Err("layer surface was closed".to_owned());
                }
                _ => {}
            }
        }
    }
}
