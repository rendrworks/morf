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
    hovered: Option<(SurfaceRole, Hit)>,
    pressed: Option<(SurfaceRole, Hit, f64, f64, bool)>,
    focused: HashMap<SurfaceRole, NodeHandle>,
    touches: HashMap<i32, (SurfaceRole, Hit, f64, f64)>,
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

/// Builds the client-side geometry request for one configured popup.
///
/// Every field here is a positioner field, which is what lets the same builder
/// serve both creating a popup and moving one.
fn popup_client_config(config: &PopupSurfaceConfig) -> Result<PopupConfig, String> {
    Ok(PopupConfig {
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
    })
}

/// Whether a popup's new configuration needs a brand-new `xdg_popup`.
///
/// Everything a positioner carries — anchor rectangle, anchor edge, gravity,
/// offset, constraint policy, size — is replaceable on a mapped popup, so a
/// change to any of them is *positional* and goes through `xdg_popup.reposition`.
/// Two fields are not: the parent is bound when the popup object is created, and
/// the grab is taken once against an input serial. Only those force a teardown.
fn popup_change_is_structural(current: &PopupSurfaceConfig, next: &PopupSurfaceConfig) -> bool {
    current.parent != next.parent || current.grab_focus != next.grab_focus
}

/// Resolves the surface a popup anchors to, defaulting to the shell's own layer.
fn popup_parent_role(
    config: &PopupSurfaceConfig,
    surfaces_by_id: &HashMap<u64, &WindowSurfaceConfig>,
) -> Result<SurfaceRole, String> {
    let Some(parent) = config.parent else {
        return Ok(SurfaceRole::Layer(PRIMARY_LAYER));
    };
    let parent = surfaces_by_id
        .get(&parent)
        .ok_or_else(|| "popup parent is stale".to_owned())?;
    Ok(match parent.kind {
        WindowSurfaceKind::Popup(_) => SurfaceRole::Popup(parent.id),
        WindowSurfaceKind::Floating(_) => SurfaceRole::Floating(parent.id),
        WindowSurfaceKind::Layer(_) => SurfaceRole::Layer(window_layer_id(parent.id)),
    })
}

/// Creates a popup's Wayland surface and records the host state that tracks it.
///
/// The renderer starts empty and is rebuilt from the first configure. It cannot
/// be carried over: a new `xdg_popup` carries a new `wl_surface`, and a wgpu
/// swapchain cannot outlive the surface it was created from. That cost is why
/// this path is taken only when the popup truly cannot be moved in place.
fn open_popup_surface(
    client: &mut LayerClient,
    surface: &WindowSurfaceConfig,
    config: &PopupSurfaceConfig,
    parent: SurfaceRole,
    popups: &mut HashMap<u64, AuxiliarySurface>,
) -> Result<(), String> {
    client
        .open_popup(surface.id, parent, popup_client_config(config)?)
        .map_err(|error| error.to_string())?;
    popups.insert(
        surface.id,
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
    Ok(())
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
    // A surface is wanted only when it and every ancestor it hangs off are
    // visible, and the three kinds are then handled in identifier order so a
    // parent is always opened before the child anchored to it.
    let desired = |wanted: fn(&WindowSurfaceKind) -> bool| {
        let mut surfaces = surfaces
            .iter()
            .filter(|surface| {
                wanted(&surface.kind)
                    && window_surface_effectively_visible(
                        surface.id,
                        &surfaces_by_id,
                        &mut HashSet::new(),
                    )
            })
            .collect::<Vec<_>>();
        surfaces.sort_by_key(|surface| surface.id);
        surfaces
    };
    let desired_popups = desired(|kind| matches!(kind, WindowSurfaceKind::Popup(_)));
    let desired_floatings = desired(|kind| matches!(kind, WindowSurfaceKind::Floating(_)));
    let desired_layers = desired(|kind| matches!(kind, WindowSurfaceKind::Layer(_)));
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
        // A popup the compositor has dismissed is gone from the client while the
        // host still tracks it, and has nothing left to reposition.
        let tracked = popups
            .get(&id)
            .and_then(|current| current.popup_config.as_ref())
            .filter(|_| client.popup_surface(id).is_some());
        // A popup whose parent was just re-created is anchored to a surface that
        // no longer exists, so it follows its parent down whatever its geometry.
        let mut structural = tracked
            .is_none_or(|tracked| popup_change_is_structural(tracked, config))
            || config
                .parent
                .is_some_and(|parent| reopened.contains(&parent));
        if !structural && tracked != Some(config) {
            // Only the positioner moved, so the popup moves with its wl_surface,
            // its GPU surface and its swapchain all intact. A compositor whose
            // `xdg_popup` predates version 3 has no `reposition` request and says
            // so by changing nothing; then the popup has to be rebuilt after all.
            structural = !client
                .reposition_popup(id, popup_client_config(config)?)
                .map_err(|error| error.to_string())?;
        }
        if structural {
            let parent = popup_parent_role(config, &surfaces_by_id)?;
            open_popup_surface(client, surface, config, parent, popups)?;
            reopened.insert(id);
        } else if let Some(current) = popups.get_mut(&id) {
            // The stored size is deliberately left alone. A repositioned popup
            // keeps its current dimensions until the compositor answers with the
            // configure carrying the geometry it settled on, and that configure
            // is also what resizes the swapchain — writing the requested size
            // here would let the two disagree for a frame.
            resumed |= !current.updates_enabled && surface.updates_enabled;
            current.root = surface.root;
            current.updates_enabled = surface.updates_enabled;
            current.popup_config = Some(config.clone());
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
