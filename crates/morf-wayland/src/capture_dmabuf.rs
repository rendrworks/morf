//! Captures that go straight into GPU memory.
//!
//! `ext-image-copy-capture-v1` can draw into a dmabuf as readily as into
//! shared memory, and a session says so: it names the device the buffer must
//! live on and the formats and modifiers it will draw with. What it cannot do
//! is allocate that buffer -- only the renderer can, since the memory has to
//! be something its GPU will read afterwards. So the session's description is
//! handed out as a `CaptureOffer`, and the answer comes back here as a file
//! descriptor with a layout, which `zwp_linux_dmabuf_v1` turns into the
//! `wl_buffer` the frame is given.
//!
//! The shared-memory path is not replaced by this, it is the fallback for it:
//! a renderer that cannot export, a device that is not the compositor's, a
//! format nobody has in common, and the capture continues as it always did.

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};

use crate::WaylandError;
use crate::state_types::LayerState;
use crate::surface_types::{LayerClient, LayerEvent};
use crate::types::CaptureBuffer;

/// `DRM_FORMAT_XRGB8888`: the one capture format with nothing in the top byte.
pub(crate) const FOURCC_XRGB8888: u32 = 0x3432_5258;

// The dmabuf global's own format events describe what *any* buffer may be,
// which is broader than what a capture may be; the params object only speaks
// for the deferred `create`, and this engine uses the immediate one; a buffer
// says `release`, which matters to a surface and not to a capture.
wayland_client::delegate_noop!(LayerState: ignore ZwpLinuxDmabufV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpLinuxBufferParamsV1);
wayland_client::delegate_noop!(LayerState: ignore WlBuffer);

impl LayerState {
    /// Reports a GPU-wanted session's description, if there is one to report.
    ///
    /// Returns whether an offer went out -- in which case the capture waits
    /// for a buffer rather than allocating one. A session may say `done`
    /// again later, when its source changes shape; an offer already out is
    /// not repeated for that, since the buffer that answers it is checked
    /// against the size the compositor then wants anyway.
    pub(crate) fn offer_capture(&mut self, index: usize) -> bool {
        let capture = &mut self.captures[index];
        if !capture.gpu || capture.started || capture.dmabuf_formats.is_empty() {
            return false;
        }
        if capture.offered {
            return true;
        }
        let Some((width, height)) = capture.size else {
            return false;
        };
        capture.offered = true;
        self.events.push_back(LayerEvent::CaptureOffer {
            request_id: capture.request_id,
            width,
            height,
            device: capture.dmabuf_device,
            formats: capture.dmabuf_formats.clone(),
        });
        true
    }

    fn capture_index(&self, request_id: u64) -> Option<usize> {
        self.captures
            .iter()
            .position(|capture| capture.request_id == request_id && capture.offered)
    }
}

impl LayerClient {
    /// Whether a capture can be drawn straight into a dmabuf.
    ///
    /// The protocol side only: whether the renderer can export one is its
    /// own question, and the capability a configuration sees is both.
    pub fn supports_dmabuf_capture(&self) -> bool {
        self.state.linux_dmabuf.is_some() && self.state.capture_manager.is_some()
    }

    /// Answers a `CaptureOffer` with a dmabuf for the compositor to draw into.
    ///
    /// The buffer is created immediately rather than asynchronously, so a
    /// layout the compositor rejects is a protocol error rather than a late
    /// `failed` event -- and a renderer that only offers what the session
    /// named never triggers one.
    pub fn attach_capture_dmabuf(
        &mut self,
        request_id: u64,
        buffer: &CaptureBuffer<'_>,
    ) -> Result<(), WaylandError> {
        let index = self
            .state
            .capture_index(request_id)
            .ok_or_else(|| WaylandError("no capture is waiting for a buffer".into()))?;
        let dmabuf = self
            .state
            .linux_dmabuf
            .clone()
            .ok_or_else(|| WaylandError("the compositor has no dmabuf support".into()))?;
        let qh: QueueHandle<LayerState> = self.queue.handle();
        let params = dmabuf.create_params(&qh, ());
        params.add(
            buffer.fd,
            0,
            buffer.offset,
            buffer.stride,
            (buffer.modifier >> 32) as u32,
            (buffer.modifier & 0xffff_ffff) as u32,
        );
        let wl_buffer = params.create_immed(
            buffer.width as i32,
            buffer.height as i32,
            buffer.fourcc,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &qh,
            (),
        );
        params.destroy();
        let frame = self.state.captures[index].session.create_frame(&qh, ());
        frame.attach_buffer(&wl_buffer);
        frame.capture();
        let capture = &mut self.state.captures[index];
        capture.dmabuf_buffer = Some((wl_buffer, buffer.fourcc));
        capture.frame = Some(frame);
        capture.started = true;
        Ok(())
    }

    /// Answers a `CaptureOffer` with shared memory after all.
    ///
    /// For a renderer that cannot export, or a device that is not the one the
    /// compositor draws on. Returns whether the capture was still waiting.
    pub fn attach_capture_shm(&mut self, request_id: u64) -> bool {
        let Some(index) = self.state.capture_index(request_id) else {
            return false;
        };
        let qh = self.queue.handle();
        self.state.captures[index].gpu = false;
        self.state.begin_capture_frame(index, &qh);
        true
    }
}
