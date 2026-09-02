use crate::LayerMask;
use morf_layout::{Geometry, Transform2D};
use morf_scene::Color;
use morf_text::{RasterContent, RasterGlyph};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::backend_types::*;

#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Default)]
pub(crate) struct GlyphInstance {
    pub(crate) origin: [f32; 2],
    pub(crate) axes: [f32; 4],
    pub(crate) uv: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) color_overlay: [f32; 4],
    pub(crate) mode: [f32; 4],
    pub(crate) surface: [f32; 4],
    pub(crate) mask_bounds: [f32; 4],
    pub(crate) mask_inverse_0: [f32; 4],
    pub(crate) mask_inverse_1: [f32; 4],
    pub(crate) mask_radii: [f32; 4],
    /// Field edge, feathering, outline width, and how far this glyph has
    /// travelled towards the one it is morphing into.
    pub(crate) field: [f32; 4],
    /// Outline colour composited beneath a distance-field fill.
    pub(crate) outline_color: [f32; 4],
    /// Atlas rect of the glyph being morphed towards.
    ///
    /// Last, and it has to stay last: the vertex attributes are laid out by
    /// offset, so a field inserted higher up would be read as whichever
    /// attribute used to sit at that offset.
    ///
    /// A zero size means there is nothing opposite this glyph — the text it is
    /// turning into is shorter — and the shader reads "outside" there instead,
    /// so an unpaired letter dissolves rather than snapping away.
    pub(crate) morph_uv: [f32; 4],
    /// How much the field changes across one device pixel.
    ///
    /// Known exactly from the size the glyph is drawn at, so the edge does not
    /// have to guess it from the gradient of a sampled texture — which is a
    /// noisy thing to measure once the field is minified, and reads as an edge
    /// that will not settle.
    pub(crate) ramp: f32,
}

pub(crate) fn layer_mask_data(
    mask: Option<LayerMask>,
) -> (f32, [f32; 4], [f32; 4], [f32; 4], [f32; 4]) {
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

pub(crate) fn transformed_quad(
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

pub(crate) struct GlyphBatch {
    pub(crate) instances: Vec<GlyphInstance>,
    pub(crate) command_spans: Vec<Vec<GlyphSpan>>,
}

pub(crate) struct GlyphSpan {
    pub(crate) range: Range<u32>,
    pub(crate) color: bool,
}

pub(crate) const GLYPH_ATLAS_SIZE: u32 = 2048;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GlyphKey {
    pub(crate) id: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl GlyphKey {
    pub(crate) fn from_glyph(glyph: &RasterGlyph) -> Self {
        Self {
            id: glyph.cache_key,
            width: glyph.width,
            height: glyph.height,
        }
    }
}

impl PreparedGlyph {
    /// Every glyph this one needs in the atlas — its own, and the one it is
    /// morphing into. A partner that is never uploaded is a partner the shader
    /// samples as empty space.
    pub(crate) fn sources(&self) -> impl Iterator<Item = &RasterGlyph> {
        std::iter::once(&self.glyph).chain(self.morph.iter())
    }
}

pub(crate) struct GlyphAtlasEntry {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) last_used: u64,
    pub(crate) pixels: Vec<u8>,
    /// The byte the one-pixel gutter around this entry is filled with.
    ///
    /// Not always zero, because the atlas holds two kinds of thing that
    /// disagree about what an empty byte means. For coverage, zero is "no
    /// ink" and the gutter is simply blank. For a distance field, zero is the
    /// *inside* of the glyph — so a zeroed gutter reads as solid ink, and
    /// linear filtering at the quad's edge drags it into frame as a bright
    /// rectangle around every glyph drawn from a field.
    pub(crate) outside: u8,
}

/// The byte that means "nothing here" for a given kind of atlas content.
pub(crate) fn outside_byte(content: RasterContent) -> u8 {
    match content {
        // Furthest outside the glyph the encoded spread can express, in every
        // channel — the median of three "far outside" is far outside.
        RasterContent::Field => u8::MAX,
        RasterContent::Mask | RasterContent::Color => 0,
    }
}

pub(crate) struct PreparedGlyph {
    pub(crate) glyph: RasterGlyph,
    /// The glyph this one is turning into, when it has a partner.
    pub(crate) morph: Option<RasterGlyph>,
    /// How far between the two, zero at `glyph` and one at `morph`.
    ///
    /// A glyph with no partner still carries a progress: it interpolates
    /// towards the far-outside value instead, which is how a letter with
    /// nothing opposite it dissolves.
    pub(crate) morph_progress: f32,
    /// The field's change across one device pixel.
    pub(crate) ramp: f32,
    pub(crate) color: Color,
    pub(crate) color_overlay: Color,
    pub(crate) transform: Transform2D,
    pub(crate) command_index: usize,
    /// Edge, softness and outline width, already in sampled-field units.
    pub(crate) field: [f32; 4],
    /// Outline colour, composited beneath the fill.
    pub(crate) outline_color: [f32; 4],
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ShelfAllocator {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) row_height: u32,
}

impl ShelfAllocator {
    pub(crate) fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
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

pub(crate) struct GlyphAtlas {
    pub(crate) texture: wgpu::Texture,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) content: RasterContent,
    pub(crate) bytes_per_pixel: u32,
    pub(crate) entries: HashMap<GlyphKey, GlyphAtlasEntry>,
    pub(crate) allocator: ShelfAllocator,
    pub(crate) clock: u64,
}

impl GlyphAtlas {
    /// Whether this atlas is the one a glyph of that kind belongs in.
    ///
    /// Coverage and distance are both one byte a pixel, so they share the
    /// single-channel atlas and differ only in how the shader reads them;
    /// colour glyphs need four and have their own.
    /// Whether this atlas holds that kind of glyph.
    ///
    /// A field is three channels and a mask is one, but they share an atlas: it
    /// is four bytes a texel and linear, and a mask simply uses the first of
    /// them. A second atlas for the handful of glyphs with no outline to trace
    /// would cost more than the channels do.
    pub(crate) fn accepts(&self, content: RasterContent) -> bool {
        match self.content {
            RasterContent::Mask | RasterContent::Field => {
                matches!(content, RasterContent::Mask | RasterContent::Field)
            }
            RasterContent::Color => content == RasterContent::Color,
        }
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        content: RasterContent,
    ) -> Self {
        let (format, bytes_per_pixel, label) = match content {
            RasterContent::Mask | RasterContent::Field => {
                // Linear, not sRGB: these are distances, and a transfer curve
                // applied to a distance is a different shape.
                (wgpu::TextureFormat::Rgba8Unorm, 4, "morf glyph field atlas")
            }
            RasterContent::Color => (
                wgpu::TextureFormat::Rgba8UnormSrgb,
                4,
                "morf color glyph atlas",
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

    pub(crate) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        glyphs: &[PreparedGlyph],
    ) -> Result<(), GpuError> {
        self.clock = self.clock.wrapping_add(1);
        let mut requested = HashSet::new();
        let mut missing = Vec::new();
        for glyph in glyphs.iter().flat_map(PreparedGlyph::sources) {
            if !self.accepts(glyph.content) {
                continue;
            }
            let key = GlyphKey::from_glyph(glyph);
            if !requested.insert(key) {
                continue;
            }
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.last_used = self.clock;
            } else {
                missing.push((key, glyph_pixels(glyph), outside_byte(glyph.content)));
            }
        }
        let mut allocator = self.allocator;
        let mut placements = Vec::with_capacity(missing.len());
        for (key, _, _) in &missing {
            let Some(placement) = allocator.allocate(key.width, key.height) else {
                return self.rebuild(queue, glyphs, &requested);
            };
            placements.push(placement);
        }
        self.allocator = allocator;
        for ((key, pixels, outside), (x, y)) in missing.into_iter().zip(placements) {
            let entry = GlyphAtlasEntry {
                x,
                y,
                width: key.width,
                height: key.height,
                last_used: self.clock,
                pixels,
                outside,
            };
            upload_glyph(queue, &self.texture, &entry, self.bytes_per_pixel);
            self.entries.insert(key, entry);
        }
        Ok(())
    }

    pub(crate) fn rebuild(
        &mut self,
        queue: &wgpu::Queue,
        glyphs: &[PreparedGlyph],
        requested: &HashSet<GlyphKey>,
    ) -> Result<(), GpuError> {
        let old = std::mem::take(&mut self.entries);
        let mut requested_entries = Vec::new();
        let mut seen = HashSet::new();
        for glyph in glyphs.iter().flat_map(PreparedGlyph::sources) {
            if !self.accepts(glyph.content) {
                continue;
            }
            let key = GlyphKey::from_glyph(glyph);
            if !seen.insert(key) {
                continue;
            }
            let pixels = old
                .get(&key)
                .map_or_else(|| glyph_pixels(glyph), |entry| entry.pixels.clone());
            requested_entries.push((key, pixels, outside_byte(glyph.content)));
        }
        let mut retained: Vec<_> = old
            .into_iter()
            .filter(|(key, _)| !requested.contains(key))
            .collect();
        retained.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.last_used));
        self.allocator = ShelfAllocator::default();
        for (key, pixels, outside) in requested_entries {
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
                outside,
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

pub(crate) fn glyph_pixels(glyph: &RasterGlyph) -> Vec<u8> {
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
        // Four bytes a texel: three channels sharing the outline's edges out
        // between them, and the plain distance alongside.
        RasterContent::Field | RasterContent::Color => glyph.data[..pixel_count * 4].to_vec(),
    }
}

pub(crate) fn upload_glyph(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    entry: &GlyphAtlasEntry,
    bytes_per_pixel: u32,
) {
    let padded_width = entry.width + 2;
    let padded_height = entry.height + 2;
    let bytes_per_pixel = bytes_per_pixel as usize;
    let mut padded =
        vec![entry.outside; padded_width as usize * padded_height as usize * bytes_per_pixel];
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
