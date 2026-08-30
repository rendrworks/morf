impl LayerClient {

    /// Sends one evdev keycode through the compositor virtual keyboard protocol.
    pub fn send_virtual_key(&mut self, keycode: u32, pressed: bool) -> bool {
        self.state.refresh_virtual_keyboard(&self.queue.handle());
        let Some(keyboard) = &self.state.virtual_keyboard else {
            return false;
        };
        if self.connection.flush().is_err() {
            return false;
        }
        let time = self
            .state
            .virtual_keyboard_clock
            .elapsed()
            .as_millis()
            .min(u32::MAX as u128) as u32;
        keyboard.key(time, keycode, u32::from(pressed));
        true
    }

    /// Sends virtual keyboard modifier masks and layout group.
    pub fn send_virtual_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> bool {
        self.state.refresh_virtual_keyboard(&self.queue.handle());
        let Some(keyboard) = &self.state.virtual_keyboard else {
            return false;
        };
        if self.connection.flush().is_err() {
            return false;
        }
        keyboard.modifiers(depressed, latched, locked, group);
        true
    }

    /// Returns whether a virtual keyboard was created for the current seat.
    pub fn supports_virtual_keyboard(&self) -> bool {
        self.state.virtual_keyboard_manager.is_some()
            && self.state.seats.seats().next().is_some()
            && self.state.virtual_keyboard_keymap.is_some()
    }

    /// Claims the compositor input-method role for the current seat.
    pub fn enable_input_method(&mut self) -> bool {
        if self.state.input_method.is_some() {
            return true;
        }
        let Some(manager) = &self.state.input_method_manager else {
            return false;
        };
        let Some(seat) = self.state.seats.seats().next() else {
            return false;
        };
        self.state.input_method = Some(manager.get_input_method(&seat, &self.queue.handle(), ()));
        true
    }

    /// Returns whether the compositor exposes input-method-v2 for a seat.
    pub fn supports_input_method(&self) -> bool {
        self.state.input_method_manager.is_some() && self.state.seats.seats().next().is_some()
    }

    /// Commits UTF-8 text through the active input-method context.
    pub fn input_method_commit(&self, text: &str) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.commit_string(text.to_owned());
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Replaces the active preedit string and cursor range.
    pub fn input_method_preedit(&self, text: &str, begin: i32, end: i32) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.set_preedit_string(text.to_owned(), begin, end);
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Deletes byte ranges around the application cursor.
    pub fn input_method_delete(&self, before: u32, after: u32) -> bool {
        let Some(input_method) = &self.state.input_method else {
            return false;
        };
        input_method.delete_surrounding_text(before, after);
        input_method.commit(self.state.input_method_state.serial);
        true
    }

    /// Creates and requests text-input-v3 for the current seat.
    pub fn enable_text_input(&mut self) -> bool {
        self.state.text_input_requested = true;
        if self.state.text_input.is_some() {
            return true;
        }
        let Some(manager) = &self.state.text_input_manager else {
            return false;
        };
        let Some(seat) = self.state.seats.seats().next() else {
            return false;
        };
        self.state.text_input = Some(manager.get_text_input(&seat, &self.queue.handle(), ()));
        true
    }

    /// Disables text-input-v3 for the focused surface.
    pub fn disable_text_input(&mut self) -> bool {
        self.state.text_input_requested = false;
        let Some(text_input) = &self.state.text_input else {
            return false;
        };
        text_input.disable();
        text_input.commit();
        true
    }

    /// Sends surrounding UTF-8 text and byte offsets.
    pub fn set_text_input_surrounding(&self, text: &str, cursor: i32, anchor: i32) -> bool {
        let Some(text_input) = &self.state.text_input else {
            return false;
        };
        text_input.set_surrounding_text(text.to_owned(), cursor, anchor);
        text_input.commit();
        true
    }

    /// Sends raw content hint flags and purpose values.
    pub fn set_text_input_content_type(&self, hints: u32, purpose: u32) -> bool {
        let Some(text_input) = &self.state.text_input else {
            return false;
        };
        let Some(hints) = zwp_text_input_v3::ContentHint::from_bits(hints) else {
            return false;
        };
        let Ok(purpose) = zwp_text_input_v3::ContentPurpose::try_from(purpose) else {
            return false;
        };
        text_input.set_content_type(hints, purpose);
        text_input.commit();
        true
    }

    /// Sends the surface-local cursor rectangle.
    pub fn set_text_input_cursor_rect(&self, rect: InputRect) -> bool {
        let Some(text_input) = &self.state.text_input else {
            return false;
        };
        text_input.set_cursor_rectangle(rect.x, rect.y, rect.width, rect.height);
        text_input.commit();
        true
    }

    /// Returns whether text-input-v3 is available for the current seat.
    pub fn supports_text_input(&self) -> bool {
        self.state.text_input_manager.is_some() && self.state.seats.seats().next().is_some()
    }
}

