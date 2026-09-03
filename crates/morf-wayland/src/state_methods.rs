use smithay_client_toolkit::session_lock::SessionLock;
use smithay_client_toolkit::shm::slot::SlotPool;
use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_output, wl_shm};
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_v1::{self};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;

use crate::{helpers::*, state_types::*, surface_types::*, types::*};

impl LayerState {
    /// Maps a layer surface with one transparent pixel, once it is configured.
    ///
    /// Only a surface that asked for it and has not been mapped yet is touched,
    /// so this is safe to call from both the request and the configure handler.
    pub(crate) fn attach_blank_buffer(&mut self, id: u64) {
        let Some(record) = self.layers.get(&id) else {
            return;
        };
        if !record.wants_blank || record.blank.is_some() || !record.configured {
            return;
        }
        let Some(shm) = self.shm.as_ref() else {
            return;
        };
        let Ok(mut pool) = SlotPool::new(4, shm) else {
            return;
        };
        let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) else {
            return;
        };
        canvas[..4].fill(0);
        let Some(record) = self.layers.get_mut(&id) else {
            return;
        };
        let surface = record.surface.wl_surface();
        if buffer.attach_to(surface).is_err() {
            return;
        }
        surface.damage_buffer(0, 0, 1, 1);
        surface.commit();
        record.blank = Some((pool, buffer));
    }

    pub(crate) fn refresh_virtual_keyboard(&mut self, qh: &QueueHandle<Self>) {
        if self.virtual_keyboard.is_some() {
            return;
        }
        let Some(manager) = &self.virtual_keyboard_manager else {
            return;
        };
        let Some(seat) = self.seats.seats().next() else {
            return;
        };
        let Some(keymap) = &self.virtual_keyboard_keymap else {
            return;
        };
        let keyboard = manager.create_virtual_keyboard(&seat, qh, ());
        match install_virtual_keymap(&keyboard, keymap) {
            Ok(file) => {
                self.virtual_keyboard_keymap_file = Some(file);
                self.virtual_keyboard = Some(keyboard);
            }
            Err(_) => keyboard.destroy(),
        }
    }

    pub(crate) fn refresh_data_devices(&mut self, qh: &QueueHandle<Self>) {
        let Some(manager) = &self.data_device_manager else {
            return;
        };
        for seat in self.seats.seats() {
            if self
                .data_devices
                .iter()
                .all(|device| device.data().seat() != &seat)
            {
                self.data_devices.push(manager.get_data_device(qh, &seat));
            }
        }
    }

    pub(crate) fn apply_output_power(
        &mut self,
        output: &wl_output::WlOutput,
        mode: OutputPowerMode,
        qh: &QueueHandle<Self>,
    ) {
        let Some(manager) = self.output_power_manager.clone() else {
            return;
        };
        let control = self
            .output_power
            .iter()
            .find(|control| control.output == *output)
            .map(|control| control.control.clone())
            .unwrap_or_else(|| {
                let control = manager.get_output_power(output, qh, output.clone());
                self.output_power.push(OutputPowerControl {
                    output: output.clone(),
                    control: control.clone(),
                });
                control
            });
        control.set_mode(match mode {
            OutputPowerMode::Off => zwlr_output_power_v1::Mode::Off,
            OutputPowerMode::On => zwlr_output_power_v1::Mode::On,
        });
    }

    pub(crate) fn start_screencopy(&mut self, frame: &ZwlrScreencopyFrameV1) -> Result<(), String> {
        let pending = self
            .screencopies
            .iter_mut()
            .find(|pending| pending.frame == *frame)
            .ok_or_else(|| "unknown screencopy frame".to_owned())?;
        if pending.buffer.is_some() {
            return Ok(());
        }
        let (format, width, height, stride) = pending
            .offer
            .ok_or_else(|| "compositor supplied no shared-memory format".to_owned())?;
        let public_format = match format {
            wl_shm::Format::Argb8888 => ScreencopyFormat::Argb8888,
            wl_shm::Format::Xrgb8888 => ScreencopyFormat::Xrgb8888,
            _ => return Err(format!("unsupported screencopy format {format:?}")),
        };
        if width == 0
            || height == 0
            || stride
                < width
                    .checked_mul(4)
                    .ok_or_else(|| "screencopy width overflow".to_owned())?
        {
            return Err("invalid screencopy dimensions".to_owned());
        }
        let byte_len = (height as usize)
            .checked_mul(stride as usize)
            .filter(|size| *size <= 64 * 1024 * 1024)
            .ok_or_else(|| "screencopy buffer exceeds 64 MiB".to_owned())?;
        let width = i32::try_from(width).map_err(|_| "screencopy width is too large".to_owned())?;
        let height =
            i32::try_from(height).map_err(|_| "screencopy height is too large".to_owned())?;
        let stride =
            i32::try_from(stride).map_err(|_| "screencopy stride is too large".to_owned())?;
        let shm = self
            .shm
            .as_ref()
            .ok_or_else(|| "wl_shm is unavailable".to_owned())?;
        let mut pool = SlotPool::new(byte_len.max(1), shm).map_err(|error| error.to_string())?;
        let (buffer, _) = pool
            .create_buffer(width, height, stride, format)
            .map_err(|error| error.to_string())?;
        frame.copy(buffer.wl_buffer());
        pending.pool = Some(pool);
        pending.buffer = Some(buffer);
        pending.format = Some(public_format);
        Ok(())
    }

    pub(crate) fn finish_screencopy(
        &mut self,
        frame: &ZwlrScreencopyFrameV1,
    ) -> Result<ScreencopyFrame, String> {
        let index = self
            .screencopies
            .iter()
            .position(|pending| pending.frame == *frame)
            .ok_or_else(|| "unknown screencopy frame".to_owned())?;
        let mut pending = self.screencopies.remove(index);
        let mut pool = pending
            .pool
            .take()
            .ok_or_else(|| "screencopy has no shared-memory pool".to_owned())?;
        let buffer = pending
            .buffer
            .take()
            .ok_or_else(|| "screencopy has no shared-memory buffer".to_owned())?;
        let pixels = buffer
            .canvas(&mut pool)
            .ok_or_else(|| "screencopy buffer is still active".to_owned())?
            .to_vec();
        let (_, width, height, stride) = pending
            .offer
            .ok_or_else(|| "screencopy metadata is missing".to_owned())?;
        Ok(ScreencopyFrame {
            width,
            height,
            stride,
            format: pending
                .format
                .ok_or_else(|| "screencopy format is missing".to_owned())?,
            y_invert: pending.y_invert,
            pixels,
        })
    }

    pub(crate) fn fail_screencopy(&mut self, frame: &ZwlrScreencopyFrameV1, error: String) {
        let request_id = self
            .screencopies
            .iter()
            .position(|pending| pending.frame == *frame)
            .map(|index| self.screencopies.remove(index).request_id);
        frame.destroy();
        if let Some(request_id) = request_id {
            self.events.push_back(LayerEvent::Screencopy {
                request_id,
                result: Err(error),
            });
        }
    }

    pub(crate) fn refresh_idle(&mut self, qh: &QueueHandle<Self>) {
        for notification in self.idle_notifications.drain(..) {
            notification.destroy();
        }
        let Some(notifier) = &self.idle_notifier else {
            return;
        };
        let Some(seat) = self.seats.seats().next() else {
            return;
        };
        self.idle_notifications = self
            .idle_timeouts
            .iter()
            .map(|timeout| notifier.get_idle_notification(*timeout, &seat, qh, *timeout))
            .collect();
    }

    /// Starts tracking fractional scale for a popup or floating window.
    ///
    /// The same two objects a layer surface gets, kept in `aux_scales` because
    /// those surfaces have no record of their own. A compositor offering
    /// neither protocol leaves both `None`, and the surface stays at 1x --
    /// which is what it did before this existed.
    pub(crate) fn track_aux_scale(
        &mut self,
        role: SurfaceRole,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        qh: &QueueHandle<Self>,
    ) {
        let fractional = self
            .fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(surface, qh, role));
        let viewport = self
            .viewporter
            .as_ref()
            .map(|manager| manager.get_viewport(surface, qh, ()));
        self.aux_scales.insert(
            role,
            AuxSurfaceScale {
                fractional,
                viewport,
                scale_120: 120,
            },
        );
    }

    /// Holds the session awake, or stops holding it.
    ///
    /// The protocol has no "off": an inhibitor exists or it does not, and
    /// destroying it is how the session is released. So this is idempotent by
    /// construction — asking twice for the same state does nothing the second
    /// time, which matters because a configuration is likely to assign this
    /// from a binding that re-runs on every frame.
    pub(crate) fn set_idle_inhibited(&mut self, inhibited: bool, qh: &QueueHandle<Self>) {
        if inhibited == self.idle_inhibitor.is_some() {
            return;
        }
        match self.idle_inhibitor.take() {
            Some(inhibitor) => inhibitor.destroy(),
            None => {
                let Some(manager) = &self.idle_inhibit_manager else {
                    return;
                };
                // Against the shell's own surface, because the protocol scopes
                // inhibition to a surface. Looked up rather than taken through
                // `layer()`, which panics when there is none: a configuration
                // may ask for this before its surface exists, and refusing to
                // inhibit is the right answer there rather than dying.
                let Some(layer) = self.layers.get(&crate::PRIMARY_LAYER) else {
                    return;
                };
                self.idle_inhibitor =
                    Some(manager.create_inhibitor(layer.surface.wl_surface(), qh, ()));
            }
        }
    }

    pub(crate) fn create_lock_surface(
        &mut self,
        output: wl_output::WlOutput,
        qh: &QueueHandle<Self>,
    ) {
        let Some(lock) = self.session_lock.clone().filter(SessionLock::is_locked) else {
            return;
        };
        if self
            .lock_surfaces
            .iter()
            .any(|surface| surface.output == output)
        {
            return;
        }
        let scale = self
            .outputs
            .info(&output)
            .map(|info| info.scale_factor.max(1) as u32)
            .unwrap_or(1);
        let surface = self.compositor.create_surface(qh);
        surface.set_buffer_scale(scale as i32);
        let surface = lock.create_lock_surface(surface, &output, qh);
        surface.wl_surface().commit();
        self.lock_surfaces.push(LockSurface {
            surface,
            output,
            size: (1, 1),
            scale,
        });
    }
}
