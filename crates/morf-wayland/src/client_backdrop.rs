//! Asking the compositor to blur what is behind a surface.
//!
//! `ext-background-effect-v1`: a cross-desktop staging protocol, in the `ext`
//! namespace rather than a vendor one, which is the whole reason it is worth
//! having — it replaced a decade of per-compositor blur extensions with one
//! request every compositor can answer.
//!
//! A client never receives the pixels behind it — Wayland
//! does not offer them, and this protocol does not either. What it offers is a
//! region: the compositor blurs its own already-composited result inside that
//! region, and only then blends this surface over the top.
//!
//! Which means the alpha is what reveals it. A panel painted opaque sits on a
//! blurred backdrop nobody can see; a panel painted at a fifth of full alpha is
//! frosted glass. Everything that makes it look like glass rather than like a
//! blur filter — the tint, the grain, the lit edge — is painted here, over a
//! backdrop this process never touches.

use morf_region::Region;

use crate::WaylandError;
use crate::state_types::PendingCapture;
use crate::surface_types::LayerClient;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    self, ExtImageCopyCaptureManagerV1,
};

impl LayerClient {
    /// Whether the compositor will blur behind a surface.
    ///
    /// False both when the protocol is absent and when it is present but has
    /// withdrawn the capability, because a configuration cannot act on the
    /// difference: either way it should paint something that stands on its own.
    pub fn supports_backdrop_blur(&self) -> bool {
        self.state.background_effect.is_some() && self.state.blur_capable
    }

    /// Blurs the backdrop inside `rectangles`, in surface-local coordinates.
    ///
    /// An empty slice blurs nothing, which is different from `None`: `None`
    /// clears the effect entirely and lets the surface go back to being an
    /// ordinary one.
    pub fn set_layer_backdrop_region(
        &self,
        id: u64,
        rectangles: Option<&[morf_region::Rect]>,
    ) -> Result<(), WaylandError> {
        if !self.supports_backdrop_blur() {
            return Ok(());
        }
        let Some(backdrop) = self
            .state
            .layers
            .get(&id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?
            .backdrop
            .as_ref()
        else {
            return Ok(());
        };
        let qh = self.queue.handle();

        let Some(rectangles) = rectangles else {
            // A null region is the protocol's own way of saying "no effect",
            // and it leaves the object in place to be used again.
            backdrop.set_blur_region(None);
            return Ok(());
        };

        let region = self.state.compositor.wl_compositor().create_region(&qh, ());
        for rectangle in rectangles {
            if rectangle.width > 0 && rectangle.height > 0 {
                region.add(rectangle.x, rectangle.y, rectangle.width, rectangle.height);
            }
        }
        backdrop.set_blur_region(Some(&region));
        region.destroy();
        Ok(())
    }

    /// Builds a blur region from composable shapes and applies it.
    ///
    /// A region is a set of rectangles, but at pixel granularity — so a circle
    /// is not approximated as a shape, it is one span per scanline, and the
    /// merged silhouette of a distance field comes out exactly. What a region
    /// cannot carry is a soft edge: membership is one bit per pixel, so the
    /// boundary of the blur is hard, and it is the surface's own antialiased
    /// painting on top that hides the step.
    pub fn set_layer_composed_backdrop_region(
        &self,
        id: u64,
        regions: &[Region],
    ) -> Result<(), WaylandError> {
        if !self.supports_backdrop_blur() {
            return Ok(());
        }
        let (width, height) = self
            .layer_logical_size(id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?;
        let rectangles = morf_region::build(width, height, regions)
            .map_err(|error| WaylandError(error.to_string()))?;
        self.set_layer_backdrop_region(id, Some(&rectangles))
    }
}

impl LayerClient {
    /// Whether the newer capture protocol is available, with an output source.
    pub fn supports_image_capture(&self) -> bool {
        self.state.capture_manager.is_some() && self.state.output_source_manager.is_some()
    }

    /// Whether a single window can be captured on its own.
    ///
    /// Separate from the above because a compositor may implement the copy
    /// machinery and only the output source — and the difference is the whole
    /// difference between a screenshot and an overview.
    pub fn supports_window_capture(&self) -> bool {
        self.state.capture_manager.is_some() && self.state.toplevel_source_manager.is_some()
    }

    /// Captures one window, named by the identifier `morf.windows` reported.
    ///
    /// By identifier rather than by index or title: an index means something
    /// different the moment a window opens, and two windows of one application
    /// share a title as readily as an app id.
    ///
    /// With `gpu`, the session's dmabuf offer is reported as a `CaptureOffer`
    /// instead of being answered with shared memory, so the renderer can hand
    /// over a buffer the compositor draws into directly.
    pub fn capture_window(&mut self, request_id: u64, identifier: &str, gpu: bool) -> bool {
        let (Some(manager), Some(sources)) = (
            self.state.capture_manager.clone(),
            self.state.toplevel_source_manager.clone(),
        ) else {
            return false;
        };
        let Some(handle) = self.state.toplevel_handles.get(identifier).cloned() else {
            return false;
        };
        if self.state.shm.is_none() || self.state.captures.len() >= 8 {
            return false;
        }
        let qh = self.queue.handle();
        let source = sources.create_source(&handle, &qh, ());
        self.start_capture(request_id, gpu, &manager, &source, &qh);
        true
    }

    /// Captures an output through the newer protocol.
    pub fn capture_output_image(&mut self, request_id: u64, gpu: bool) -> bool {
        let (Some(manager), Some(sources)) = (
            self.state.capture_manager.clone(),
            self.state.output_source_manager.clone(),
        ) else {
            return false;
        };
        let output = self
            .state
            .output_power_target
            .clone()
            .or_else(|| self.state.outputs.outputs().next());
        let Some(output) = output else {
            return false;
        };
        if self.state.shm.is_none() || self.state.captures.len() >= 8 {
            return false;
        }
        let qh = self.queue.handle();
        let source = sources.create_source(&output, &qh, ());
        self.start_capture(request_id, gpu, &manager, &source, &qh);
        true
    }

    /// Opens a session against a source and waits for it to describe itself.
    ///
    /// Nothing is allocated here. The session has not yet said what size or
    /// format it can produce, and guessing would mean allocating a buffer the
    /// compositor is about to refuse.
    fn start_capture(
        &mut self,
        request_id: u64,
        gpu: bool,
        manager: &ExtImageCopyCaptureManagerV1,
        source: &ExtImageCaptureSourceV1,
        qh: &wayland_client::QueueHandle<crate::state_types::LayerState>,
    ) {
        let session = manager.create_session(
            source,
            // Cursors are a separate session in this protocol, and a thumbnail
            // with somebody's pointer baked into it is not a thumbnail of the
            // window.
            ext_image_copy_capture_manager_v1::Options::empty(),
            qh,
            (),
        );
        self.state.captures.push(PendingCapture {
            request_id,
            session,
            frame: None,
            size: None,
            format: None,
            pool: None,
            buffer: None,
            started: false,
            // Only an offer the compositor can honour is worth making: with
            // no dmabuf global there is no way to hand it a buffer.
            gpu: gpu && self.state.linux_dmabuf.is_some(),
            offered: false,
            dmabuf_device: None,
            dmabuf_formats: Vec::new(),
            dmabuf_buffer: None,
        });
    }
}
