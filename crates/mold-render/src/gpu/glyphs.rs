fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mold SDF instances"),
        size: (capacity * mem::size_of::<SdfQuadInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy)]
struct GlyphInstance {
    origin: [f32; 2],
    axes: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    color_overlay: [f32; 4],
    mode: [f32; 4],
    surface: [f32; 4],
    mask_bounds: [f32; 4],
    mask_inverse_0: [f32; 4],
    mask_inverse_1: [f32; 4],
    mask_radii: [f32; 4],
}

fn layer_mask_data(mask: Option<LayerMask>) -> (f32, [f32; 4], [f32; 4], [f32; 4], [f32; 4]) {
    let Some(mask) = mask else {
        return (0.0, [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
    };
    let [a, b, c, d, tx, ty] = mask.transform.matrix;
    let determinant = a * d - b * c;
    if determinant.abs() <= f64::EPSILON {
        return (0.0, [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
    }
    let inverse_a = d / determinant;
    let inverse_b = -b / determinant;
    let inverse_c = -c / determinant;
    let inverse_d = a / determinant;
    let inverse_tx = -(inverse_a * tx + inverse_c * ty);
    let inverse_ty = -(inverse_b * tx + inverse_d * ty);
    (
        1.0,
        [
            mask.bounds.x as f32,
            mask.bounds.y as f32,
            mask.bounds.width as f32,
            mask.bounds.height as f32,
        ],
        [inverse_a as f32, inverse_c as f32, inverse_tx as f32, 0.0],
        [inverse_b as f32, inverse_d as f32, inverse_ty as f32, 0.0],
        mask.radii.map(|radius| radius as f32),
    )
}

fn transformed_quad(
    transform: Transform2D,
    bounds: Geometry,
    scale: f64,
    target_size: (u32, u32),
) -> ([f32; 2], [f32; 4]) {
    let origin = transform.point(bounds.x, bounds.y);
    let horizontal = transform.point(bounds.x + bounds.width, bounds.y);
    let vertical = transform.point(bounds.x, bounds.y + bounds.height);
    let (target_width, target_height) = target_size;
    let clip = |point: (f64, f64)| {
        [
            (point.0 * scale) as f32 / target_width as f32 * 2.0 - 1.0,
            1.0 - (point.1 * scale) as f32 / target_height as f32 * 2.0,
        ]
    };
    let origin_clip = clip(origin);
    let horizontal_clip = clip(horizontal);
    let vertical_clip = clip(vertical);
    (
        origin_clip,
        [
            horizontal_clip[0] - origin_clip[0],
            horizontal_clip[1] - origin_clip[1],
            vertical_clip[0] - origin_clip[0],
            vertical_clip[1] - origin_clip[1],
        ],
    )
}

struct GlyphBatch {
    instances: Vec<GlyphInstance>,
    command_spans: Vec<Vec<GlyphSpan>>,
}

struct GlyphSpan {
    range: Range<u32>,
    color: bool,
}

const GLYPH_ATLAS_SIZE: u32 = 2048;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GlyphKey {
    id: u64,
    width: u32,
    height: u32,
}

impl GlyphKey {
    fn from_glyph(glyph: &RasterGlyph) -> Self {
        Self {
            id: glyph.cache_key,
            width: glyph.width,
            height: glyph.height,
        }
    }
}

struct GlyphAtlasEntry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    last_used: u64,
    pixels: Vec<u8>,
}

struct PreparedGlyph {
    glyph: RasterGlyph,
    color: Color,
    color_overlay: Color,
    transform: Transform2D,
    command_index: usize,
}

#[derive(Clone, Copy, Default)]
struct ShelfAllocator {
    x: u32,
    y: u32,
    row_height: u32,
}

impl ShelfAllocator {
    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let width = width.checked_add(2)?;
        let height = height.checked_add(2)?;
        if width > GLYPH_ATLAS_SIZE || height > GLYPH_ATLAS_SIZE {
            return None;
        }
        if self.x + width > GLYPH_ATLAS_SIZE {
            self.x = 0;
            self.y = self.y.checked_add(self.row_height)?;
            self.row_height = 0;
        }
        if self.y + height > GLYPH_ATLAS_SIZE {
            return None;
        }
        let placement = (self.x + 1, self.y + 1);
        self.x += width;
        self.row_height = self.row_height.max(height);
        Some(placement)
    }
}

struct GlyphAtlas {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    content: RasterContent,
    bytes_per_pixel: u32,
    entries: HashMap<GlyphKey, GlyphAtlasEntry>,
    allocator: ShelfAllocator,
    clock: u64,
}

impl GlyphAtlas {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        content: RasterContent,
    ) -> Self {
        let (format, bytes_per_pixel, label) = match content {
            RasterContent::Mask => (wgpu::TextureFormat::R8Unorm, 1, "mold glyph mask atlas"),
            RasterContent::Color => (
                wgpu::TextureFormat::Rgba8UnormSrgb,
                4,
                "mold color glyph atlas",
            ),
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: GLYPH_ATLAS_SIZE,
                height: GLYPH_ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
            content,
            bytes_per_pixel,
            entries: HashMap::new(),
            allocator: ShelfAllocator::default(),
            clock: 0,
        }
    }

    fn prepare(&mut self, queue: &wgpu::Queue, glyphs: &[PreparedGlyph]) -> Result<(), GpuError> {
        self.clock = self.clock.wrapping_add(1);
        let mut requested = HashSet::new();
        let mut missing = Vec::new();
        for prepared in glyphs {
            let glyph = &prepared.glyph;
            if glyph.content != self.content {
                continue;
            }
            let key = GlyphKey::from_glyph(glyph);
            if !requested.insert(key) {
                continue;
            }
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.last_used = self.clock;
            } else {
                missing.push((key, glyph_pixels(glyph)));
            }
        }
        let mut allocator = self.allocator;
        let mut placements = Vec::with_capacity(missing.len());
        for (key, _) in &missing {
            let Some(placement) = allocator.allocate(key.width, key.height) else {
                return self.rebuild(queue, glyphs, &requested);
            };
            placements.push(placement);
        }
        self.allocator = allocator;
        for ((key, pixels), (x, y)) in missing.into_iter().zip(placements) {
            let entry = GlyphAtlasEntry {
                x,
                y,
                width: key.width,
                height: key.height,
                last_used: self.clock,
                pixels,
            };
            upload_glyph(queue, &self.texture, &entry, self.bytes_per_pixel);
            self.entries.insert(key, entry);
        }
        Ok(())
    }

    fn rebuild(
        &mut self,
        queue: &wgpu::Queue,
        glyphs: &[PreparedGlyph],
        requested: &HashSet<GlyphKey>,
    ) -> Result<(), GpuError> {
        let old = std::mem::take(&mut self.entries);
        let mut requested_entries = Vec::new();
        let mut seen = HashSet::new();
        for prepared in glyphs {
            let glyph = &prepared.glyph;
            if glyph.content != self.content {
                continue;
            }
            let key = GlyphKey::from_glyph(glyph);
            if !seen.insert(key) {
                continue;
            }
            let pixels = old
                .get(&key)
                .map_or_else(|| glyph_pixels(glyph), |entry| entry.pixels.clone());
            requested_entries.push((key, pixels));
        }
        let mut retained: Vec<_> = old
            .into_iter()
            .filter(|(key, _)| !requested.contains(key))
            .collect();
        retained.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_used));
        self.allocator = ShelfAllocator::default();
        for (key, pixels) in requested_entries {
            let Some((x, y)) = self.allocator.allocate(key.width, key.height) else {
                return Err(GpuError(
                    "visible glyphs exceed the persistent atlas capacity".to_owned(),
                ));
            };
            let entry = GlyphAtlasEntry {
                x,
                y,
                width: key.width,
                height: key.height,
                last_used: self.clock,
                pixels,
            };
            upload_glyph(queue, &self.texture, &entry, self.bytes_per_pixel);
            self.entries.insert(key, entry);
        }
        for (key, mut entry) in retained {
            let Some((x, y)) = self.allocator.allocate(key.width, key.height) else {
                continue;
            };
            entry.x = x;
            entry.y = y;
            upload_glyph(queue, &self.texture, &entry, self.bytes_per_pixel);
            self.entries.insert(key, entry);
        }
        Ok(())
    }
}

fn glyph_pixels(glyph: &RasterGlyph) -> Vec<u8> {
    let pixel_count = glyph.width as usize * glyph.height as usize;
    match glyph.content {
        RasterContent::Mask if glyph.data.len() >= pixel_count * 3 => {
            let mut pixels = Vec::with_capacity(pixel_count);
            for index in 0..pixel_count {
                let at = index * 3;
                pixels.push(glyph.data[at..at + 3].iter().copied().max().unwrap_or(0));
            }
            pixels
        }
        RasterContent::Mask => glyph.data[..pixel_count].to_vec(),
        RasterContent::Color => glyph.data[..pixel_count * 4].to_vec(),
    }
}

fn upload_glyph(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    entry: &GlyphAtlasEntry,
    bytes_per_pixel: u32,
) {
    let padded_width = entry.width + 2;
    let padded_height = entry.height + 2;
    let bytes_per_pixel = bytes_per_pixel as usize;
    let mut padded = vec![0; padded_width as usize * padded_height as usize * bytes_per_pixel];
    for row in 0..entry.height as usize {
        let source = row * entry.width as usize * bytes_per_pixel;
        let destination = ((row + 1) * padded_width as usize + 1) * bytes_per_pixel;
        let row_bytes = entry.width as usize * bytes_per_pixel;
        padded[destination..destination + row_bytes]
            .copy_from_slice(&entry.pixels[source..source + row_bytes]);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: entry.x - 1,
                y: entry.y - 1,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(padded_width * bytes_per_pixel as u32),
            rows_per_image: Some(padded_height),
        },
        wgpu::Extent3d {
            width: padded_width,
            height: padded_height,
            depth_or_array_layers: 1,
        },
    );
}

