fn apply_window_surface_actions(
    runtime: &mut Runtime,
    client: &LayerClient,
    floatings: &HashMap<u64, AuxiliarySurface>,
) {
    for action in runtime.take_window_surface_actions() {
        match action {
            WindowSurfaceAction::Move { id } if floatings.contains_key(&id) => {
                client.start_floating_move(id);
            }
            WindowSurfaceAction::Resize { id, edge } if floatings.contains_key(&id) => {
                let edge = match edge.as_str() {
                    "top" => FloatingResizeEdge::Top,
                    "bottom" => FloatingResizeEdge::Bottom,
                    "left" => FloatingResizeEdge::Left,
                    "right" => FloatingResizeEdge::Right,
                    "top_left" => FloatingResizeEdge::TopLeft,
                    "top_right" => FloatingResizeEdge::TopRight,
                    "bottom_left" => FloatingResizeEdge::BottomLeft,
                    "bottom_right" => FloatingResizeEdge::BottomRight,
                    _ => continue,
                };
                client.start_floating_resize(id, edge);
            }
            WindowSurfaceAction::Move { .. } | WindowSurfaceAction::Resize { .. } => {}
        }
    }
}

fn apply_parent_transitions(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
) -> Result<(), String> {
    let transitions = runtime.take_parent_transitions();
    if transitions.is_empty() {
        return Ok(());
    }
    let root = primary_surface_root(runtime)?;
    let (width, height) = client.logical_size();
    let available = Size {
        width: width as f64,
        height: height as f64,
    };
    for transition in transitions {
        Layout::transition_reparent(
            &mut runtime.scene_mut(),
            renderer.backend_mut(),
            ReparentTransition {
                root,
                node: transition.node,
                new_parent: transition.parent,
                anchors: transition.anchors,
                available,
                behavior: transition.behavior,
            },
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

