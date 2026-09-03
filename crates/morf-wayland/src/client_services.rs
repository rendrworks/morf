use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use std::time::Duration;

use crate::{state_types::*, surface_types::*, types::*};

impl LayerClient {
    /// Blocks until at least one Wayland event is dispatched.
    pub fn dispatch(&mut self) -> Result<(), WaylandError> {
        self.queue
            .blocking_dispatch(&mut self.state)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))?;
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))?;
        Ok(())
    }

    /// Dispatches Wayland events or returns when the timeout expires.
    pub fn dispatch_timeout(&mut self, timeout: Duration) -> Result<bool, WaylandError> {
        if self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))?
            > 0
        {
            return Ok(true);
        }
        self.queue
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))?;
        let Some(guard) = self.queue.prepare_read() else {
            return self
                .queue
                .dispatch_pending(&mut self.state)
                .map(|count| count > 0)
                .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")));
        };
        let seconds = timeout.as_secs().min(i64::MAX as u64) as i64;
        let timeout = Timespec {
            tv_sec: seconds,
            tv_nsec: timeout.subsec_nanos() as i64,
        };
        let mut fds = [PollFd::new(&self.queue, PollFlags::IN)];
        let ready = poll(&mut fds, Some(&timeout))
            .map_err(|error| WaylandError(format!("Wayland poll failed: {error}")))?;
        if ready == 0 {
            drop(guard);
            return Ok(false);
        }
        guard
            .read()
            .map_err(|error| WaylandError(format!("Wayland read failed: {error}")))?;
        self.queue
            .dispatch_pending(&mut self.state)
            .map(|count| count > 0)
            .map_err(|error| WaylandError(format!("Wayland dispatch failed: {error}")))
    }

    /// Replaces seat idle thresholds and returns whether the compositor supports them.
    pub fn set_idle_timeouts(&mut self, timeouts: &[(u32, bool)]) -> bool {
        self.state.idle_timeouts = timeouts.iter().copied().take(64).collect();
        self.state.idle_timeouts.sort_unstable();
        self.state.idle_timeouts.dedup();
        self.state.refresh_idle(&self.queue.handle());
        self.state.idle_notifier.is_some()
    }

    /// What can be done to another window, by identifier.
    ///
    /// One entry point rather than five methods, because every one of them is
    /// the same lookup followed by one request, and the lookup is the part that
    /// can fail. `false` means the window is not controllable — see
    /// [`ToplevelInfo::controllable`].
    pub fn control_toplevel(&mut self, identifier: &str, action: ToplevelAction) -> bool {
        if !self.supports_toplevel_control() {
            return false;
        }
        let Some(handle) = self.toplevel_control_handle(identifier) else {
            return false;
        };
        match action {
            ToplevelAction::Activate => {
                // Activation is scoped to a seat: the protocol wants to know
                // *whose* focus is moving, and a client with no seat has no
                // business moving anybody's.
                let Some(seat) = self.state.seats.seats().next() else {
                    return false;
                };
                handle.activate(&seat);
            }
            ToplevelAction::Close => handle.close(),
            ToplevelAction::Maximized(true) => handle.set_maximized(),
            ToplevelAction::Maximized(false) => handle.unset_maximized(),
            ToplevelAction::Minimized(true) => handle.set_minimized(),
            ToplevelAction::Minimized(false) => handle.unset_minimized(),
            ToplevelAction::Fullscreen(true) => handle.set_fullscreen(None),
            ToplevelAction::Fullscreen(false) => handle.unset_fullscreen(),
            ToplevelAction::MinimizeTarget {
                x,
                y,
                width,
                height,
            } => {
                // Relative to the shell's own primary surface: that is where
                // the task bar is, and a rectangle on any other surface would
                // be a window flying towards the wrong thing.
                let Some(layer) = self.state.layers.get(&crate::PRIMARY_LAYER) else {
                    return false;
                };
                handle.set_rectangle(layer.surface.wl_surface(), x, y, width, height);
            }
        }
        true
    }

    /// Holds the compositor's shortcuts off the shell, and reports whether
    /// the compositor speaks the protocol at all -- whether it *agrees* comes
    /// later, as `LayerEvent::ShortcutsInhibited`.
    pub fn set_shortcuts_inhibited(&mut self, inhibited: bool) -> bool {
        self.state
            .set_shortcuts_inhibited(inhibited, &self.queue.handle());
        self.state.shortcuts_inhibit_manager.is_some()
    }

    /// Whether this compositor lets a client act on other windows at all.
    ///
    /// Separate from a window's own `controllable`, which additionally says
    /// whether *that* window was matched to a handle.
    pub fn supports_toplevel_control(&self) -> bool {
        self.state.toplevel_control_manager.is_some()
    }

    /// Finds the control handle for a window named by the enumeration protocol.
    ///
    /// Matched on application and title, because nothing correlates the two
    /// protocols' handles — see the module note on `toplevel_control`.
    fn toplevel_control_handle(
        &self,
        identifier: &str,
    ) -> Option<&wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1>
    {
        let listed = self
            .state
            .toplevels
            .values()
            .find(|info| info.identifier == identifier)?;
        let key = self
            .state
            .toplevel_controls
            .iter()
            .find(|(_, control)| control.app_id == listed.app_id && control.title == listed.title)
            .map(|(key, _)| key)?;
        self.state.toplevel_control_handles.get(key)
    }

    /// One surface's scale in 120ths, whatever kind of surface it is.
    ///
    /// A layer surface answers from its own record; a popup or floating window
    /// from `aux_scales`. 120 -- one to one -- when the surface is unknown or
    /// the compositor offers no fractional scale, which is what every surface
    /// but the primary layer used to get.
    pub fn surface_scale_120(&self, role: SurfaceRole) -> u32 {
        match role {
            SurfaceRole::Layer(id) => self.layer_scale_120(id).unwrap_or(120),
            other => self
                .state
                .aux_scales
                .get(&other)
                .map_or(120, |entry| entry.scale_120),
        }
    }

    /// Every workspace the compositor reports, in a stable order.
    ///
    /// Sorted by coordinates and then id, because the protocol delivers them in
    /// whatever order it happens to and a bar whose workspaces reshuffle
    /// between frames is unusable.
    pub fn workspaces(&self) -> Vec<WorkspaceInfo> {
        let mut workspaces = self.state.workspaces.values().cloned().collect::<Vec<_>>();
        workspaces.sort_by(|a, b| a.coordinates.cmp(&b.coordinates).then(a.key.cmp(&b.key)));
        workspaces
    }

    /// Whether the workspace list changed since this was last asked.
    pub fn take_workspaces_changed(&mut self) -> bool {
        std::mem::take(&mut self.state.workspaces_changed)
    }

    /// Switches to a workspace by its key, reporting whether it could.
    ///
    /// `false` covers three different disappointments a configuration would
    /// otherwise have to guess between: no such workspace, a compositor that
    /// will not switch to it, or no workspace protocol at all.
    pub fn activate_workspace(&mut self, key: &str) -> bool {
        let Some(manager) = &self.state.workspace_manager else {
            return false;
        };
        let Some(key) = self
            .state
            .workspaces
            .iter()
            .find(|(_, info)| info.key == key && info.activatable)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };
        let Some(handle) = self.state.workspace_handles.get(&key) else {
            return false;
        };
        handle.activate();
        // Nothing happens until the manager is told to apply it. The protocol
        // batches, so a configuration that activated one workspace and
        // deactivated another gets both or neither.
        manager.commit();
        true
    }

    /// Holds the session awake, and reports whether the compositor allows it.
    ///
    /// `false` means no compositor support rather than failure to apply: a
    /// configuration can tell the difference between "not inhibiting" and
    /// "cannot inhibit here", which otherwise look identical from Lua.
    pub fn set_idle_inhibited(&mut self, inhibited: bool) -> bool {
        self.state
            .set_idle_inhibited(inhibited, &self.queue.handle());
        self.state.idle_inhibit_manager.is_some()
    }

    /// Requests a power state for the configured output, or every output for a lock client.
    pub fn set_output_power(&mut self, mode: OutputPowerMode) -> bool {
        if self.state.output_power_manager.is_none() {
            return false;
        }
        self.state.output_power_mode = Some(mode);
        let available = self.state.outputs.outputs().collect::<Vec<_>>();
        let outputs = match self.state.output_power_target.clone() {
            Some(output) => available
                .into_iter()
                .filter(|item| *item == output)
                .collect(),
            None => available,
        };
        let qh = self.queue.handle();
        for output in outputs {
            self.state.apply_output_power(&output, mode, &qh);
        }
        true
    }

    /// Starts an asynchronous capture of the configured or first output.
    pub fn capture_output(&mut self, request_id: u64, include_cursor: bool) -> bool {
        let Some(manager) = &self.state.screencopy_manager else {
            return false;
        };
        if self.state.shm.is_none() || self.state.screencopies.len() >= 4 {
            return false;
        }
        let output = self
            .state
            .output_power_target
            .clone()
            .or_else(|| self.state.outputs.outputs().next());
        let Some(output) = output else {
            return false;
        };
        let frame =
            manager.capture_output(i32::from(include_cursor), &output, &self.queue.handle(), ());
        self.state.screencopies.push(PendingScreencopy {
            request_id,
            frame,
            offer: None,
            pool: None,
            buffer: None,
            format: None,
            y_invert: false,
        });
        true
    }

    /// Returns whether shared-memory output capture is available.
    /// Every window the compositor currently knows about.
    ///
    /// Sorted by identifier so the order is the same on two consecutive calls:
    /// the protocol makes no promise about it, and a list that reshuffles under
    /// a person's cursor is worse than one in an arbitrary but stable order.
    ///
    /// Windows still being described are left out. A handle arrives before its
    /// title does, and a task switcher showing a blank row for half a frame is
    /// a worse answer than showing nothing for that frame.
    pub fn toplevels(&self) -> Vec<ToplevelInfo> {
        let mut toplevels: Vec<ToplevelInfo> = self
            .state
            .toplevels
            .values()
            .filter(|toplevel| !toplevel.identifier.is_empty())
            .cloned()
            .collect();
        // The control protocol's view folded onto the enumeration's, matched on
        // application and title. A window with no match keeps its defaults and
        // stays `controllable: false`, which is the honest answer: the state is
        // not false, it is unknown.
        for toplevel in &mut toplevels {
            let Some(control) = self.state.toplevel_controls.values().find(|control| {
                control.app_id == toplevel.app_id && control.title == toplevel.title
            }) else {
                continue;
            };
            toplevel.activated = control.activated;
            toplevel.maximized = control.maximized;
            toplevel.minimized = control.minimized;
            toplevel.fullscreen = control.fullscreen;
            toplevel.controllable = true;
        }
        toplevels.sort_by(|a, b| a.identifier.cmp(&b.identifier));
        toplevels
    }

    /// Whether the window list changed since this was last called.
    ///
    /// Taking the flag rather than reading it, so a caller that acts on a
    /// change cannot act on it twice.
    pub fn take_toplevels_changed(&mut self) -> bool {
        std::mem::take(&mut self.state.toplevels_changed)
    }

    /// Whether the compositor reports its windows at all.
    pub fn supports_toplevels(&self) -> bool {
        self.state.toplevel_list.is_some()
    }

    pub fn supports_screencopy(&self) -> bool {
        self.state.screencopy_manager.is_some() && self.state.shm.is_some()
    }

    /// Publishes UTF-8 text to the clipboard after a compositor input serial is available.
    pub fn set_clipboard(&mut self, text: impl Into<String>) -> bool {
        let Some(manager) = &self.state.data_device_manager else {
            return false;
        };
        let Some(device) = self.state.data_devices.first() else {
            return false;
        };
        let Some(serial) = self.state.latest_input_serial else {
            return false;
        };
        let source = manager.create_copy_paste_source(
            &self.queue.handle(),
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"],
        );
        source.set_selection(device, serial);
        self.state.clipboard_text = text.into();
        self.state.clipboard_source = Some(source);
        true
    }

    /// Returns whether clipboard publication has a data device and a current input serial.
    pub fn can_set_clipboard(&self) -> bool {
        self.supports_clipboard() && self.state.latest_input_serial.is_some()
    }

    /// Returns whether the compositor exposes a clipboard data device.
    pub fn supports_clipboard(&self) -> bool {
        self.state.data_device_manager.is_some() && !self.state.data_devices.is_empty()
    }
}
