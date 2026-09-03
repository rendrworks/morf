use std::time::Duration;

use morf_io::Timer as IoTimer;
use morf_layout::Layout;
use morf_scene::{NodeHandle, Value as SceneValue};

use crate::{
    reactive_execute::*, runtime_helpers::*, scene_bindings::*, state::*, surface_types::*,
    types::*, views::*,
};

impl Runtime {
    /// Updates native transform watchers from one rendered surface layout.
    pub fn observe_layout(&self, layout: &Layout) -> bool {
        let mut state = self.reactive.borrow_mut();
        state.transform_tracker.update(layout);
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

    /// Polls native service jobs and runs completed callbacks with bounded fuel.
    pub fn poll_services(&mut self) -> bool {
        let mut ready = Vec::new();
        let mut timers = Vec::new();
        let mut dbus_signals = Vec::new();
        let mut dbus_calls = Vec::new();
        let mut udev_events = Vec::new();
        let mut status_updates = Vec::new();
        let mut loaders = Vec::new();
        let mut loader_drops = Vec::new();
        let mut retained_destroys = Vec::new();
        let mut transform_callbacks = Vec::new();
        let mut service_changed = false;
        {
            let mut state = self.reactive.borrow_mut();
            let mut index = 0;
            while index < state.pam_tasks.len() {
                let result = state.pam_tasks[index].task.wait(Duration::ZERO);
                if let Some(result) = result {
                    let task = state.pam_tasks.swap_remove(index);
                    ready.push((task.callback, task.unlock_on_success, result));
                } else {
                    index += 1;
                }
            }
            let timer_definitions = state
                .timer_callbacks
                .iter()
                .map(|(node, callback)| (*node, callback.clone()))
                .collect::<Vec<_>>();
            let mut stale_timers = Vec::new();
            for (node, callback) in timer_definitions {
                let Ok(running) = state.scene.bool_value(node, "running") else {
                    stale_timers.push(node);
                    continue;
                };
                let interval = state.scene.number(node, "interval").unwrap_or(0.0);
                let repeat = state.scene.bool_value(node, "repeat").unwrap_or(false);
                let duration = (interval.is_finite() && interval > 0.0)
                    .then(|| Duration::from_secs_f64(interval / 1_000.0));
                let current = state
                    .timers
                    .iter()
                    .position(|timer| timer.node == Some(node));
                if !running || duration.is_none() {
                    if let Some(index) = current {
                        state.timers.swap_remove(index);
                        service_changed = true;
                    }
                    continue;
                }
                let duration = duration.expect("validated duration");
                let matches = current.is_some_and(|index| {
                    state.timers[index].interval == duration && state.timers[index].repeat == repeat
                });
                if matches {
                    continue;
                }
                if let Some(index) = current {
                    state.timers.swap_remove(index);
                }
                match IoTimer::every(duration) {
                    Ok(timer) => state.timers.push(PendingTimer {
                        timer,
                        callback,
                        repeat,
                        interval: duration,
                        node: Some(node),
                    }),
                    Err(error) => state.log(LogLevel::Warn, format!("Timer: {error}")),
                }
                service_changed = true;
            }
            for node in stale_timers {
                state.timer_callbacks.remove(&node);
                state.timers.retain(|timer| timer.node != Some(node));
            }
            let loader_definitions = state
                .loader_factories
                .iter()
                .map(|(node, factory)| (*node, factory.clone()))
                .collect::<Vec<_>>();
            let mut stale_loaders = Vec::new();
            for (node, factory) in loader_definitions {
                let Ok(active) = state.scene.bool_value(node, "active") else {
                    stale_loaders.push(node);
                    continue;
                };
                let loading = state.scene.bool_value(node, "loading").unwrap_or(false);
                let active_async = state
                    .scene
                    .bool_value(node, "active_async")
                    .unwrap_or(false);
                let requested = active || loading || active_async;
                if requested && state.loaded_loaders.insert(node) {
                    loaders.push((node, factory));
                } else if !requested && state.loaded_loaders.remove(&node) {
                    loader_drops.extend_from_slice(state.scene.children(node).unwrap_or_default());
                    service_changed = true;
                }
            }
            for node in stale_loaders {
                state.loader_factories.remove(&node);
                state.loaded_loaders.remove(&node);
            }
            let mut index = 0;
            while index < state.timers.len() {
                if state.timers[index].timer.tick(Duration::ZERO) {
                    timers.push(state.timers[index].callback.clone());
                    if !state.timers[index].repeat {
                        if let Some(node) = state.timers[index].node {
                            let _ = assign_scene_property(
                                &mut state,
                                node,
                                "running",
                                SceneValue::Bool(false),
                            );
                        }
                        state.timers.swap_remove(index);
                        continue;
                    }
                }
                index += 1;
            }
            for subscription in &state.dbus_signals {
                while let Some(value) = subscription.signal.next_value(Duration::ZERO) {
                    dbus_signals.push((subscription.callback.clone(), value));
                }
            }
            // Bounded per frame, unlike the signal drain above. A signal that
            // arrives faster than it is read is the sender's problem; a *call*
            // that does is ours, because the caller is blocked until we answer
            // and answering happens after this loop. Taking them all would let
            // one chatty peer hold the frame open.
            // How many calls one service may hand over per frame.
            const MAX_CALLS_PER_FRAME: usize = 32;
            for entry in &state.dbus_services {
                for _ in 0..MAX_CALLS_PER_FRAME {
                    let Some(call) = entry.service.borrow_mut().next_call(Duration::ZERO) else {
                        break;
                    };
                    dbus_calls.push((entry.callback.clone(), call));
                }
            }
            let mut udev_errors = Vec::new();
            for subscription in &mut state.udev_monitors {
                for _ in 0..32 {
                    match subscription.monitor.next_event(Duration::ZERO) {
                        Ok(Some(event)) => {
                            udev_events.push((subscription.callback.clone(), event));
                        }
                        Ok(None) => break,
                        Err(error) => {
                            udev_errors.push(error.to_string());
                            break;
                        }
                    }
                }
            }
            for error in udev_errors {
                state.log(LogLevel::Warn, format!("udev: {error}"));
            }
            let mut status_errors = Vec::new();
            for subscription in &mut state.status_notifiers {
                match subscription.host.poll_changed() {
                    Ok(Some(items)) => status_updates.push((subscription.callback.clone(), items)),
                    Ok(None) => {}
                    Err(error) => status_errors.push(error.to_string()),
                }
            }
            for error in status_errors {
                state.log(LogLevel::Warn, format!("status notifier: {error}"));
            }
            retained_destroys.extend(state.retained_destroy_queue.drain());
            for watcher in state.transform_watchers.values_mut() {
                if watcher.pending {
                    watcher.pending = false;
                    if let Some(callback) = &watcher.callback {
                        transform_callbacks.push((callback.clone(), watcher.revision));
                    }
                }
            }
        }
        for node in retained_destroys {
            self.lua
                .enter(|ctx| finish_retained_destroy(&self.reactive, ctx, self.limits, node));
            service_changed = true;
        }
        for &node in &loader_drops {
            self.lua
                .enter(|ctx| drop_retainable(&self.reactive, ctx, self.limits, node));
        }
        for (node, factory) in loaders {
            let result = self
                .lua
                .enter(|ctx| execute_node_factory(ctx, &factory, self.limits));
            match result {
                Ok(child) => {
                    let mut state = self.reactive.borrow_mut();
                    if state.scene.reparent(child, Some(node)).is_ok() {
                        let _ = assign_scene_property(
                            &mut state,
                            node,
                            "active",
                            SceneValue::Bool(true),
                        );
                        let _ = assign_scene_property(
                            &mut state,
                            node,
                            "loading",
                            SceneValue::Bool(false),
                        );
                        let _ = assign_scene_property(
                            &mut state,
                            node,
                            "active_async",
                            SceneValue::Bool(false),
                        );
                        service_changed = true;
                    } else {
                        remove_scene_subtree(&mut state, child);
                        state.loaded_loaders.remove(&node);
                    }
                }
                Err(error) => {
                    let mut state = self.reactive.borrow_mut();
                    state.loaded_loaders.remove(&node);
                    let _ =
                        assign_scene_property(&mut state, node, "loading", SceneValue::Bool(false));
                    let _ = assign_scene_property(
                        &mut state,
                        node,
                        "active_async",
                        SceneValue::Bool(false),
                    );
                    state.log(LogLevel::Warn, format!("Loader: {error}"));
                }
            }
        }
        // Whether a repaint is owed is decided after the callbacks below have
        // run, by asking whether the scene actually changed. A callback merely
        // firing is not a reason to render: a 16ms timer that polls a file and
        // finds it unchanged would otherwise force a full render of every
        // output sixty times a second, forever.
        let revision_before = self.reactive.borrow().scene_revision;
        let service_changed = service_changed || !transform_callbacks.is_empty();
        for (callback, unlock_on_success, result) in ready {
            if unlock_on_success && result.is_ok() {
                self.reactive.borrow_mut().session_unlock_requested = true;
            }
            let args = match result {
                Ok(()) => vec![IpcValue::Boolean(true), IpcValue::Nil],
                Err(error) => vec![
                    IpcValue::Boolean(false),
                    IpcValue::String(error.to_string()),
                ],
            };
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, &callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("PAM callback: {message}"));
            }
        }
        for callback in timers {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, &callback, &[], self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("timer callback: {message}"));
            }
        }
        for (callback, revision) in transform_callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(
                    ctx,
                    &callback,
                    &[IpcValue::Integer(revision as i64)],
                    self.limits,
                )
            }) {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("transform callback: {message}"));
            }
        }
        for (callback, call) in dbus_calls {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_dbus_call_handler(ctx, &callback, call, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("D-Bus call: {message}"));
            }
        }
        for (callback, value) in dbus_signals {
            let value = match value {
                Ok(value) => value,
                Err(message) => {
                    self.reactive
                        .borrow_mut()
                        .log(LogLevel::Warn, format!("D-Bus signal: {message}"));
                    continue;
                }
            };
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_dbus_handler(ctx, &callback, value, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("D-Bus callback: {message}"));
            }
        }
        for (callback, event) in udev_events {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_dbus_handler(ctx, &callback, udev_event_value(event), self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("udev callback: {message}"));
            }
        }
        for (callback, items) in status_updates {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_dbus_handler(ctx, &callback, status_notifier_value(items), self.limits)
            }) {
                self.reactive.borrow_mut().log(
                    LogLevel::Warn,
                    format!("status notifier callback: {message}"),
                );
            }
        }
        service_changed || self.reactive.borrow().scene_revision != revision_before
    }
}

impl Runtime {
    /// Takes the nodes destroyed since the last frame, and drops what this
    /// crate holds for them on the way past.
    ///
    /// The transform tracker is reachable from here; the caches in the render
    /// backend are not, so the list is handed back for the caller to finish
    /// the job. Nobody else has both the scene and those caches in scope.
    pub fn take_removed_nodes(&self) -> Vec<NodeHandle> {
        let mut state = self.reactive.borrow_mut();
        let removed = state.scene.take_removed_nodes();
        if !removed.is_empty() {
            let ReactiveState {
                scene,
                transform_tracker,
                ..
            } = &mut *state;
            transform_tracker.retain_scene(&*scene);
        }
        removed
    }
}
