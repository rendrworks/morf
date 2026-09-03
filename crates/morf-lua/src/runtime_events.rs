use std::cell::{Ref, RefMut};

use morf_scene::{NodeHandle, Scene};

use crate::{
    events::*, reactive_bindings::*, reactive_execute::*, runtime_helpers::*, surface_types::*,
    types::*,
};

impl Runtime {
    /// Borrows the scene produced by executed configuration code.
    pub fn scene(&self) -> Ref<'_, Scene> {
        Ref::map(self.reactive.borrow(), |state| &state.scene)
    }

    /// Mutably borrows the scene for frame-pipeline structural operations.
    pub fn scene_mut(&mut self) -> RefMut<'_, Scene> {
        RefMut::map(self.reactive.borrow_mut(), |state| &mut state.scene)
    }

    /// Drains parent transitions queued by Lua handlers.
    pub fn take_parent_transitions(&mut self) -> Vec<ParentTransitionRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().parent_transitions)
    }

    /// Updates the clock service signal and recomputes dependent Lua bindings.
    ///
    /// Returns whether the scene actually changed. The clock ticks once a
    /// second whether or not anything reads it, and a repaint of a shell that
    /// shows no time is pure cost — a full tessellation and GPU submit per
    /// output per second. So the answer is the same one `poll_services` gives:
    /// not "did a signal move" but "did the scene".
    pub fn update_clock(&mut self, value: impl Into<String>) -> Result<bool, Error> {
        let revision_before = self.reactive.borrow().scene_revision;
        let value = IpcValue::String(value.into());
        {
            let mut state = self.reactive.borrow_mut();
            let clock = state.clock;
            state
                .graph
                .as_mut()
                .ok_or_else(|| Error::Runtime("reactive graph is already running".to_owned()))?
                .write(clock, value.clone())
                .map_err(|error| Error::Runtime(error.to_string()))?;
            state.values.insert(clock, value);
        }
        self.lua
            .enter(|ctx| flush_reactive(&self.reactive, ctx, self.limits))
            .map_err(Error::Runtime)?;
        Ok(self.reactive.borrow().scene_revision != revision_before)
    }

    /// Returns the number of Lua effect evaluations performed by this runtime.
    pub fn effect_runs(&self) -> u64 {
        self.reactive.borrow().effect_runs
    }

    /// Runs one bounded Lua UI handler and retains failures as runtime logs.
    pub fn dispatch_ui_event(&mut self, node: NodeHandle, event: UiEvent) -> bool {
        self.dispatch_ui_event_with_args(node, event, &[])
    }

    /// Returns compositor idle thresholds requested by Lua callbacks, each
    /// with whether it should ignore idle inhibitors.
    pub fn idle_timeouts(&self) -> Vec<(u32, bool)> {
        let mut timeouts = self
            .reactive
            .borrow()
            .idle_callbacks
            .keys()
            .copied()
            .collect::<Vec<_>>();
        timeouts.sort_unstable();
        timeouts
    }

    /// Dispatches one compositor idle state change to registered Lua callbacks.
    pub fn dispatch_idle(&mut self, timeout_ms: u32, input_only: bool, idle: bool) -> bool {
        let callbacks = self
            .reactive
            .borrow()
            .idle_callbacks
            .get(&(timeout_ms, input_only))
            .cloned()
            .unwrap_or_default();
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, &[IpcValue::Boolean(idle)], self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("idle callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending compositor output power requests.
    pub fn take_output_power_requests(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.reactive.borrow_mut().output_power_requests)
    }

    /// Takes a pending change to whether the session is being held awake.
    pub fn take_idle_inhibit_change(&mut self) -> Option<bool> {
        let mut state = self.reactive.borrow_mut();
        state.idle_inhibit_changed.then(|| {
            state.idle_inhibit_changed = false;
            state.idle_inhibited
        })
    }

    /// Takes a pending change to whether the shell wants the compositor's
    /// shortcuts held off it.
    pub fn take_shortcuts_inhibit_change(&mut self) -> Option<bool> {
        let mut state = self.reactive.borrow_mut();
        state.shortcuts_inhibit_changed.then(|| {
            state.shortcuts_inhibit_changed = false;
            state.shortcuts_inhibited
        })
    }

    /// Delivers the compositor's answer to that request.
    pub fn dispatch_shortcuts_inhibited(&mut self, active: bool) -> bool {
        let callbacks = self.reactive.borrow().shortcuts_callbacks.clone();
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, &[IpcValue::Boolean(active)], self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("shortcuts callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending compositor clipboard publications.
    pub fn take_clipboard_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reactive.borrow_mut().clipboard_requests)
    }

    /// Dispatches a compositor clipboard selection to registered Lua callbacks.
    pub fn dispatch_clipboard(&mut self, text: Option<String>) -> bool {
        let callbacks = self.reactive.borrow().clipboard_callbacks.clone();
        let value = text.map_or(IpcValue::Nil, IpcValue::String);
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, std::slice::from_ref(&value), self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("clipboard callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending output-capture requests.
    pub fn take_screencopy_requests(&mut self) -> Vec<ScreencopyRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().screencopy_requests)
    }

    /// Takes the name a capture asked to be published under, if it chose one.
    pub fn take_screencopy_name(&mut self, request_id: u64) -> Option<String> {
        self.reactive
            .borrow_mut()
            .screencopy_names
            .remove(&request_id)
    }

    /// Takes the published captures a configuration has released.
    ///
    /// Each is a source string as `frame.source` gave it, or the bare name.
    pub fn take_screencopy_releases(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reactive.borrow_mut().screencopy_releases)
    }

    /// Dispatches one output capture to its requesting Lua callback.
    pub fn dispatch_screencopy(
        &mut self,
        request_id: u64,
        result: Result<Screencopy, String>,
    ) -> bool {
        let Some(callback) = self
            .reactive
            .borrow_mut()
            .screencopy_callbacks
            .remove(&request_id)
        else {
            return false;
        };
        if let Err(message) = self
            .lua
            .enter(|ctx| execute_screencopy_handler(ctx, &callback, result, self.limits))
        {
            self.reactive
                .borrow_mut()
                .log(LogLevel::Warn, format!("screencopy callback: {message}"));
        }
        true
    }

    /// Takes pending virtual keyboard protocol requests.
    pub fn take_virtual_keyboard_requests(&mut self) -> Vec<VirtualKeyboardRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().virtual_keyboard_requests)
    }

    /// Takes whether Lua requested the compositor input-method role.
    pub fn take_input_method_enable_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().input_method_enable_requested)
    }

    /// Takes pending input-method protocol requests.
    pub fn take_input_method_requests(&mut self) -> Vec<InputMethodRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().input_method_requests)
    }

    /// Dispatches an atomically committed input-method context to Lua.
    pub fn dispatch_input_method(
        &mut self,
        active: bool,
        surrounding_text: Option<String>,
        cursor: u32,
        anchor: u32,
        serial: u32,
    ) -> bool {
        let callbacks = self.reactive.borrow().input_method_callbacks.clone();
        let args = [
            IpcValue::Boolean(active),
            surrounding_text.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(cursor)),
            IpcValue::Integer(i64::from(anchor)),
            IpcValue::Integer(i64::from(serial)),
        ];
        for callback in &callbacks {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("input method callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes whether Lua requested text-input-v3 creation.
    pub fn take_text_input_enable_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().text_input_enable_requested)
    }

    /// Takes pending text-input-v3 state requests.
    pub fn take_text_input_requests(&mut self) -> Vec<TextInputRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().text_input_requests)
    }

    /// Dispatches one atomically committed text-input edit batch to Lua.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_text_input(
        &mut self,
        focused: bool,
        preedit: Option<String>,
        preedit_begin: i32,
        preedit_end: i32,
        commit: Option<String>,
        delete_before: u32,
        delete_after: u32,
        serial: u32,
    ) -> bool {
        let callbacks = self.reactive.borrow().text_input_callbacks.clone();
        let args = [
            IpcValue::Boolean(focused),
            preedit.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(preedit_begin)),
            IpcValue::Integer(i64::from(preedit_end)),
            commit.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(delete_before)),
            IpcValue::Integer(i64::from(delete_after)),
            IpcValue::Integer(i64::from(serial)),
        ];
        for callback in &callbacks {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("text input callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Returns the first key handler within one scene root.
    pub fn first_key_target_in(&self, root: NodeHandle) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets_in(&state, root);
        targets
            .iter()
            .copied()
            .find(|node| state.scene.bool_value(*node, "focus").unwrap_or(false))
            .or_else(|| targets.first().copied())
    }

    /// Returns the nearest key-handling ancestor of a hit-tested node.
    pub fn key_target_for_node(&self, node: NodeHandle) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let mut current = Some(node);
        while let Some(node) = current {
            if state.handlers.contains_key(&(node, UiEvent::KeyPressed))
                && state.scene.bool_value(node, "enabled").unwrap_or(false)
                && state.scene.bool_value(node, "visible").unwrap_or(false)
            {
                return Some(node);
            }
            current = state.scene.parent(node).ok().flatten();
        }
        None
    }

    /// Returns whether a node belongs to the subtree rooted at `root`.
    pub fn node_in_subtree(&self, root: NodeHandle, node: NodeHandle) -> bool {
        let state = self.reactive.borrow();
        scene_node_in_subtree(&state.scene, root, node)
    }

    /// Advances keyboard focus within one scene root.
    pub fn next_key_target_in(
        &self,
        root: NodeHandle,
        current: Option<NodeHandle>,
    ) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets_in(&state, root);
        if targets.is_empty() {
            return None;
        }
        let next = current
            .and_then(|current| targets.iter().position(|node| *node == current))
            .map_or(0, |index| (index + 1) % targets.len());
        Some(targets[next])
    }

    pub(crate) fn dispatch_ui_event_with_args(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        args: &[IpcValue],
    ) -> bool {
        let handler = self.reactive.borrow().handlers.get(&(node, event)).cloned();
        let Some(handler) = handler else {
            return false;
        };
        let result = self
            .lua
            .enter(|ctx| execute_handler_args(ctx, &handler, args, self.limits));
        if let Err(message) = result {
            self.reactive.borrow_mut().log(
                LogLevel::Warn,
                format!("{:?}.{}: {message}", node, event.property()),
            );
        }
        true
    }
}
