use super::FORMAT;
use crate::DamageRect;
use wgpu::util::DeviceExt;

use super::{backend_types::*, shaders::*, textures::*};

pub(crate) fn create_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("morf persistent target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

pub(crate) fn create_blur_chain(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    source: &wgpu::TextureView,
    width: u32,
    height: u32,
    offset: f32,
) -> BlurChain {
    let half = ((width / 2).max(1), (height / 2).max(1));
    let quarter = ((width / 4).max(1), (height / 4).max(1));
    let sizes = [half, quarter, half, (width.max(1), height.max(1))];
    let mut textures = Vec::with_capacity(4);
    let mut views = Vec::with_capacity(4);
    for (target_width, target_height) in sizes {
        let (texture, view) = create_target(device, target_width, target_height);
        textures.push(texture);
        views.push(view);
    }
    let sources = [source, &views[0], &views[1], &views[2]];
    let source_sizes = [(width.max(1), height.max(1)), half, quarter, half];
    let mut passes = Vec::with_capacity(4);
    for index in 0..4 {
        passes.push(create_blur_pass(
            device,
            layout,
            sampler,
            sources[index],
            source_sizes[index],
            offset,
            if index < 2 { 0.0 } else { 1.0 },
        ));
    }
    BlurChain {
        _textures: textures,
        views,
        passes,
    }
}

pub(crate) fn create_blur_pass(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    source: &wgpu::TextureView,
    source_size: (u32, u32),
    offset: f32,
    mode: f32,
) -> BlurPass {
    let params = [
        1.0 / source_size.0.max(1) as f32,
        1.0 / source_size.1.max(1) as f32,
        offset,
        mode,
    ];
    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("morf blur parameters"),
        contents: bytemuck::cast_slice(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("morf blur bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffer.as_entire_binding(),
            },
        ],
    });
    BlurPass {
        _params: buffer,
        bind_group,
    }
}

pub(crate) struct SurfaceState {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) texture_layout: wgpu::BindGroupLayout,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) bind_group: wgpu::BindGroup,
    /// Whether the surface asked to be reconfigured while a frame was still in
    /// hand, so it has to be done before the next one is acquired.
    pub(crate) stale: bool,
}

pub(crate) fn create_surface_state(
    device: &wgpu::Device,
    adapter: &wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    target_view: &wgpu::TextureView,
    width: u32,
    height: u32,
) -> Result<SurfaceState, GpuError> {
    let capabilities = surface.get_capabilities(adapter);
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| capabilities.formats.first().copied())
        .ok_or_else(|| GpuError("GPU surface exposes no texture format".to_owned()))?;
    let present_mode = capabilities
        .present_modes
        .iter()
        .copied()
        .find(|mode| *mode == wgpu::PresentMode::Fifo)
        .or_else(|| capabilities.present_modes.first().copied())
        .ok_or_else(|| GpuError("GPU surface exposes no presentation mode".to_owned()))?;
    let alpha_mode = capabilities
        .alpha_modes
        .iter()
        .copied()
        .find(|mode| *mode == wgpu::CompositeAlphaMode::PreMultiplied)
        .or_else(|| capabilities.alpha_modes.first().copied())
        .ok_or_else(|| GpuError("GPU surface exposes no alpha mode".to_owned()))?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.max(1),
        height: height.max(1),
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    };
    surface.configure(device, &config);
    let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("morf composite texture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("morf composite sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = create_composite_bind_group(device, &texture_layout, target_view, &sampler);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("morf composite pipeline layout"),
        bind_group_layouts: &[Some(&texture_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("morf composite shader"),
        source: wgpu::ShaderSource::Wgsl(
            fullscreen_source(include_str!("../composite.wgsl")).into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("morf composite pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    });
    Ok(SurfaceState {
        stale: false,
        surface,
        config,
        pipeline,
        texture_layout,
        sampler,
        bind_group,
    })
}

pub(crate) fn create_composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("morf composite bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// The next image to draw into, or `None` if this frame should be skipped.
///
/// Not every unsuccessful acquisition is a failure, and wgpu says as much for
/// each one. A timeout means the compositor has not released a buffer yet, and
/// occlusion means the surface is not on screen to draw to; the documented
/// answer to both is to skip this frame and try the next. Treating them as
/// errors instead took the whole surface down — which is how a shell died with
/// `could not acquire GPU surface: Timeout` for what is, on a busy compositor
/// driving three large outputs, an ordinary event.
///
/// The genuinely broken states still error. The difference is that they are now
/// the ones wgpu describes that way.
pub(crate) fn acquire_frame(
    device: &wgpu::Device,
    surface: &mut SurfaceState,
) -> Result<Option<wgpu::SurfaceTexture>, GpuError> {
    // Anything the last frame asked for, done now — before a texture is in
    // hand rather than while one is, which is the only moment it is allowed.
    if surface.stale {
        surface.surface.configure(device, &surface.config);
        surface.stale = false;
    }
    match surface.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => Ok(Some(frame)),
        // Suboptimal hands back a frame that is perfectly drawable — it only
        // says the surface no longer matches the swapchain, which is what a
        // compositor reports when it rescales or rotates a window. So it is
        // drawn, and the reconfigure waits for the next acquire.
        //
        // Reconfiguring here instead is a validation error, and a fatal one:
        // wgpu requires the surface texture to be dropped first, and this still
        // held it. Nothing on the desk ever returned Suboptimal, so nothing
        // ever hit it; a phone whose compositor scales the window returns it on
        // the very first frame and the process aborts before drawing anything.
        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            surface.stale = true;
            Ok(Some(frame))
        }
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => Ok(None),
        // Outdated wants a reconfigure; lost wants the surface recreated, which
        // needs a window handle this layer does not hold — so it gets the
        // reconfigure too, because attempting it and failing is no worse than
        // failing immediately and is sometimes enough.
        status @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost) => {
            surface.surface.configure(device, &surface.config);
            match surface.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(Some(frame)),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    Ok(None)
                }
                after => Err(GpuError(format!(
                    "could not acquire GPU surface after {status:?}: {after:?}"
                ))),
            }
        }
        status => Err(GpuError(format!(
            "could not acquire GPU surface: {status:?}"
        ))),
    }
}

pub(crate) fn clamp_scissor(
    damage: DamageRect,
    target_width: u32,
    target_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x = damage.x.min(target_width);
    let y = damage.y.min(target_height);
    let right = damage.x.saturating_add(damage.width).min(target_width);
    let bottom = damage.y.saturating_add(damage.height).min(target_height);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    (width > 0 && height > 0).then_some((x, y, width, height))
}

pub(crate) fn intersect_damage(left: DamageRect, right: DamageRect) -> Option<DamageRect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let bottom_edge = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    if right_edge <= x || bottom_edge <= y {
        return None;
    }
    Some(DamageRect {
        x,
        y,
        width: right_edge - x,
        height: bottom_edge - y,
    })
}
