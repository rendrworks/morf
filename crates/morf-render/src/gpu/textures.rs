use crate::effects::color_array;
use crate::{DistanceFieldStyle, DrawCommand, DrawList, ImageFillMode};
use morf_image::ImageCache;
use morf_layout::{Geometry, Transform2D};
use morf_scene::Color;
use std::collections::{HashMap, HashSet};

use super::glyphs::*;

#[derive(Clone)]
pub(crate) struct TextureImage {
    pub(crate) _texture: wgpu::Texture,
    pub(crate) bind_group: wgpu::BindGroup,
}

/// How many decoded images the GPU keeps around.
///
/// The cache is keyed on the pixel size an image was rasterised at, and that
/// size comes off live geometry — so animating an icon's width from 16 to 256
/// mints a texture per step. Unbounded, that is a leak with a config-reachable
/// trigger; bounded, it is a cache that holds the sizes actually in use and
/// lets a one-off animation frame fall out again.
pub(crate) const MAX_IMAGE_TEXTURES: usize = 192;

/// Drops the least recently used textures until the cache is within bounds.
pub(crate) fn evict_image_textures(
    textures: &mut HashMap<TextureKey, TextureImage>,
    used: &HashSet<TextureKey>,
) {
    if textures.len() <= MAX_IMAGE_TEXTURES {
        return;
    }
    // Anything drawn this frame stays, whatever the bound says: evicting it
    // would only force it to be decoded again before the next paint.
    textures.retain(|key, _| used.contains(key));
}

pub(crate) struct LayerTarget {
    pub(crate) _texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) instance: u32,
    pub(crate) blur: Option<BlurChain>,
    pub(crate) shadow_bind_group: Option<wgpu::BindGroup>,
    pub(crate) shadow_instance: Option<u32>,
    pub(crate) shadow: Option<BlurChain>,
}

pub(crate) struct BlurChain {
    pub(crate) _textures: Vec<wgpu::Texture>,
    pub(crate) views: Vec<wgpu::TextureView>,
    pub(crate) passes: Vec<BlurPass>,
}

pub(crate) struct BlurPass {
    pub(crate) _params: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TextureKey {
    pub(crate) source: String,
    pub(crate) theme: Option<String>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_120: u32,
    pub(crate) distance_field: bool,
    pub(crate) distance_field_spread: u32,
}

#[derive(Default)]
pub(crate) struct TextureBatch {
    pub(crate) instances: Vec<GlyphInstance>,
    pub(crate) images: Vec<TextureImage>,
    pub(crate) command_instances: Vec<Option<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TexturePlacement {
    pub(crate) bounds: Geometry,
    pub(crate) transform: Transform2D,
    pub(crate) logical_width: u32,
    pub(crate) logical_height: u32,
    pub(crate) uv: [f32; 4],
}

#[derive(Clone, Copy)]
pub(crate) struct TextureStyle {
    pub(crate) overlay: Color,
    pub(crate) distance_field: bool,
    pub(crate) field: DistanceFieldStyle,
    /// Source-pixel range the cached field encodes on either side of the edge.
    pub(crate) spread: f32,
}

pub(crate) struct TextureBatchContext<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) layout: &'a wgpu::BindGroupLayout,
    pub(crate) sampler: &'a wgpu::Sampler,
    pub(crate) target_size: (u32, u32),
}

pub(crate) fn create_texture_batch(
    context: TextureBatchContext<'_>,
    cache: &mut ImageCache,
    textures: &mut HashMap<TextureKey, TextureImage>,
    list: &DrawList,
    scale_120: u32,
) -> TextureBatch {
    let mut batch = TextureBatch {
        command_instances: vec![None; list.commands.len()],
        ..TextureBatch::default()
    };
    let scale = scale_120.max(1) as f64 / 120.0;
    let mut used: HashSet<TextureKey> = HashSet::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Texture {
            bounds,
            transform,
            source,
            icon_theme,
            color_overlay,
            fill_mode,
            distance_field,
            distance_field_spread,
            distance_field_style,
            ..
        } = command
        else {
            continue;
        };
        if source.is_empty() || bounds.width <= 0.0 || bounds.height <= 0.0 {
            continue;
        }
        let preferred = bounds.width.max(bounds.height).ceil().max(1.0) as u32;
        let intrinsic = match icon_theme {
            Some(theme) => cache.icon_intrinsic_size(source, theme, preferred).ok(),
            None => cache.intrinsic_size(source).ok(),
        }
        .unwrap_or((bounds.width.ceil() as u32, bounds.height.ceil() as u32));
        let placement = texture_placement(*bounds, intrinsic, *fill_mode, *transform);
        let logical_width = placement.logical_width;
        let logical_height = placement.logical_height;
        let key = TextureKey {
            source: source.clone(),
            theme: icon_theme.clone(),
            width: logical_width,
            height: logical_height,
            scale_120,
            distance_field: *distance_field,
            distance_field_spread: distance_field_spread.to_bits(),
        };
        used.insert(key.clone());
        if let Some(image) = textures.get(&key) {
            push_texture_instance(
                &mut batch,
                command_index,
                image.clone(),
                placement,
                TextureStyle {
                    overlay: *color_overlay,
                    distance_field: *distance_field,
                    field: *distance_field_style,
                    spread: *distance_field_spread,
                },
                context.target_size,
                scale,
            );
            continue;
        }
        let loaded = match (icon_theme, distance_field) {
            (Some(theme), true) => cache.load_icon_distance_field_sized(
                source,
                theme,
                logical_width,
                logical_height,
                scale_120,
                *distance_field_spread,
            ),
            (Some(theme), false) => {
                cache.load_icon_sized(source, theme, logical_width, logical_height, scale_120)
            }
            (None, true) => cache.load_distance_field(
                source,
                logical_width,
                logical_height,
                scale_120,
                *distance_field_spread,
            ),
            (None, false) => cache.load(source, logical_width, logical_height, scale_120),
        };
        let Ok(image) = loaded else {
            continue;
        };
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("morf image texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("morf image bind group"),
                layout: context.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(context.sampler),
                    },
                ],
            });
        let (target_width, target_height) = context.target_size;
        let texture_image = TextureImage {
            _texture: texture,
            bind_group,
        };
        textures.insert(key, texture_image.clone());
        push_texture_instance(
            &mut batch,
            command_index,
            texture_image,
            placement,
            TextureStyle {
                overlay: *color_overlay,
                distance_field: *distance_field,
                field: *distance_field_style,
                spread: *distance_field_spread,
            },
            (target_width, target_height),
            scale,
        );
    }
    evict_image_textures(textures, &used);
    cache.shrink();

    batch
}

pub(crate) fn texture_placement(
    bounds: Geometry,
    intrinsic: (u32, u32),
    fill_mode: ImageFillMode,
    transform: Transform2D,
) -> TexturePlacement {
    let source_width = f64::from(intrinsic.0.max(1));
    let source_height = f64::from(intrinsic.1.max(1));
    match fill_mode {
        ImageFillMode::Stretch => TexturePlacement {
            bounds,
            transform,
            logical_width: bounds.width.ceil().max(1.0) as u32,
            logical_height: bounds.height.ceil().max(1.0) as u32,
            uv: [0.0, 0.0, 1.0, 1.0],
        },
        ImageFillMode::PreserveAspectFit => {
            let scale = (bounds.width / source_width).min(bounds.height / source_height);
            let width = source_width * scale;
            let height = source_height * scale;
            TexturePlacement {
                bounds: Geometry {
                    x: bounds.x + (bounds.width - width) / 2.0,
                    y: bounds.y + (bounds.height - height) / 2.0,
                    width,
                    height,
                },
                transform,
                logical_width: width.ceil().max(1.0) as u32,
                logical_height: height.ceil().max(1.0) as u32,
                uv: [0.0, 0.0, 1.0, 1.0],
            }
        }
        ImageFillMode::PreserveAspectCrop => {
            let scale = (bounds.width / source_width).max(bounds.height / source_height);
            let width = source_width * scale;
            let height = source_height * scale;
            let uv_width = (bounds.width / width) as f32;
            let uv_height = (bounds.height / height) as f32;
            TexturePlacement {
                bounds,
                transform,
                logical_width: width.ceil().max(1.0) as u32,
                logical_height: height.ceil().max(1.0) as u32,
                uv: [
                    (1.0 - uv_width) / 2.0,
                    (1.0 - uv_height) / 2.0,
                    uv_width,
                    uv_height,
                ],
            }
        }
    }
}

/// Converts the field style into the units the shader samples in.
///
/// The cached texture maps `[-spread, spread]` source pixels onto `[0, 1]`, so
/// a width expressed in pixels has to be divided by the full span to land in
/// the same space as the sampled value.
/// The same uniform for a glyph, whose field was measured at a fixed size.
///
/// A glyph's spread is in reference pixels, not in the pixels it is drawn at,
/// so an outline asked for in logical pixels has to be converted through the
/// ratio between the two. Doing it here rather than in the configuration is
/// what lets an outline width mean the same thing at every font size.
/// Extra edge outset for small text, in logical pixels.
///
/// A hinted rasterizer snaps a stem onto the pixel grid, so a one-pixel stem is
/// one solid pixel. A field has no hinting: the same stem lands wherever the
/// outline puts it, usually spread across two pixels at part strength each, and
/// the letter reads lighter than the hinted one it replaced. Moving the edge out
/// by a fraction of a pixel gives that back.
///
/// Only where it is the problem. Above the fade the stems are wide enough that
/// the grid no longer decides how solid they look, and the same outset there
/// would simply be a heavier font than the one asked for.
fn hinting_bias(size: f64) -> f32 {
    const FULL_BELOW: f64 = 10.0;
    const NONE_ABOVE: f64 = 20.0;
    const OUTSET: f32 = 0.18;
    let reach = ((NONE_ABOVE - size) / (NONE_ABOVE - FULL_BELOW)).clamp(0.0, 1.0);
    OUTSET * reach as f32
}

pub(crate) fn glyph_field_uniform(style: DistanceFieldStyle, size: f64) -> [f32; 4] {
    // How much of the field one logical pixel covers at this size. Asked for
    // rather than derived here: the spread is capped, so it is no longer a
    // fixed fraction of the reference and a second copy of the arithmetic would
    // disagree with the first.
    let per_pixel = morf_text::field_units_per_logical_px(size.max(1.0) as f32);
    [
        // Positive thickness moves the edge outwards, which is the direction
        // that adds ink — the field counts upwards away from the glyph.
        0.5 + (style.thickness + hinting_bias(size)) * per_pixel,
        style.softness * per_pixel,
        style.outline_width * per_pixel,
        0.0,
    ]
}

pub(crate) fn distance_field_uniform(style: DistanceFieldStyle, spread: f32) -> [f32; 4] {
    let span = (spread.max(0.5) * 2.0).max(f32::EPSILON);
    [
        // The same neutral edge and the same signed offset the glyph path
        // uses. This used to pass `weight` through as an absolute threshold,
        // so one struct field meant two different things depending on which
        // producer had filled it in.
        0.5 + style.thickness / span,
        style.softness / span,
        style.outline_width / span,
        0.0,
    ]
}

pub(crate) fn push_texture_instance(
    batch: &mut TextureBatch,
    command_index: usize,
    image: TextureImage,
    placement: TexturePlacement,
    style: TextureStyle,
    target_size: (u32, u32),
    scale: f64,
) {
    let bounds = placement.bounds;
    let (origin, axes) = transformed_quad(placement.transform, bounds, scale, target_size);
    batch.command_instances[command_index] = Some(batch.instances.len() as u32);
    batch.instances.push(GlyphInstance {
        origin,
        axes,
        uv: placement.uv,
        color: [1.0, 1.0, 1.0, 1.0],
        color_overlay: color_array(style.overlay),
        mode: [
            0.0,
            0.0,
            f32::from(style.distance_field),
            f32::from(style.distance_field),
        ],
        field: distance_field_uniform(style.field, style.spread),
        outline_color: color_array(style.field.outline_color),
        ..GlyphInstance::default()
    });
    batch.images.push(image);
}

impl super::backend_types::WgpuBackend {
    /// Uploads one decoded image as a texture a shader can sample.
    ///
    /// Kept apart from the image-texture cache: that one is keyed by node and
    /// evicted when a node dies, and a shader's textures live as long as the
    /// shader does.
    pub(crate) fn upload_shader_texture(&self, image: &morf_image::ImageData) -> wgpu::TextureView {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("morf shader texture"),
            size: wgpu::Extent3d {
                width: image.width.max(1),
                height: image.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width.max(1) * 4),
                rows_per_image: Some(image.height.max(1)),
            },
            wgpu::Extent3d {
                width: image.width.max(1),
                height: image.height.max(1),
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }
}
