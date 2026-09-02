//! `ext-image-copy-capture-v1`: the protocol that can capture a window.
//!
//! The reason to want it over `wlr-screencopy` is not that the older one is
//! deprecated. It is that the older one captures *outputs*, and there is no way
//! to get a window out of an output capture — cropping to a window's rectangle
//! gives whatever is on top there, which is frequently not the window.
//!
//! It negotiates before it copies, which the older one did not: a session says
//! what size and formats it can produce, and only once that is `done` is there
//! anything to allocate against. Then a frame is created, given a buffer, and
//! told to capture. That is more states to carry, and it is the price of being
//! able to ask for a window by name.

use smithay_client_toolkit::shm::slot::SlotPool;
use wayland_client::protocol::wl_shm;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

use crate::state_types::LayerState;
use crate::surface_types::LayerEvent;
use crate::types::{ScreencopyFormat, ScreencopyFrame};

// The factories and the source handle say nothing back.
wayland_client::delegate_noop!(LayerState: ignore ExtImageCopyCaptureManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ExtImageCaptureSourceV1);

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for LayerState {
    /// The session describing what it can produce.
    ///
    /// `buffer_size` and `shm_format` may each arrive more than once and mean
    /// nothing until `done`. The first format this engine can carry is kept:
    /// the compositor offers them in its own order of preference, and there is
    /// no reason to second-guess that between two it treats as equivalent.
    fn event(
        state: &mut Self,
        session: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _data: &(),
        _connection: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        let id = session.id();
        let Some(index) = state
            .captures
            .iter()
            .position(|capture| capture.session.id() == id)
        else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.captures[index].size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                if state.captures[index].format.is_none()
                    && let Ok(format) = format.into_result()
                    && matches!(format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)
                {
                    state.captures[index].format = Some(format);
                }
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                state.begin_capture_frame(index, queue);
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                state.fail_capture(index, "capture session stopped".to_owned());
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for LayerState {
    /// The frame either arrives or does not.
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        let id = frame.id();
        let Some(index) = state
            .captures
            .iter()
            .position(|capture| capture.frame.as_ref().is_some_and(|held| held.id() == id))
        else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.finish_capture(index),
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.fail_capture(index, format!("capture failed: {reason:?}"));
            }
            _ => {}
        }
    }
}

impl LayerState {
    /// Allocates against what the session offered and asks for the picture.
    pub(crate) fn begin_capture_frame(&mut self, index: usize, queue: &QueueHandle<Self>) {
        if self.captures[index].started {
            return;
        }
        let Some((width, height)) = self.captures[index].size else {
            self.fail_capture(index, "capture session offered no size".to_owned());
            return;
        };
        let Some(format) = self.captures[index].format else {
            self.fail_capture(index, "capture session offered no usable format".to_owned());
            return;
        };
        let Some(shm) = self.shm.as_ref() else {
            self.fail_capture(index, "no shared memory".to_owned());
            return;
        };
        let stride = width.saturating_mul(4);
        let Ok(mut pool) = SlotPool::new((stride as usize).saturating_mul(height as usize), shm)
        else {
            self.fail_capture(index, "could not allocate a capture pool".to_owned());
            return;
        };
        let Ok((buffer, _)) =
            pool.create_buffer(width as i32, height as i32, stride as i32, format)
        else {
            self.fail_capture(index, "could not allocate a capture buffer".to_owned());
            return;
        };
        let frame = self.captures[index].session.create_frame(queue, ());
        frame.attach_buffer(buffer.wl_buffer());
        frame.capture();
        let capture = &mut self.captures[index];
        capture.pool = Some(pool);
        capture.buffer = Some(buffer);
        capture.frame = Some(frame);
        capture.started = true;
    }

    /// Reads the captured pixels out and hands them to the event queue.
    pub(crate) fn finish_capture(&mut self, index: usize) {
        let mut capture = self.captures.remove(index);
        let request_id = capture.request_id;
        let result = (|| {
            let (width, height) = capture.size.ok_or("capture has no size")?;
            let format = capture.format.ok_or("capture has no format")?;
            let mut pool = capture.pool.take().ok_or("capture has no pool")?;
            let buffer = capture.buffer.take().ok_or("capture has no buffer")?;
            let pixels = buffer
                .canvas(&mut pool)
                .ok_or("capture buffer is still in use")?
                .to_vec();
            Ok::<_, &str>(ScreencopyFrame {
                width,
                height,
                stride: width.saturating_mul(4),
                format: match format {
                    wl_shm::Format::Xrgb8888 => ScreencopyFormat::Xrgb8888,
                    _ => ScreencopyFormat::Argb8888,
                },
                // This protocol reports orientation through `transform` rather
                // than a flip flag, and every compositor tested reports normal
                // for an output or a toplevel. Anything else would be a rotated
                // capture, which is a thing to handle when one turns up rather
                // than to guess at now.
                y_invert: false,
                pixels,
            })
        })();
        capture.session.destroy();
        self.events.push_back(LayerEvent::Screencopy {
            request_id,
            result: result.map_err(str::to_owned),
        });
    }

    /// Abandons a capture and tells the configuration why.
    pub(crate) fn fail_capture(&mut self, index: usize, error: String) {
        if index >= self.captures.len() {
            return;
        }
        let capture = self.captures.remove(index);
        capture.session.destroy();
        self.events.push_back(LayerEvent::Screencopy {
            request_id: capture.request_id,
            result: Err(error),
        });
    }
}
