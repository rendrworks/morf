use morf_lua::{EventPoint, Runtime, UiEvent};
use morf_wayland::LayerEvent;

use crate::surfaces::*;

// Touch handling, split out of `surface_events.rs` to keep each file inside
// the repository's 500-line limit. Touch is self-contained: it owns the
// `touches` map and shares nothing with the pointer path but the hit test.

pub(crate) fn handle_touch_event(
    runtime: &mut Runtime,
    state: &mut SurfaceEventState,
    event: LayerEvent,
) -> Result<bool, String> {
    let mut repaint = false;
    match event {
        LayerEvent::TouchDown { surface, id, x, y } => {
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
                let point = EventPoint::new((x, y), (hit.local_x, hit.local_y));
                state.touches.insert(id, (surface, hit, x, y));
                if let Some(target) = runtime.key_target_for_node(hit.node) {
                    state.focused.insert(surface, target);
                } else {
                    state.focused.remove(&surface);
                }
                repaint |= runtime.dispatch_pointer(hit.node, UiEvent::Pressed, point, (0.0, 0.0));
                repaint |= runtime.dispatch_touch_event(hit.node, UiEvent::TouchPressed, id, point);
            }
        }
        LayerEvent::TouchMotion { id, x, y, .. } => {
            if let Some((touch_surface, hit, last_x, last_y)) = state.touches.get_mut(&id) {
                *last_x = x;
                *last_y = y;
                let node = hit.node;
                let role = *touch_surface;
                let local = surface_layout(
                    role,
                    &state.layout,
                    &state.popup_surfaces,
                    &state.floating_surfaces,
                    &state.layer_surfaces,
                )
                .map(|layout| layout.local_point(&runtime.scene(), node, x, y))
                .unwrap_or((x, y));
                repaint |= runtime.dispatch_touch_event(
                    node,
                    UiEvent::TouchMoved,
                    id,
                    EventPoint::new((x, y), local),
                );
            }
        }
        LayerEvent::TouchUp { surface, id, x, y } => {
            if let Some((touch_surface, pressed_hit, _, _)) = state.touches.remove(&id) {
                let layout = surface_layout(
                    surface,
                    &state.layout,
                    &state.popup_surfaces,
                    &state.floating_surfaces,
                    &state.layer_surfaces,
                );
                let local = layout
                    .map(|layout| layout.local_point(&runtime.scene(), pressed_hit.node, x, y))
                    .unwrap_or((x, y));
                let point = EventPoint::new((x, y), local);
                repaint |= runtime.dispatch_touch_event(
                    pressed_hit.node,
                    UiEvent::TouchReleased,
                    id,
                    point,
                );
                repaint |= runtime.dispatch_pointer(
                    pressed_hit.node,
                    UiEvent::Released,
                    point,
                    (0.0, 0.0),
                );
                let hit = layout
                    .filter(|_| touch_surface == surface)
                    .map(|layout| layout.hit_test(&runtime.scene(), x, y))
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .flatten();
                // A click is a release over the node the press landed on, so it
                // is compared by node rather than by the whole hit.
                if hit.map(|hit| hit.node) == Some(pressed_hit.node) {
                    repaint |= runtime.dispatch_pointer(
                        pressed_hit.node,
                        UiEvent::Clicked,
                        point,
                        (0.0, 0.0),
                    );
                }
            }
        }
        LayerEvent::TouchCancel => {
            for (id, (_, hit, x, y)) in state.touches.drain() {
                let point = EventPoint::new((x, y), (hit.local_x, hit.local_y));
                repaint |=
                    runtime.dispatch_touch_event(hit.node, UiEvent::TouchCanceled, id, point);
                repaint |= runtime.dispatch_pointer(hit.node, UiEvent::Released, point, (0.0, 0.0));
            }
        }
        _ => {}
    }
    Ok(repaint)
}
