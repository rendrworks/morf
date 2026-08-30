fn paint(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
) -> Result<Layout, String> {
    let root = primary_surface_root(runtime)?;
    paint_layer(
        runtime,
        renderer,
        client,
        PRIMARY_LAYER,
        root,
        &runtime.layer_surface_config(),
    )
}

/// Lays out, masks, and renders the scene subtree of one layer surface.
///
/// Every layer surface derives its input region the same way: an explicit mask
/// when the configuration supplies one, and otherwise the live geometry of the
/// interactive items, recomputed here so it tracks the surface as it changes.
fn paint_layer(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
    layer: u64,
    root: NodeHandle,
    config: &LayerSurfaceConfig,
) -> Result<Layout, String> {
    let scene = runtime.scene();
    let (width, height) = client
        .layer_logical_size(layer)
        .ok_or_else(|| "layer surface disappeared while painting".to_owned())?;
    let scale_120 = client.layer_scale_120(layer).unwrap_or(120);
    let layout = Layout::compute(
        &scene,
        root,
        Size {
            width: width as f64,
            height: height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    let (physical_width, physical_height) = auxiliary_physical_size(width, height, scale_120);
    if let Some(regions) = &config.input_regions {
        client
            .set_layer_composed_input_region(layer, regions)
            .map_err(|error| error.to_string())?;
    } else {
        let input = layout
            .input_geometry(&scene)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|geometry| {
                let left = geometry.x.floor() as i32;
                let top = geometry.y.floor() as i32;
                let right = (geometry.x + geometry.width).ceil() as i32;
                let bottom = (geometry.y + geometry.height).ceil() as i32;
                InputRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                }
            })
            .collect::<Vec<_>>();
        client.set_layer_input_region(layer, Some(&input));
    }
    client.request_layer_frame(layer);
    client
        .layer_surface(layer)
        .ok_or_else(|| "layer surface disappeared while painting".to_owned())?
        .damage_buffer(0, 0, physical_width as i32, physical_height as i32);
    let damage = renderer
        .render(&scene, &layout, scale_120)
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        client.commit_layer(layer);
    }
    drop(scene);
    runtime.observe_layout(&layout);
    Ok(layout)
}

/// Paints one configured layer surface into its own renderer.
fn paint_layer_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let config = surface
        .layer_config
        .clone()
        .ok_or_else(|| "layer surface lost its configuration".to_owned())?;
    let layout = paint_layer(
        runtime,
        renderer,
        client,
        window_layer_id(surface.id),
        surface.root,
        &config,
    )?;
    surface.layout = Some(layout);
    Ok(())
}

fn paint_popup_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let scene = runtime.scene();
    let layout = Layout::compute(
        &scene,
        surface.root,
        Size {
            width: surface.width as f64,
            height: surface.height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    let (width, height) =
        auxiliary_physical_size(surface.width, surface.height, client.scale_120());
    client.request_popup_frame(surface.id);
    let popup = client
        .popup_surface(surface.id)
        .ok_or_else(|| "popup surface disappeared while painting".to_owned())?;
    popup.damage_buffer(0, 0, width as i32, height as i32);
    let damage = renderer
        .render(&scene, &layout, client.scale_120())
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        popup.commit();
    }
    drop(scene);
    runtime.observe_layout(&layout);
    surface.layout = Some(layout);
    Ok(())
}

fn paint_floating_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let scene = runtime.scene();
    let layout = Layout::compute(
        &scene,
        surface.root,
        Size {
            width: surface.width as f64,
            height: surface.height as f64,
        },
        renderer.backend_mut(),
    )
    .map_err(|error| error.to_string())?;
    let (width, height) =
        auxiliary_physical_size(surface.width, surface.height, client.scale_120());
    client.request_floating_frame(surface.id);
    let floating = client
        .floating_surface(surface.id)
        .ok_or_else(|| "floating surface disappeared while painting".to_owned())?;
    floating.damage_buffer(0, 0, width as i32, height as i32);
    let damage = renderer
        .render(&scene, &layout, client.scale_120())
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        floating.commit();
    }
    drop(scene);
    runtime.observe_layout(&layout);
    surface.layout = Some(layout);
    Ok(())
}

fn clock_text() -> String {
    jiff::Zoned::now().strftime("%H:%M:%S").to_string()
}

fn until_next_second() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Duration::from_nanos(1_000_000_000 - elapsed.subsec_nanos() as u64)
}
