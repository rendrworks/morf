#[derive(Clone)]
struct TextureImage {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct LayerTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    instance: u32,
    blur: Option<BlurChain>,
    shadow_bind_group: Option<wgpu::BindGroup>,
    shadow_instance: Option<u32>,
    shadow: Option<BlurChain>,
}

struct BlurChain {
    _textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    passes: Vec<BlurPass>,
}

struct BlurPass {
    _params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextureKey {
    source: String,
    theme: Option<String>,
    width: u32,
    height: u32,
    scale_120: u32,
    distance_field: bool,
    distance_field_spread: u32,
}

#[derive(Default)]
struct TextureBatch {
    instances: Vec<GlyphInstance>,
    images: Vec<TextureImage>,
    command_instances: Vec<Option<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TexturePlacement {
    bounds: Geometry,
    transform: Transform2D,
    logical_width: u32,
    logical_height: u32,
    uv: [f32; 4],
}

#[derive(Clone, Copy)]
struct TextureStyle {
    opacity: f32,
    overlay: Color,
    distance_field: bool,
    field: DistanceFieldStyle,
    /// Source-pixel range the cached field encodes on either side of the edge.
    spread: f32,
}

struct TextureBatchContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    target_size: (u32, u32),
}

fn create_texture_batch(
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
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Texture {
            bounds,
            transform,
            source,
            icon_theme,
            opacity,
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
        if let Some(image) = textures.get(&key) {
            push_texture_instance(
                &mut batch,
                command_index,
                image.clone(),
                placement,
                TextureStyle {
                    opacity: *opacity,
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
            label: Some("mold image texture"),
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
                label: Some("mold image bind group"),
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
                opacity: *opacity,
                overlay: *color_overlay,
                distance_field: *distance_field,
                field: *distance_field_style,
                spread: *distance_field_spread,
            },
            (target_width, target_height),
            scale,
        );
    }
    batch
}

fn texture_placement(
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
fn distance_field_uniform(style: DistanceFieldStyle, spread: f32) -> [f32; 4] {
    let span = (spread.max(0.5) * 2.0).max(f32::EPSILON);
    [
        style.weight,
        style.softness / span,
        style.outline_width / span,
        0.0,
    ]
}

fn push_texture_instance(
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
        color: [1.0, 1.0, 1.0, style.opacity],
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

struct GlyphBatchContext<'a> {
    queue: &'a wgpu::Queue,
    mask_atlas: &'a mut GlyphAtlas,
    color_atlas: &'a mut GlyphAtlas,
    target_size: (u32, u32),
}

fn create_glyph_batch(
    context: GlyphBatchContext<'_>,
    text_system: &mut TextSystem,
    list: &DrawList,
    scale_120: u32,
) -> Result<Option<GlyphBatch>, GpuError> {
    let GlyphBatchContext {
        queue,
        mask_atlas,
        color_atlas,
        target_size: (target_width, target_height),
    } = context;
    let scale = scale_120.max(1) as f32 / 120.0;
    let mut glyphs = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Text {
            node,
            bounds,
            transform,
            text,
            family,
            font_source,
            size,
            font_weight,
            color,
            color_overlay,
            wrap,
            elide,
            horizontal_alignment,
            vertical_alignment,
            ..
        } = command
        else {
            continue;
        };
        let measured = text_system.measure(
            *node,
            text,
            family,
            *size,
            TextOptions {
                width: Some(bounds.width),
                wrap: *wrap,
                alignment: *horizontal_alignment,
                elide: *elide,
                font_weight: *font_weight,
                font_source: (!font_source.is_empty()).then(|| font_source.clone()),
            },
        );
        let spare_height = (bounds.height - measured.height).max(0.0);
        let vertical_offset = match vertical_alignment {
            VerticalAlignment::Top => 0.0,
            VerticalAlignment::Center => spare_height / 2.0,
            VerticalAlignment::Bottom => spare_height,
        };
        for glyph in text_system.rasterize(
            *node,
            (
                bounds.x as f32 * scale,
                (bounds.y + vertical_offset) as f32 * scale,
            ),
            scale,
        ) {
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push(PreparedGlyph {
                    glyph,
                    color: *color,
                    color_overlay: *color_overlay,
                    transform: *transform,
                    command_index,
                });
            }
        }
    }
    if glyphs.is_empty() {
        return Ok(None);
    }
    mask_atlas.prepare(queue, &glyphs)?;
    color_atlas.prepare(queue, &glyphs)?;
    let mut instances = Vec::with_capacity(glyphs.len());
    let mut command_spans: Vec<Vec<GlyphSpan>> =
        (0..list.commands.len()).map(|_| Vec::new()).collect();
    for prepared in glyphs {
        let glyph = prepared.glyph;
        let key = GlyphKey::from_glyph(&glyph);
        let color_glyph = glyph.content == RasterContent::Color;
        let atlas = if color_glyph {
            &*color_atlas
        } else {
            &*mask_atlas
        };
        let entry = atlas.entries.get(&key).ok_or_else(|| {
            GpuError("prepared glyph is missing from the persistent atlas".to_owned())
        })?;
        let tint = match glyph.content {
            RasterContent::Mask => color_array(prepared.color),
            RasterContent::Color => [1.0, 1.0, 1.0, prepared.color.alpha],
        };
        let (origin, axes) = transformed_quad(
            prepared.transform,
            Geometry {
                x: f64::from(glyph.x) / f64::from(scale),
                y: f64::from(glyph.y) / f64::from(scale),
                width: f64::from(glyph.width) / f64::from(scale),
                height: f64::from(glyph.height) / f64::from(scale),
            },
            f64::from(scale),
            (target_width, target_height),
        );
        let instance = instances.len() as u32;
        let spans = &mut command_spans[prepared.command_index];
        if let Some(span) = spans.last_mut()
            && span.color == color_glyph
            && span.range.end == instance
        {
            span.range.end = instance + 1;
        } else {
            spans.push(GlyphSpan {
                range: instance..instance + 1,
                color: color_glyph,
            });
        }
        instances.push(GlyphInstance {
            origin,
            axes,
            uv: [
                entry.x as f32 / GLYPH_ATLAS_SIZE as f32,
                entry.y as f32 / GLYPH_ATLAS_SIZE as f32,
                glyph.width as f32 / GLYPH_ATLAS_SIZE as f32,
                glyph.height as f32 / GLYPH_ATLAS_SIZE as f32,
            ],
            color: tint,
            color_overlay: color_array(prepared.color_overlay),
            mode: [0.0, 0.0, if color_glyph { 0.0 } else { 1.0 }, 0.0],
            ..GlyphInstance::default()
        });
    }
    Ok(Some(GlyphBatch {
        instances,
        command_spans,
    }))
}
