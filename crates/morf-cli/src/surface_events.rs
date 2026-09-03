use morf_layout::Hit;
use morf_lua::{EventPoint, Runtime, UiEvent};
use morf_render::{RenderEngine, WgpuBackend};
use morf_wayland::{LayerClient, LayerEvent, PRIMARY_LAYER, SurfaceRole, physical_size};
use std::sync::mpsc;

use crate::{lock::*, paint::*, services::*, surface_layers::*, surface_touch::*, surfaces::*};

pub(crate) fn handle_surface_event(
    runtime: &mut Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &mut LayerClient,
    state: &mut SurfaceEventState,
    event: LayerEvent,
    tx: &mpsc::Sender<SupervisorMessage>,
    name: &str,
) -> Result<bool, String> {
    let mut repaint = false;
    match event {
        LayerEvent::Configure { id, .. } | LayerEvent::Scale { id, .. } if id == PRIMARY_LAYER => {
            let (width, height) = client.physical_size();
            renderer.resize(width, height);
            for surface in state
                .popup_surfaces
                .values_mut()
                .chain(state.floating_surfaces.values_mut())
            {
                if let Some(renderer) = &mut surface.renderer {
                    // Still the layer's scale here, as a fallback: a compositor
                    // with no fractional-scale protocol never sends `AuxScale`,
                    // and these surfaces would otherwise never be resized at
                    // all. Where it does, `AuxScale` arrives too and corrects
                    // this with the surface's own.
                    let (width, height) =
                        physical_size((surface.width, surface.height), client.scale_120());
                    renderer.resize(width, height);
                }
            }
            // The opaque region is a size, so it follows the new one.
            apply_primary_opaque(runtime, client);
            repaint = true;
        }
        LayerEvent::Configure { id, width, height } => {
            layer_surface_configure(runtime, client, state, id, width, height)?;
        }
        LayerEvent::Scale { id, .. } => layer_surface_scale(runtime, client, state, id)?,
        LayerEvent::Frame { id, time_ms } if id == PRIMARY_LAYER => {
            let delta = animation_delta(state.last_frame, time_ms);
            let frame = runtime
                .tick_animations(delta)
                .map_err(|error| error.to_string())?;
            // Carried forward only while motion continues, so the next run of
            // animation starts from a clean timebase rather than inheriting
            // however long the shell was idle.
            state.last_frame = frame.active.then_some(time_ms);
            // The callbacks themselves are the clock: whatever rate the
            // compositor offers this output is the rate to pace against.
            if !delta.is_zero() {
                state.refresh = delta;
            }
            // A shader reading the clock is motion like any other: it makes
            // the frame *advance*, and the pacer still decides which callbacks
            // are painted on. Forcing a repaint outside this path would spin as
            // fast as the event loop turns rather than at the output's rate.
            let advanced = frame.active || frame.changed > 0 || state.animating_shaders;
            if advanced {
                // A surface that cannot paint inside one refresh paints on
                // every second callback instead, and keeps that cadence rather
                // than missing deadlines at random.
                if state.pacer.due(state.refresh) {
                    repaint = true;
                } else {
                    // The next callback is asked for by painting, so a skipped
                    // frame has to ask for itself — otherwise the compositor
                    // has nothing outstanding, never calls back, and the
                    // surface stops dead on the first frame it gives up.
                    client.request_layer_frame(PRIMARY_LAYER);
                    client.commit_layer(PRIMARY_LAYER);
                }
            } else {
                state.pacer.rest();
            }
            // Configured layer surfaces have no clock of their own; the shell's
            // tick is what tells them a repaint is due, and a surface that is
            // already idle needs a frame callback to come back on.
            if advanced {
                for surface in state.layer_surfaces.values_mut() {
                    if surface.updates_enabled && !surface.needs_paint {
                        surface.needs_paint = true;
                        client.request_layer_frame(window_layer_id(surface.id));
                    }
                }
            }
        }
        LayerEvent::Frame { id, .. } => layer_surface_frame(runtime, client, state, id)?,
        LayerEvent::Closed { id } if id == PRIMARY_LAYER => {
            return Err("layer surface was closed".to_owned());
        }
        LayerEvent::Closed { id } => layer_surface_closed(runtime, client, state, id),
        LayerEvent::Idle {
            timeout_ms,
            input_only,
            idle,
        } => {
            repaint |= runtime.dispatch_idle(timeout_ms, input_only, idle);
        }
        LayerEvent::Clipboard { text } => {
            repaint |= runtime.dispatch_clipboard(text);
        }
        LayerEvent::Screencopy { request_id, result } => {
            repaint |= dispatch_screencopy(runtime, Some(renderer), request_id, result);
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
                &state.layer_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            // Hover is compared by node, not by hit: the same node under a
            // moving pointer is still the same hover, even though its local
            // coordinates change with every motion event.
            let next_hovered = hit.map(|hit| (surface, hit));
            let entered = next_hovered.map(|(role, hit)| (role, hit.node));
            let left = state
                .hovered
                .map(|(role, hit): (SurfaceRole, Hit)| (role, hit.node));
            if entered != left {
                if let Some((_, node)) = left {
                    repaint |= runtime.dispatch_ui_event(node, UiEvent::PointerExited);
                }
                if let Some(hit) = hit {
                    repaint |= runtime.dispatch_ui_event(hit.node, UiEvent::PointerEntered);
                }
            }
            state.hovered = next_hovered;
            if let Some(hit) = hit {
                repaint |= runtime.dispatch_pointer(
                    hit.node,
                    UiEvent::PointerMoved,
                    EventPoint::new((x, y), (hit.local_x, hit.local_y)),
                    (0.0, 0.0),
                );
            }
            if let Some((pressed_surface, pressed_hit, start_x, start_y, dragging)) =
                &mut state.pressed
                && *pressed_surface == surface
            {
                let delta_x = x - *start_x;
                let delta_y = y - *start_y;
                // A drag that has pulled off its handle still reports where the
                // pointer is relative to that handle, so the node keeps its own
                // frame of reference for the whole gesture.
                let local = hit_layout.local_point(&runtime.scene(), pressed_hit.node, x, y);
                let point = EventPoint::new((x, y), local);
                if !*dragging && delta_x.hypot(delta_y) >= 8.0 {
                    *dragging = true;
                    repaint |= runtime.dispatch_pointer(
                        pressed_hit.node,
                        UiEvent::DragStarted,
                        point,
                        (delta_x, delta_y),
                    );
                }
                if *dragging {
                    repaint |= runtime.dispatch_pointer(
                        pressed_hit.node,
                        UiEvent::Dragged,
                        point,
                        (delta_x, delta_y),
                    );
                }
            }
        }
        LayerEvent::PointerLeave { surface } => {
            if state
                .hovered
                .is_some_and(|(hovered_surface, _)| hovered_surface == surface)
                && let Some((_, hit)) = state.hovered.take()
            {
                repaint |= runtime.dispatch_ui_event(hit.node, UiEvent::PointerExited);
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
                &state.layer_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            if let Some(hit) = hit {
                repaint |= runtime.dispatch_wheel_event(
                    hit.node,
                    EventPoint::new((x, y), (hit.local_x, hit.local_y)),
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
                &state.layer_surfaces,
            ) else {
                return Ok(false);
            };
            let hit = hit_layout
                .hit_test(&runtime.scene(), x, y)
                .map_err(|error| error.to_string())?;
            let hit = hit.filter(|hit| runtime.accepts_pointer_button(hit.node, button));
            state.pressed = hit.map(|hit| (surface, hit, x, y, false));
            if let Some(target) = hit.and_then(|hit| runtime.key_target_for_node(hit.node)) {
                state.focused.insert(surface, target);
            } else {
                state.focused.remove(&surface);
            }
            if let Some(hit) = hit {
                // The press carries its position now, so a handler can act on
                // where it landed without waiting for a motion event first.
                repaint |= runtime.dispatch_pointer(
                    hit.node,
                    UiEvent::Pressed,
                    EventPoint::new((x, y), (hit.local_x, hit.local_y)),
                    (0.0, 0.0),
                );
            }
        }
        LayerEvent::TouchDown { .. }
        | LayerEvent::TouchMotion { .. }
        | LayerEvent::TouchUp { .. }
        | LayerEvent::TouchCancel => {
            repaint |= handle_touch_event(runtime, state, event)?;
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
                &state.layer_surfaces,
            )
            .map(|layout| layout.hit_test(&runtime.scene(), x, y))
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten();
            if let Some((pressed_surface, pressed_hit, start_x, start_y, dragging)) =
                state.pressed.take()
            {
                let local = surface_layout(
                    pressed_surface,
                    &state.layout,
                    &state.popup_surfaces,
                    &state.floating_surfaces,
                    &state.layer_surfaces,
                )
                .map(|layout| layout.local_point(&runtime.scene(), pressed_hit.node, x, y))
                .unwrap_or((x, y));
                let point = EventPoint::new((x, y), local);
                repaint |= runtime.dispatch_pointer(
                    pressed_hit.node,
                    UiEvent::Released,
                    point,
                    (0.0, 0.0),
                );
                if dragging {
                    repaint |= runtime.dispatch_pointer(
                        pressed_hit.node,
                        UiEvent::DragFinished,
                        point,
                        (x - start_x, y - start_y),
                    );
                // A click is a release over the node the press landed on, so the
                // comparison is by node rather than by the whole hit.
                } else if pressed_surface == surface
                    && hit.map(|hit| hit.node) == Some(pressed_hit.node)
                {
                    repaint |= runtime.dispatch_pointer(
                        pressed_hit.node,
                        UiEvent::Clicked,
                        point,
                        (0.0, 0.0),
                    );
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
                state.primary_root,
                &state.popup_surfaces,
                &state.floating_surfaces,
                &state.layer_surfaces,
            ) else {
                return Ok(false);
            };
            let mut focused = state.focused.get(&surface).copied();
            repaint |=
                dispatch_key_in_subtree(runtime, root, &mut focused, keysym, text.as_deref());
            match focused {
                Some(node) => {
                    state.focused.insert(surface, node);
                }
                None => {
                    state.focused.remove(&surface);
                }
            }
        }
        LayerEvent::PopupConfigure { id, width, height } => {
            if let Some(surface) = state.popup_surfaces.get_mut(&id) {
                let initial = surface.renderer.is_none();
                surface.width = width.max(1);
                surface.height = height.max(1);
                let (physical_width, physical_height) = physical_size(
                    (surface.width, surface.height),
                    client.surface_scale_120(SurfaceRole::Popup(id)),
                );
                if let Some(renderer) = &mut surface.renderer {
                    renderer.resize(physical_width, physical_height);
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
        LayerEvent::ShortcutsInhibited { active } => {
            repaint |= runtime.dispatch_shortcuts_inhibited(active);
        }
        LayerEvent::AuxScale { role, scale_120 } => {
            // A popup on a 2x screen opened from a bar on a 1x one used to be
            // rendered at the bar's scale and stretched. It has its own now.
            let surface = match role {
                SurfaceRole::Popup(id) => state.popup_surfaces.get_mut(&id),
                SurfaceRole::Floating(id) => state.floating_surfaces.get_mut(&id),
                SurfaceRole::Layer(_) => None,
            };
            if let Some(surface) = surface
                && let Some(renderer) = &mut surface.renderer
            {
                let (width, height) = physical_size((surface.width, surface.height), scale_120);
                renderer.resize(width, height);
                repaint = true;
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
                let (physical_width, physical_height) = physical_size(
                    (surface.width, surface.height),
                    client.surface_scale_120(SurfaceRole::Floating(id)),
                );
                if let Some(renderer) = &mut surface.renderer {
                    renderer.resize(physical_width, physical_height);
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
        | LayerEvent::SessionLocked
        | LayerEvent::SessionLockFinished
        | LayerEvent::SessionLockConfigure { .. }
        | LayerEvent::SessionLockSurfaceRemoved { .. }
        | LayerEvent::SessionLockFrame { .. } => {}
        LayerEvent::Screens(screens) => {
            // This client sees every output, not just the one it draws to. The
            // supervisor records the list and hands it back to every worker, so
            // each runtime's `morf.screens` follows the hotplug rather than
            // keeping the entry for a monitor that has gone away.
            tx.send(SupervisorMessage::Worker(WorkerMessage::Screens {
                output: name.to_owned(),
                screens,
            }))
            .map_err(|_| "output supervisor stopped".to_owned())?;
        }
    }
    Ok(repaint)
}
