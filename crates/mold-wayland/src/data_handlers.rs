impl DataDeviceHandler for LayerState {
    fn enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
        _surface: &wl_surface::WlSurface,
    ) {
    }

    fn leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
    ) {
    }

    fn motion(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }

    fn selection(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        data_device: &wl_data_device::WlDataDevice,
    ) {
        let Some(offer) = self
            .data_devices
            .iter()
            .find(|device| device.inner() == data_device)
            .and_then(|device| device.data().selection_offer())
        else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        let mime = offer.with_mime_types(|types| {
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                .into_iter()
                .find(|preferred| types.iter().any(|mime| mime == preferred))
                .map(str::to_owned)
        });
        let Some(mime) = mime else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        let Ok(pipe) = offer.receive(mime) else {
            self.events.push_back(LayerEvent::Clipboard { text: None });
            return;
        };
        if self
            .clipboard_reads
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                (active < 8).then_some(active + 1)
            })
            .is_err()
        {
            return;
        }
        let tx = self.clipboard_tx.clone();
        let active = Arc::clone(&self.clipboard_reads);
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let text = pipe
                .take(1_048_577)
                .read_to_end(&mut bytes)
                .ok()
                .filter(|_| bytes.len() <= 1_048_576)
                .and_then(|_| String::from_utf8(bytes).ok());
            let _ = tx.send(text);
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }

    fn drop_performed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _data_device: &wl_data_device::WlDataDevice,
    ) {
    }
}

impl DataOfferHandler for LayerState {
    fn source_actions(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _offer: &mut DragOffer,
        _actions: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

impl DataSourceHandler for LayerState {
    fn accept_mime(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
        _mime: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wl_data_source::WlDataSource,
        _mime: String,
        mut pipe: WritePipe,
    ) {
        let Some(text) = self
            .clipboard_source
            .as_ref()
            .filter(|current| current.inner() == source)
            .map(|_| self.clipboard_text.clone())
        else {
            return;
        };
        if self
            .clipboard_writes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |active| {
                (active < 8).then_some(active + 1)
            })
            .is_err()
        {
            return;
        }
        let active = Arc::clone(&self.clipboard_writes);
        thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }

    fn cancelled(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        source: &wl_data_source::WlDataSource,
    ) {
        if self
            .clipboard_source
            .as_ref()
            .is_some_and(|current| current.inner() == source)
        {
            self.clipboard_source = None;
        }
    }

    fn dnd_dropped(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
    ) {
    }

    fn dnd_finished(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
    ) {
    }

    fn action(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _source: &wl_data_source::WlDataSource,
        _action: wayland_client::protocol::wl_data_device_manager::DndAction,
    ) {
    }
}

