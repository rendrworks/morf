impl LayerClient {

    /// Requests exclusive compositor session ownership.
    pub fn begin_session_lock(&mut self) -> Result<(), WaylandError> {
        if self.state.session_lock.is_some() {
            return Err(WaylandError("session lock is already active".to_owned()));
        }
        let lock = self
            .state
            .session_locks
            .lock(&self.queue.handle())
            .map_err(|error| WaylandError(format!("session lock is unavailable: {error}")))?;
        self.state.session_lock = Some(lock);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Unlocks only after the compositor confirmed that the lock is active.
    pub fn unlock_session(&mut self) -> Result<(), WaylandError> {
        let lock = self
            .state
            .session_lock
            .take()
            .ok_or_else(|| WaylandError("session lock is not active".to_owned()))?;
        if !lock.is_locked() {
            self.state.session_lock = Some(lock);
            return Err(WaylandError(
                "session lock has not been confirmed by the compositor".to_owned(),
            ));
        }
        lock.unlock();
        self.state.lock_surfaces.clear();
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Returns one configured lock surface for rendering.
    pub fn lock_surface(&self, index: usize) -> Option<&wl_surface::WlSurface> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.surface.wl_surface())
    }

    /// Returns one lock surface's configured logical size.
    pub fn lock_size(&self, index: usize) -> Option<(u32, u32)> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.size)
    }

    /// Returns one lock surface's preferred integer scale in protocol 120ths.
    pub fn lock_scale_120(&self, index: usize) -> Option<u32> {
        self.state
            .lock_surfaces
            .get(index)
            .map(|surface| surface.scale.saturating_mul(120))
    }

    /// Returns one lock surface's physical buffer size.
    pub fn lock_physical_size(&self, index: usize) -> Option<(u32, u32)> {
        self.state.lock_surfaces.get(index).map(|surface| {
            (
                surface.size.0.saturating_mul(surface.scale),
                surface.size.1.saturating_mul(surface.scale),
            )
        })
    }

    /// Returns an owned raw-window target for one lock surface.
    pub fn lock_window_target(&self, index: usize) -> Option<WaylandWindowTarget> {
        self.lock_surface(index).map(|surface| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: surface.clone(),
        })
    }

    /// Requests a compositor frame callback for one lock surface.
    pub fn request_lock_frame(&self, index: usize) {
        let Some(surface) = self.lock_surface(index) else {
            return;
        };
        surface.frame(&self.queue.handle(), FrameCallbackData(surface.clone()));
    }

    /// Commits one lock surface without attaching a new buffer.
    pub fn commit_lock(&self, index: usize) {
        if let Some(surface) = self.lock_surface(index) {
            surface.commit();
        }
    }
}

