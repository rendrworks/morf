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

    /// Advances animations entirely in Rust.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, Error> {
        self.reactive
            .borrow_mut()
            .scene
            .tick_animations(delta)
            .map_err(|error| Error::Runtime(error.to_string()))
    }

    /// Updates the clock service signal and recomputes dependent Lua bindings.
    pub fn update_clock(&mut self, value: impl Into<String>) -> Result<(), Error> {
        let value = ScriptValue::String(value.into());
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
            .map_err(Error::Runtime)
    }

    /// Returns the number of Lua effect evaluations performed by this runtime.
    pub fn effect_runs(&self) -> u64 {
        self.reactive.borrow().effect_runs
    }

    /// Runs one bounded Lua UI handler and retains failures as runtime logs.
    pub fn dispatch_ui_event(&mut self, node: NodeHandle, event: UiEvent) -> bool {
        self.dispatch_ui_event_with_args(node, event, &[])
    }

    /// Returns compositor idle thresholds requested by Lua callbacks.
    pub fn idle_timeouts(&self) -> Vec<u32> {
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
    pub fn dispatch_idle(&mut self, timeout_ms: u32, idle: bool) -> bool {
        let callbacks = self
            .reactive
            .borrow()
            .idle_callbacks
            .get(&timeout_ms)
            .cloned()
            .unwrap_or_default();
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, &[IpcValue::Boolean(idle)], self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("idle callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending compositor output power requests.
    pub fn take_output_power_requests(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.reactive.borrow_mut().output_power_requests)
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
                    .logs
                    .push(format!("clipboard callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending output-capture requests.
    pub fn take_screencopy_requests(&mut self) -> Vec<ScreencopyRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().screencopy_requests)
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
                .logs
                .push(format!("screencopy callback: {message}"));
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
                    .logs
                    .push(format!("input method callback: {message}"));
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
                    .logs
                    .push(format!("text input callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Runs one bounded key handler with keysym and UTF-8 text arguments.
    pub fn dispatch_key_event(
        &mut self,
        node: NodeHandle,
        keysym: u32,
        text: Option<&str>,
    ) -> bool {
        self.dispatch_ui_event_with_args(
            node,
            UiEvent::KeyPressed,
            &[
                IpcValue::Integer(keysym as i64),
                text.map_or(IpcValue::Nil, |value| IpcValue::String(value.to_owned())),
            ],
        )
    }

    /// Dispatches one touch event with contact identity and surface coordinates.
    pub fn dispatch_touch_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        id: i32,
        x: f64,
        y: f64,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::TouchPressed
                | UiEvent::TouchMoved
                | UiEvent::TouchReleased
                | UiEvent::TouchCanceled
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Integer(i64::from(id)),
                IpcValue::Number(x),
                IpcValue::Number(y),
            ],
        )
    }

    /// Dispatches pointer coordinates and displacement to a movement handler.
    pub fn dispatch_pointer_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::PointerMoved | UiEvent::DragStarted | UiEvent::Dragged | UiEvent::DragFinished
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Number(x),
                IpcValue::Number(y),
                IpcValue::Number(delta_x),
                IpcValue::Number(delta_y),
            ],
        )
    }

    /// Dispatches one wheel or touchpad-axis event to a MouseArea.
    pub fn dispatch_wheel_event(
        &mut self,
        node: NodeHandle,
        position: (f64, f64),
        pixels: (f64, f64),
        steps: (i32, i32),
    ) -> bool {
        self.dispatch_ui_event_with_args(
            node,
            UiEvent::Wheel,
            &[
                IpcValue::Number(position.0),
                IpcValue::Number(position.1),
                IpcValue::Number(pixels.0),
                IpcValue::Number(pixels.1),
                IpcValue::Integer(i64::from(steps.0)),
                IpcValue::Integer(i64::from(steps.1)),
            ],
        )
    }

    /// Returns whether a MouseArea accepts one Linux input button code.
    pub fn accepts_pointer_button(&self, node: NodeHandle, button: u32) -> bool {
        let state = self.reactive.borrow();
        let Ok(value) = state.scene.current(node, "accepted_buttons") else {
            return false;
        };
        let accepted = |value: &SceneValue| match value {
            SceneValue::String(name) => match name.as_str() {
                "all" => true,
                "left" => button == 0x110,
                "right" => button == 0x111,
                "middle" => button == 0x112,
                _ => false,
            },
            SceneValue::Number(code) => *code == f64::from(button),
            _ => false,
        };
        match value {
            SceneValue::List(values) => values.iter().any(accepted),
            value => accepted(value),
        }
    }

    /// Returns the first scene node with a key handler in tree order.
    pub fn first_key_target(&self) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets(&state);
        targets
            .iter()
            .copied()
            .find(|node| state.scene.bool_value(*node, "focus").unwrap_or(false))
            .or_else(|| targets.first().copied())
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

    /// Advances keyboard focus through enabled visible key handlers.
    pub fn next_key_target(&self, current: Option<NodeHandle>) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets(&state);
        if targets.is_empty() {
            return None;
        }
        let next = current
            .and_then(|current| targets.iter().position(|node| *node == current))
            .map_or(0, |index| (index + 1) % targets.len());
        Some(targets[next])
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

    fn dispatch_ui_event_with_args(
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
            self.reactive.borrow_mut().logs.push(format!(
                "{:?}.{}: {message}",
                node,
                event.property()
            ));
        }
        true
    }
}

