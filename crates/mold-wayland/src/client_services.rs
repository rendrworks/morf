use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;
use std::time::Duration;

use crate::{state_types::*, surface_types::*};

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
