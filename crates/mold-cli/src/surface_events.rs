fn handle_surface_event(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
    state: &mut SurfaceEventState,
    event: LayerEvent,
    tx: &mpsc::Sender<SupervisorMessage>,
    name: &str,
) -> Result<bool, String> {
    let mut repaint = false;
    match event {
        LayerEvent::Configure { .. } | LayerEvent::Scale(_) => {
            let (width, height) = client.physical_size();
            renderer.backend_mut().resize(width, height);
            for surface in state
                .popup_surfaces
                .values_mut()
                .chain(state.floating_surfaces.values_mut())
            {
                if let Some(renderer) = &mut surface.renderer {
                    let (width, height) =
                        auxiliary_physical_size(surface.width, surface.height, client.scale_120());
                    renderer.backend_mut().resize(width, height);
                }
            }
            repaint = true;
        }
        LayerEvent::Frame { time_ms } => {
            let delta = state
                .last_frame
                .map(|previous: u32| time_ms.wrapping_sub(previous).min(250))
                .unwrap_or(0);
            state.last_frame = Some(time_ms);
            let frame = runtime
                .tick_animations(Duration::from_millis(delta as u64))
                .map_err(|error| error.to_string())?;
            repaint |= frame.active || !frame.changes.is_empty();
        }
        LayerEvent::Closed => return Err("layer surface was closed".to_owned()),
        LayerEvent::Idle { timeout_ms, idle } => {
            repaint |= runtime.dispatch_idle(timeout_ms, idle);
        }
        LayerEvent::OutputPower { .. } => {}
        LayerEvent::Clipboard { text } => {
            repaint |= runtime.dispatch_clipboard(text);
        }
        LayerEvent::Screencopy { request_id, result } => {
            repaint |= dispatch_screencopy(runtime, request_id, result);
        }
        LayerEvent::InputMethod(state) => {
            repaint |= runtime.dispatch_input_method(
                state.active,
                state.surrounding_text,
                state.cursor,
                state.anchor,
                state.serial,
            );
        }
        LayerEvent::TextInput(state) => {
            repaint |= runtime.dispatch_text_input(
                state.focused,
                state.preedit,
                state.preedit_begin,
                state.preedit_end,
                state.commit,
                state.delete_before,
                state.delete_after,
                state.serial,
            );
        }
        LayerEvent::PointerMotion { surface, x, y } => {
            let Some(hit_layout) = surface_layout(
                surface,
                &state.layout,
                &state.popup_surfaces,
                &state.floating_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            let next_hovered = hit.map(|node| (surface, node));
            if next_hovered != state.hovered {
                if let Some((_, node)) = state.hovered {
                    repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                }
                if let Some(node) = hit {
                    repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerEntered);
                }
                state.hovered = next_hovered;
            }
            if let Some(node) = hit {
                repaint |=
                    runtime.dispatch_pointer_event(node, UiEvent::PointerMoved, x, y, 0.0, 0.0);
            }
            if let Some((pressed_surface, node, start_x, start_y, dragging)) = &mut state.pressed
                && *pressed_surface == surface
            {
                let delta_x = x - *start_x;
                let delta_y = y - *start_y;
                if !*dragging && delta_x.hypot(delta_y) >= 8.0 {
                    *dragging = true;
                    repaint |= runtime.dispatch_pointer_event(
                        *node,
                        UiEvent::DragStarted,
                        x,
                        y,
                        delta_x,
                        delta_y,
                    );
                }
                if *dragging {
                    repaint |= runtime.dispatch_pointer_event(
                        *node,
                        UiEvent::Dragged,
                        x,
                        y,
                        delta_x,
                        delta_y,
                    );
                }
            }
        }
        LayerEvent::PointerLeave { surface } => {
            if state
                .hovered
                .is_some_and(|(hovered_surface, _)| hovered_surface == surface)
                && let Some((_, node)) = state.hovered.take()
            {
                repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
            }
        }
        LayerEvent::PointerAxis {
            surface,
            x,
            y,
            horizontal,
            vertical,
            horizontal_steps,
            vertical_steps,
        } => {
            let Some(hit_layout) = surface_layout(
                surface,
                &state.layout,
                &state.popup_surfaces,
                &state.floating_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            if let Some(node) = hit {
                repaint |= runtime.dispatch_wheel_event(
                    node,
                    (x, y),
                    (horizontal, vertical),
                    (horizontal_steps, vertical_steps),
                );
            }
        }
        LayerEvent::PointerButton {
            surface,
            button,
            pressed: true,
            x,
            y,
        } => {
            let Some(hit_layout) = surface_layout(
                surface,
                &state.layout,
                &state.popup_surfaces,
                &state.floating_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            let hit = hit.filter(|node| runtime.accepts_pointer_button(*node, button));
            state.pressed = hit.map(|node| (surface, node, x, y, false));
            if let Some(target) = hit.and_then(|node| runtime.key_target_for_node(node)) {
                state.focused.insert(surface, target);
            } else {
                state.focused.remove(&surface);
            }
            if let Some(node) = hit {
                repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
            }
        }
        LayerEvent::TouchDown { surface, id, x, y } => {
            let Some(hit_layout) = surface_layout(
                surface,
                &state.layout,
                &state.popup_surfaces,
                &state.floating_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            if let Some(node) = hit {
                state.touches.insert(id, (surface, node, x, y));
                if let Some(target) = runtime.key_target_for_node(node) {
                    state.focused.insert(surface, target);
                } else {
                    state.focused.remove(&surface);
                }
                repaint |= runtime.dispatch_ui_event(node, UiEvent::Pressed);
                repaint |= runtime.dispatch_touch_event(node, UiEvent::TouchPressed, id, x, y);
            }
        }
        LayerEvent::TouchMotion { id, x, y, .. } => {
            if let Some((_, node, last_x, last_y)) = state.touches.get_mut(&id) {
                *last_x = x;
                *last_y = y;
                repaint |= runtime.dispatch_touch_event(*node, UiEvent::TouchMoved, id, x, y);
            }
        }
        LayerEvent::TouchUp { surface, id, x, y } => {
            if let Some((touch_surface, node, _, _)) = state.touches.remove(&id) {
                repaint |= runtime.dispatch_touch_event(node, UiEvent::TouchReleased, id, x, y);
                repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                let hit = surface_layout(
                    surface,
                    &state.layout,
                    &state.popup_surfaces,
                    &state.floating_surfaces,
                )
                .filter(|_| touch_surface == surface)
                .map(|layout| layout.hit_test(&runtime.scene(), x, y))
                .transpose()
                .map_err(|error| error.to_string())?
                .flatten();
                if hit == Some(node) {
                    repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                }
            }
        }
        LayerEvent::TouchCancel => {
            for (id, (_, node, x, y)) in state.touches.drain() {
                repaint |= runtime.dispatch_touch_event(node, UiEvent::TouchCanceled, id, x, y);
                repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
            }
        }
        LayerEvent::PointerButton {
            surface,
            pressed: false,
            x,
            y,
            ..
        } => {
            let hit = surface_layout(
                surface,
                &state.layout,
                &state.popup_surfaces,
                &state.floating_surfaces,
            )
            .map(|layout| layout.hit_test(&runtime.scene(), x, y))
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten();
            if let Some((pressed_surface, node, start_x, start_y, dragging)) = state.pressed.take()
            {
                repaint |= runtime.dispatch_ui_event(node, UiEvent::Released);
                if dragging {
                    repaint |= runtime.dispatch_pointer_event(
                        node,
                        UiEvent::DragFinished,
                        x,
                        y,
                        x - start_x,
                        y - start_y,
                    );
                } else if pressed_surface == surface && hit == Some(node) {
                    repaint |= runtime.dispatch_ui_event(node, UiEvent::Clicked);
                }
            }
        }
        LayerEvent::Key {
            surface,
            pressed: true,
            keysym,
            text,
            ..
        } => {
            let Some(root) = surface_root(
                surface,
                primary_surface_root(runtime)?,
                &state.popup_surfaces,
                &state.floating_surfaces,
            ) else {
                return Ok(false);
            };
            let current = state
                .focused
                .get(&surface)
                .copied()
                .filter(|node| runtime.node_in_subtree(root, *node));
            if keysym == 0xff09 {
                if let Some(next) = runtime.next_key_target_in(root, current) {
                    state.focused.insert(surface, next);
                } else {
                    state.focused.remove(&surface);
                }
                repaint = true;
            } else if let Some(node) = current.or_else(|| runtime.first_key_target_in(root)) {
                state.focused.insert(surface, node);
                repaint |= runtime.dispatch_key_event(node, keysym, text.as_deref());
            }
        }
        LayerEvent::PopupConfigure { id, width, height } => {
            if let Some(surface) = state.popup_surfaces.get_mut(&id) {
                let initial = surface.renderer.is_none();
                surface.width = width.max(1);
                surface.height = height.max(1);
                let (physical_width, physical_height) =
                    auxiliary_physical_size(surface.width, surface.height, client.scale_120());
                if let Some(renderer) = &mut surface.renderer {
                    renderer
                        .backend_mut()
                        .resize(physical_width, physical_height);
                } else {
                    let target = client
                        .popup_window_target(id)
                        .ok_or_else(|| "configured popup disappeared".to_owned())?;
                    let backend = pollster::block_on(WgpuBackend::new_surface(
                        target,
                        physical_width,
                        physical_height,
                    ))
                    .map_err(|error| error.to_string())?;
                    surface.renderer = Some(RenderEngine::new(backend));
                }
                if initial || surface.updates_enabled {
                    paint_popup_surface(runtime, client, surface)?;
                }
            }
        }
        LayerEvent::PopupFrame { id, .. } => {
            if let Some(surface) = state
                .popup_surfaces
                .get_mut(&id)
                .filter(|surface| surface.updates_enabled)
            {
                paint_popup_surface(runtime, client, surface)?;
            }
        }
        LayerEvent::PopupDone { id } => {
            if let Some(surface) = state.popup_surfaces.remove(&id) {
                runtime.set_window_surface_visible(surface.id, false);
            }
        }
        LayerEvent::FloatingConfigure { id, width, height } => {
            if let Some(surface) = state.floating_surfaces.get_mut(&id) {
                let initial = surface.renderer.is_none();
                surface.width = width.max(1);
                surface.height = height.max(1);
                let (physical_width, physical_height) =
                    auxiliary_physical_size(surface.width, surface.height, client.scale_120());
                if let Some(renderer) = &mut surface.renderer {
                    renderer
                        .backend_mut()
                        .resize(physical_width, physical_height);
                } else {
                    let target = client
                        .floating_window_target(id)
                        .ok_or_else(|| "configured floating surface disappeared".to_owned())?;
                    let backend = pollster::block_on(WgpuBackend::new_surface(
                        target,
                        physical_width,
                        physical_height,
                    ))
                    .map_err(|error| error.to_string())?;
                    surface.renderer = Some(RenderEngine::new(backend));
                }
                if initial || surface.updates_enabled {
                    paint_floating_surface(runtime, client, surface)?;
                }
            }
        }
        LayerEvent::FloatingFrame { id, .. } => {
            if let Some(surface) = state
                .floating_surfaces
                .get_mut(&id)
                .filter(|surface| surface.updates_enabled)
            {
                paint_floating_surface(runtime, client, surface)?;
            }
        }
        LayerEvent::FloatingClose { id } => {
            if let Some(surface) = state.floating_surfaces.remove(&id) {
                runtime.set_window_surface_visible(surface.id, false);
            }
        }
        LayerEvent::Key { pressed: false, .. }
        | LayerEvent::Modifiers { .. }
        | LayerEvent::SessionLocked
        | LayerEvent::SessionLockFinished
        | LayerEvent::SessionLockConfigure { .. }
        | LayerEvent::SessionLockSurfaceRemoved { .. }
        | LayerEvent::SessionLockFrame { .. } => {}
        LayerEvent::Screens(screens) => {
            tx.send(SupervisorMessage::Worker(WorkerMessage::Screens {
                output: name.to_owned(),
                screens,
            }))
            .map_err(|_| "output supervisor stopped".to_owned())?;
        }
    }
    Ok(repaint)
}
