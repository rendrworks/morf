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
    pub fn set_idle_timeouts(&mut self, timeouts: &[u32]) -> bool {
        self.state.idle_timeouts = timeouts.iter().copied().take(64).collect();
        self.state.idle_timeouts.sort_unstable();
        self.state.idle_timeouts.dedup();
        self.state.refresh_idle(&self.queue.handle());
        self.state.idle_notifier.is_some()
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
