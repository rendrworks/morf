//! Captures that never leave the GPU.
//!
//! The shared-memory capture path publishes pixels under a `memory:` name;
//! this is its twin for images the compositor drew straight into GPU memory,
//! published under `gpu:<name>`. A configuration tells the two apart only by
//! the prefix, and `ui.Image` draws either.

use morf_image::ImageData;

use super::backend_types::{ExternalTexture, WgpuBackend};
use super::dmabuf::{self, DmabufImage, DmabufSupport};

impl WgpuBackend {
    /// What this device can do about dmabufs, when anything.
    pub fn dmabuf_support(&self) -> Option<&DmabufSupport> {
        self.dmabuf.as_ref()
    }

    /// The modifiers this device can export `fourcc` with, single-plane.
    ///
    /// What to intersect a compositor's offer with -- and, in a test with no
    /// compositor, what to offer.
    pub fn capture_modifiers(&self, fourcc: u32) -> Vec<u64> {
        if self.dmabuf.is_none() {
            return Vec::new();
        }
        dmabuf::modifiers_for(&self.device, fourcc)
    }

    /// Creates an image for the compositor to capture into, exported as a
    /// dmabuf, choosing a modifier from those it offered for `fourcc`.
    pub fn export_capture(
        &self,
        width: u32,
        height: u32,
        fourcc: u32,
        modifiers: &[u64],
    ) -> Result<DmabufImage, String> {
        if self.dmabuf.is_none() {
            return Err("this device cannot export dmabufs".to_owned());
        }
        dmabuf::export(&self.device, width, height, fourcc, modifiers)
    }

    /// Takes a filled capture back from the compositor and publishes it.
    ///
    /// The acquire is what orders the compositor's last write before this
    /// engine's first read; the bind group is made once here because the
    /// texture never changes size or format. Replaces whatever the name held,
    /// so a thumbnail that refreshes does not leak a texture per refresh --
    /// and the old one's memory is freed by wgpu after its last use.
    pub fn publish_texture(
        &mut self,
        name: impl Into<String>,
        image: DmabufImage,
    ) -> Result<(), String> {
        super::dmabuf_acquire::acquire(&self.device, &self.queue, &image)?;
        // Read through an sRGB view, so the capture's encoded pixels come out
        // of the sampler linear like every decoded image does.
        let view = image.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Bgra8UnormSrgb),
            ..Default::default()
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("morf capture bind group"),
            layout: &self.glyph_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.glyph_sampler),
                },
            ],
        });
        self.external_textures
            .insert(name.into(), ExternalTexture { image, bind_group });
        Ok(())
    }

    /// Keeps an exported image while the compositor draws into it.
    ///
    /// Its file descriptor has been handed over by then; what is kept is the
    /// image itself, so that when the compositor says the picture is there,
    /// there is a texture to publish. Replacing an export under the same id
    /// frees the old one, which is right: it was never going to be filled.
    pub fn stash_export(&mut self, request_id: u64, image: DmabufImage) {
        self.pending_exports.insert(request_id, image);
    }

    /// Takes back an image stashed by `stash_export`.
    pub fn take_export(&mut self, request_id: u64) -> Option<DmabufImage> {
        self.pending_exports.remove(&request_id)
    }

    /// Drops a published texture, and says whether there was one.
    pub fn forget_texture(&mut self, name: &str) -> bool {
        self.external_textures.remove(name).is_some()
    }

    /// The size of a published texture, when the name is one.
    pub fn published_texture_size(&self, name: &str) -> Option<(u32, u32)> {
        self.external_textures
            .get(name)
            .map(|external| (external.image.width, external.image.height))
    }

    /// Reads a published texture back as RGBA pixels.
    ///
    /// The one copy this path otherwise avoids, so not for drawing -- for
    /// proving. A test that captures the screen into a dmabuf and compares a
    /// few pixels against the shared-memory capture of the same screen is the
    /// only way to know the zero-copy path shows the same picture.
    pub fn texture_pixels(&self, name: &str) -> Option<ImageData> {
        let external = self.external_textures.get(name)?;
        let (width, height) = (external.image.width, external.image.height);
        let bytes_per_row = (width * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("morf capture readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("morf capture readback copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &external.image.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let mapped = slice.get_mapped_range().ok()?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let start = row * bytes_per_row as usize;
            let bytes = &mapped[start..start + (width * 4) as usize];
            // The texture is BGRA; the picture is handed back as RGBA, which is
            // what every other image in the engine is.
            for pixel in bytes.chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        drop(mapped);
        buffer.unmap();
        Some(ImageData {
            width,
            height,
            rgba,
        })
    }
}
