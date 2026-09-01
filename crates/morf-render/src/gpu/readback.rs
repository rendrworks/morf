//! Reading the offscreen target back to ordinary memory.
//!
//! Only useful when there is no compositor to hand the frame to: tests
//! asserting on pixels, and tools that want to look at what a configuration
//! actually draws. A shader can pass every gate there is and still be wrong in
//! a way only looking at it will reveal, and until this existed the only way to
//! look was to run the whole shell.

use super::backend_types::WgpuBackend;

impl WgpuBackend {
    /// The offscreen target's pixels, tightly packed as RGBA8.
    ///
    /// Rows come back with no padding, so a caller can index by `(y * width +
    /// x) * 4`. The target is sRGB, which is what an image file wants anyway.
    pub fn read_pixels(&mut self) -> Vec<u8> {
        let width = self.width;
        let height = self.height;
        // Copies out of a texture want rows aligned to 256 bytes, which is 64
        // pixels — so the buffer is wider than the image and the rows are
        // repacked afterwards.
        let bytes_per_row = width.next_multiple_of(64) * 4;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("morf readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("morf readback copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
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
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (send, receive) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the queue drains");
        receive.recv().expect("the map completes").expect("mapped");
        let mapped = slice.get_mapped_range().expect("the range is readable");
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            out.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        out
    }
}
