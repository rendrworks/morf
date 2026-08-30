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
pub struct GpuError(String);

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for GpuError {}

/// wgpu SDF renderer targeting a persistent texture.
pub struct WgpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::RenderPipeline,
    viewport_buffer: wgpu::Buffer,
    viewport_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    glyph_pipeline: wgpu::RenderPipeline,
    glyph_layout: wgpu::BindGroupLayout,
    glyph_sampler: wgpu::Sampler,
    glyph_mask_atlas: GlyphAtlas,
    glyph_color_atlas: GlyphAtlas,
    blur_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_sampler: wgpu::Sampler,
    glyph_buffer: wgpu::Buffer,
    glyph_capacity: usize,
    texture_buffer: wgpu::Buffer,
    texture_capacity: usize,
    path_pipeline: wgpu::RenderPipeline,
    path_vertex_buffer: wgpu::Buffer,
    path_vertex_capacity: usize,
    path_index_buffer: wgpu::Buffer,
    path_index_capacity: usize,
    paths: PathCache,
    images: ImageCache,
    image_textures: HashMap<TextureKey, TextureImage>,
    text: TextSystem,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    surface: Option<SurfaceState>,
    width: u32,
    height: u32,
    info: GpuInfo,
}

