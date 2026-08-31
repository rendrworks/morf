use mold_image::ImageCache;
use mold_text::TextSystem;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;

use super::{glyphs::*, targets::*, textures::*};

/// Adapter selected for the wgpu renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuInfo {
    /// Human-readable driver adapter name.
    pub name: String,
    /// Active wgpu backend.
    pub backend: wgpu::Backend,
    /// PCI vendor identifier when available.
    pub vendor: u32,
    /// PCI device identifier when available.
    pub device: u32,
}

/// wgpu initialization or submission failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuError(pub(crate) String);

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for GpuError {}

/// wgpu SDF renderer targeting a persistent texture.
pub struct WgpuBackend {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) clear_pipeline: wgpu::RenderPipeline,
    pub(crate) viewport_buffer: wgpu::Buffer,
    pub(crate) viewport_bind_group: wgpu::BindGroup,
    pub(crate) glyph_pipeline: wgpu::RenderPipeline,
    pub(crate) glyph_layout: wgpu::BindGroupLayout,
    pub(crate) glyph_sampler: wgpu::Sampler,
    pub(crate) glyph_mask_atlas: GlyphAtlas,
    pub(crate) glyph_color_atlas: GlyphAtlas,
    pub(crate) blur_pipeline: wgpu::RenderPipeline,
    pub(crate) blur_layout: wgpu::BindGroupLayout,
    pub(crate) blur_sampler: wgpu::Sampler,
    pub(crate) glyph_buffer: wgpu::Buffer,
    pub(crate) glyph_capacity: usize,
    pub(crate) texture_buffer: wgpu::Buffer,
    pub(crate) texture_capacity: usize,
    pub(crate) field_pipeline: wgpu::RenderPipeline,
    pub(crate) field_layout: wgpu::BindGroupLayout,
    pub(crate) field_buffer: wgpu::Buffer,
    pub(crate) field_capacity: usize,
    pub(crate) field_layer_buffer: wgpu::Buffer,
    pub(crate) field_material_buffer: wgpu::Buffer,
    pub(crate) field_material_capacity: usize,
    pub(crate) field_layer_capacity: usize,
    pub(crate) field_bind_group: wgpu::BindGroup,
    pub(crate) images: ImageCache,
    pub(crate) image_textures: HashMap<TextureKey, TextureImage>,
    /// Full-surface render targets, one per offscreen layer, kept between
    /// frames.
    ///
    /// Every layer — every rotation, rounded clip, blur, shadow or opacity
    /// below one — renders through one of these, and they used to be created
    /// fresh on every frame: a full-screen GPU texture per layer, sixty times a
    /// second, discarded each time. They are all the same size and every pass
    /// clears its target before drawing, so there is nothing to carry over and
    /// nothing to rebuild.
    pub(crate) layer_target_pool: Vec<(wgpu::Texture, wgpu::TextureView)>,
    pub(crate) text: TextSystem,
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) surface: Option<SurfaceState>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) info: GpuInfo,
}
