impl SeatHandler for LayerState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
    ) {
        self.refresh_data_devices(qh);
        self.refresh_idle(qh);
    }

    fn new_capability(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(qh, &seat).ok();
            if self.pointer.is_some() {
                self.pointer_seat = Some(seat.clone());
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seats.get_touch(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
            self.pointer_seat = None;
        }
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
        if capability == Capability::Touch
            && let Some(touch) = self.touch.take()
        {
            touch.release();
            self.touch_points.clear();
        }
    }

    fn remove_seat(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
    ) {
        self.data_devices
            .retain(|device| device.data().seat() != &seat);
    }
}

impl PointerHandler for LayerState {
    fn pointer_frame(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(surface) = self.surface_role(&event.surface) else {
                continue;
            };
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => self
                    .events
                    .push_back(LayerEvent::PointerMotion { surface, x, y }),
                PointerEventKind::Leave { .. } => {
                    self.events.push_back(LayerEvent::PointerLeave { surface });
                }
                PointerEventKind::Press { button, serial, .. } => {
                    self.latest_input_serial = Some(serial);
                    self.events.push_back(LayerEvent::PointerButton {
                        surface,
                        button,
                        pressed: true,
                        x,
                        y,
                    });
                }
                PointerEventKind::Release { button, .. } => {
                    self.events.push_back(LayerEvent::PointerButton {
                        surface,
                        button,
                        pressed: false,
                        x,
                        y,
                    });
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } => {
                    if !horizontal.is_none() || !vertical.is_none() {
                        self.events.push_back(LayerEvent::PointerAxis {
                            surface,
                            x,
                            y,
                            horizontal: horizontal.absolute,
                            vertical: vertical.absolute,
                            horizontal_steps: horizontal.discrete,
                            vertical_steps: vertical.discrete,
                        });
                    }
                }
            }
        }
    }
}

impl TouchHandler for LayerState {
    fn down(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let Some(surface) = self.surface_role(&surface) else {
            return;
        };
        self.latest_input_serial = Some(serial);
        self.touch_points.insert(id, (position, surface));
        self.events.push_back(LayerEvent::TouchDown {
            surface,
            id,
            x: position.0,
            y: position.1,
        });
    }

    fn up(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        id: i32,
    ) {
        if let Some(((x, y), surface)) = self.touch_points.remove(&id) {
            self.events
                .push_back(LayerEvent::TouchUp { surface, id, x, y });
        }
    }

    fn motion(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        id: i32,
        position: (f64, f64),
    ) {
        if let Some((point, surface)) = self.touch_points.get_mut(&id) {
            *point = position;
            self.events.push_back(LayerEvent::TouchMotion {
                surface: *surface,
                id,
                x: position.0,
                y: position.1,
            });
        }
    }

    fn shape(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
    ) {
        self.touch_points.clear();
        self.events.push_back(LayerEvent::TouchCancel);
    }
}

impl KeyboardHandler for LayerState {
    fn enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        self.keyboard_surface = self.surface_role(surface);
    }

    fn leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        if self.surface_role(surface) == self.keyboard_surface {
            self.keyboard_surface = None;
        }
    }

    fn press_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.latest_input_serial = Some(serial);
        self.push_key(event, true, false);
    }

    fn repeat_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.push_key(event, true, true);
    }

    fn release_key(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.push_key(event, false, false);
    }

    fn update_modifiers(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
        self.latest_input_serial = Some(serial);
        self.events.push_back(LayerEvent::Modifiers {
            surface: self.keyboard_surface.unwrap_or(SurfaceRole::Layer),
            control: modifiers.ctrl,
            alt: modifiers.alt,
            shift: modifiers.shift,
            logo: modifiers.logo,
        });
    }

    fn update_keymap(
        &mut self,
        connection: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        keymap: Keymap<'_>,
    ) {
        let keymap = keymap.as_string();
        self.virtual_keyboard_keymap = Some(keymap.clone());
        if let Some(keyboard) = &self.virtual_keyboard
            && let Ok(file) = install_virtual_keymap(keyboard, &keymap)
            && connection.flush().is_ok()
        {
            self.virtual_keyboard_keymap_file = Some(file);
        }
    }
}

