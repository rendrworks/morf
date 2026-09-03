//! What a frame's layout tells the configuration.
//!
//! Split from the service loop at the line gate. Once a surface is laid
//! out, the resolved geometry feeds transform watchers, popup anchors, and
//! the bindings that read `layout_x` and its kin.

use morf_layout::Layout;

use crate::{
    reactive_bindings::*, runtime_helpers::*, scene_bindings::*, surface_types::*, types::*,
};

impl Runtime {
    /// Updates native transform watchers from one rendered surface layout.
    ///
    /// Also where a binding on `layout_width` and its kin hears that the
    /// frame moved its node: every node such a binding read is checked
    /// against the geometry it had, and the changed ones are flushed here,
    /// since nothing else would until the next event.
    pub fn observe_layout(&mut self, layout: &Layout) -> bool {
        let moved = {
            let state = self.reactive.borrow();
            state
                .property_signals
                .keys()
                .filter(|(_, property, _)| property == LAYOUT_GEOMETRY)
                .map(|(node, _, _)| *node)
                .filter(|node| {
                    layout
                        .geometry(*node)
                        .is_some_and(|now| state.transform_tracker.geometry(*node) != Some(now))
                })
                .collect::<Vec<_>>()
        };
        let mut state = self.reactive.borrow_mut();
        state.transform_tracker.update(layout);
        for node in &moved {
            let _ = bump_property_signal(&mut state, *node, LAYOUT_GEOMETRY, false);
        }
        drop(state);
        if !moved.is_empty()
            && let Err(message) = self
                .lua
                .enter(|ctx| flush_reactive(&self.reactive, ctx, self.limits))
        {
            self.reactive
                .borrow_mut()
                .log(LogLevel::Warn, format!("layout binding: {message}"));
        }
        let mut state = self.reactive.borrow_mut();
        let anchors = state
            .popup_node_anchors
            .iter()
            .map(|(id, anchor)| (*id, anchor.clone()))
            .collect::<Vec<_>>();
        for (id, anchor) in anchors {
            let Some(geometry) = state.transform_tracker.geometry(anchor.node) else {
                continue;
            };
            let node_width = geometry_i32(geometry.width).max(1);
            let node_height = geometry_i32(geometry.height).max(1);
            let resolved = (
                geometry_i32(geometry.x)
                    .saturating_add(anchor.x)
                    .saturating_sub(anchor.margin_left),
                geometry_i32(geometry.y)
                    .saturating_add(anchor.y)
                    .saturating_sub(anchor.margin_top),
                anchor
                    .width
                    .unwrap_or(node_width)
                    .saturating_add(anchor.margin_left)
                    .saturating_add(anchor.margin_right)
                    .max(1),
                anchor
                    .height
                    .unwrap_or(node_height)
                    .saturating_add(anchor.margin_top)
                    .saturating_add(anchor.margin_bottom)
                    .max(1),
            );
            if let Some(WindowSurfaceConfig {
                kind: WindowSurfaceKind::Popup(config),
                ..
            }) = state.window_surfaces.get_mut(&id)
                && (
                    config.anchor_x,
                    config.anchor_y,
                    config.anchor_width,
                    config.anchor_height,
                ) != resolved
            {
                config.anchor_x = resolved.0;
                config.anchor_y = resolved.1;
                config.anchor_width = resolved.2;
                config.anchor_height = resolved.3;
                state.window_surfaces_changed = true;
            }
        }
        let mut watchers = std::mem::take(&mut state.transform_watchers);
        let mut changed = false;
        for watcher in watchers.values_mut() {
            match watcher
                .watcher
                .observe(&state.scene, &state.transform_tracker)
            {
                Ok(true) => {
                    watcher.revision = watcher.revision.wrapping_add(1);
                    watcher.pending = true;
                    changed = true;
                }
                Ok(false) => {}
                Err(error) => state.log(LogLevel::Warn, format!("transform watcher: {error}")),
            }
        }
        state.transform_watchers = watchers;
        changed
    }
}
