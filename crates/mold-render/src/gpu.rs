use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::mem;
use std::ops::Range;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::DeviceExt;

use mold_image::ImageCache;
use mold_layout::{Geometry, Size, TextMeasurer, TextOptions};
use mold_scene::{Element, NodeHandle};
use mold_text::{RasterContent, TextSystem};

use crate::path::PathCache;
use crate::{
    DamageRect, DrawCommand, DrawList, ImageFillMode, RenderBackend, SdfQuadInstance,
    VerticalAlignment,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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

impl WgpuBackend {
    /// Selects a Vulkan or GLES adapter and creates an offscreen render target.
    pub async fn new(width: u32, height: u32) -> Result<Self, GpuError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(descriptor);
        Self::initialize(instance, None, width, height).await
    }

    /// Creates a renderer presenting to an owned native window target.
    pub async fn new_surface<T>(window: T, width: u32, height: u32) -> Result<Self, GpuError>
    where
        T: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window)
            .map_err(|error| GpuError(format!("could not create GPU surface: {error}")))?;
        Self::initialize(instance, Some(surface), width, height).await
    }

    async fn initialize(
        instance: wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: surface.as_ref(),
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| GpuError(format!("no compatible GPU adapter: {error}")))?;
        let adapter_info = adapter.get_info();
        let adapter_limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mold device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter_limits,
                ..Default::default()
            })
            .await
            .map_err(|error| GpuError(format!("could not create GPU device: {error}")))?;
        let viewport_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mold viewport layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let viewport = [width.max(1) as f32, height.max(1) as f32, 0.0, 0.0];
        let viewport_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mold viewport"),
            contents: bytemuck::cast_slice(&viewport),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let viewport_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mold viewport bind group"),
            layout: &viewport_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mold SDF shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sdf.wgsl").into()),
        });
        let clear_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mold damage clear shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("clear.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mold SDF pipeline layout"),
            bind_group_layouts: &[Some(&viewport_layout)],
            immediate_size: 0,
        });
        let pipeline = create_pipeline(&device, &pipeline_layout, &shader, true);
        let clear_pipeline = create_pipeline(&device, &pipeline_layout, &clear_shader, false);
        let path_pipeline = create_path_pipeline(&device, &viewport_layout);
        let (glyph_pipeline, glyph_layout, glyph_sampler) = create_glyph_pipeline(&device);
        let instance_capacity = 1;
        let instance_buffer = create_instance_buffer(&device, instance_capacity);
        let glyph_capacity = 1;
        let glyph_buffer = create_glyph_buffer(&device, glyph_capacity);
        let texture_capacity = 1;
        let texture_buffer = create_instance_buffer_for::<GlyphInstance>(
            &device,
            texture_capacity,
            "mold texture instances",
        );
        let path_vertex_capacity = 1;
        let path_vertex_buffer = create_vertex_buffer_for::<PathVertex>(
            &device,
            path_vertex_capacity,
            "mold path vertices",
        );
        let path_index_capacity = 1;
        let path_index_buffer = create_index_buffer(&device, path_index_capacity);
        let (texture, view) = create_target(&device, width, height);
        let surface = surface
            .map(|surface| create_surface_state(&device, &adapter, surface, &view, width, height))
            .transpose()?;

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            viewport_buffer,
            viewport_bind_group,
            instance_buffer,
            instance_capacity,
            glyph_pipeline,
            glyph_layout,
            glyph_sampler,
            glyph_buffer,
            glyph_capacity,
            texture_buffer,
            texture_capacity,
            path_pipeline,
            path_vertex_buffer,
            path_vertex_capacity,
            path_index_buffer,
            path_index_capacity,
            paths: PathCache::default(),
            images: ImageCache::default(),
            image_textures: HashMap::new(),
            text: TextSystem::new(),
            texture,
            view,
            surface,
            width: width.max(1),
            height: height.max(1),
            info: GpuInfo {
                name: adapter_info.name,
                backend: adapter_info.backend,
                vendor: adapter_info.vendor,
                device: adapter_info.device,
            },
        })
    }

    /// Returns the selected hardware and backend identifiers.
    pub fn info(&self) -> &GpuInfo {
        &self.info
    }

    /// Recreates the physical target and updates shader viewport dimensions.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        (self.texture, self.view) = create_target(&self.device, self.width, self.height);
        let viewport = [self.width as f32, self.height as f32, 0.0, 0.0];
        self.queue
            .write_buffer(&self.viewport_buffer, 0, bytemuck::cast_slice(&viewport));
        if let Some(surface) = &mut self.surface {
            surface.config.width = self.width;
            surface.config.height = self.height;
            surface.surface.configure(&self.device, &surface.config);
            surface.bind_group = create_composite_bind_group(
                &self.device,
                &surface.texture_layout,
                &self.view,
                &surface.sampler,
            );
        }
    }

    /// Returns the persistent target for copying or diagnostics.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Returns the shaping cache used by layout and glyph rendering.
    pub fn text_mut(&mut self) -> &mut TextSystem {
        &mut self.text
    }

    fn ensure_instances(&mut self, required: usize) {
        if required <= self.instance_capacity {
            return;
        }
        self.instance_capacity = required.next_power_of_two();
        self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
    }

    fn ensure_glyphs(&mut self, required: usize) {
        if required <= self.glyph_capacity {
            return;
        }
        self.glyph_capacity = required.next_power_of_two();
        self.glyph_buffer = create_glyph_buffer(&self.device, self.glyph_capacity);
    }

    fn ensure_textures(&mut self, required: usize) {
        if required <= self.texture_capacity {
            return;
        }
        self.texture_capacity = required.next_power_of_two();
        self.texture_buffer = create_instance_buffer_for::<GlyphInstance>(
            &self.device,
            self.texture_capacity,
            "mold texture instances",
        );
    }

    fn ensure_paths(&mut self, vertices: usize, indices: usize) {
        if vertices > self.path_vertex_capacity {
            self.path_vertex_capacity = vertices.next_power_of_two();
            self.path_vertex_buffer = create_vertex_buffer_for::<PathVertex>(
                &self.device,
                self.path_vertex_capacity,
                "mold path vertices",
            );
        }
        if indices > self.path_index_capacity {
            self.path_index_capacity = indices.next_power_of_two();
            self.path_index_buffer = create_index_buffer(&self.device, self.path_index_capacity);
        }
    }
}

impl TextMeasurer for WgpuBackend {
    fn measure(
        &mut self,
        node: NodeHandle,
        text: &str,
        family: &str,
        size: f64,
        options: TextOptions,
    ) -> Size {
        self.text.measure(node, text, family, size, options)
    }

    fn measure_image(
        &mut self,
        _node: NodeHandle,
        element: Element,
        source: &str,
        theme: Option<&str>,
    ) -> Option<Size> {
        if source.is_empty() {
            return None;
        }
        let (width, height) = match element {
            Element::Image => self.images.intrinsic_size(source).ok()?,
            Element::Icon => self
                .images
                .icon_intrinsic_size(source, theme.unwrap_or("hicolor"), 48)
                .ok()?,
            _ => return None,
        };
        Some(Size {
            width: f64::from(width),
            height: f64::from(height),
        })
    }
}

impl RenderBackend for WgpuBackend {
    type Error = GpuError;

    fn render(
        &mut self,
        list: &DrawList,
        damage: &[DamageRect],
        scale_120: u32,
    ) -> Result<(), Self::Error> {
        let mut quad_indices = vec![None; list.commands.len()];
        let mut instances = Vec::new();
        for (command_index, command) in list.commands.iter().enumerate() {
            if let Some(instance) = SdfQuadInstance::from_command(command, scale_120) {
                quad_indices[command_index] = Some(instances.len() as u32);
                instances.push(instance);
            }
        }
        let glyph_batch = create_glyph_batch(
            GlyphBatchContext {
                device: &self.device,
                queue: &self.queue,
                layout: &self.glyph_layout,
                sampler: &self.glyph_sampler,
                target_size: (self.width, self.height),
            },
            &mut self.text,
            list,
            scale_120,
        );
        let texture_batch = create_texture_batch(
            TextureBatchContext {
                device: &self.device,
                queue: &self.queue,
                layout: &self.glyph_layout,
                sampler: &self.glyph_sampler,
                target_size: (self.width, self.height),
            },
            &mut self.images,
            &mut self.image_textures,
            list,
            scale_120,
        );
        let path_batch = create_path_batch(&mut self.paths, list, scale_120)
            .map_err(|error| GpuError(format!("could not prepare path draw: {error}")))?;
        self.ensure_instances(instances.len().max(1));
        self.ensure_textures(texture_batch.instances.len().max(1));
        self.ensure_glyphs(
            glyph_batch
                .as_ref()
                .map_or(1, |batch| batch.instances.len().max(1)),
        );
        self.ensure_paths(
            path_batch.vertices.len().max(1),
            path_batch.indices.len().max(1),
        );
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }
        if let Some(batch) = &glyph_batch {
            self.queue.write_buffer(
                &self.glyph_buffer,
                0,
                bytemuck::cast_slice(&batch.instances),
            );
        }
        if !texture_batch.instances.is_empty() {
            self.queue.write_buffer(
                &self.texture_buffer,
                0,
                bytemuck::cast_slice(&texture_batch.instances),
            );
        }
        if !path_batch.vertices.is_empty() {
            self.queue.write_buffer(
                &self.path_vertex_buffer,
                0,
                bytemuck::cast_slice(&path_batch.vertices),
            );
            self.queue.write_buffer(
                &self.path_index_buffer,
                0,
                bytemuck::cast_slice(&path_batch.indices),
            );
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mold frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mold frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            for damage in damage {
                let Some((x, y, width, height)) = clamp_scissor(*damage, self.width, self.height)
                else {
                    continue;
                };
                pass.set_scissor_rect(x, y, width, height);
                pass.set_pipeline(&self.clear_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.draw(0..3, 0..1);
                for (command_index, quad_instance) in quad_indices.iter().enumerate() {
                    if let Some(instance) = *quad_instance {
                        pass.set_pipeline(&self.pipeline);
                        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                        pass.draw(0..6, instance..instance + 1);
                    }
                    if let Some(instance) = texture_batch.command_instances[command_index] {
                        let image = &texture_batch.images[instance as usize];
                        pass.set_pipeline(&self.glyph_pipeline);
                        pass.set_bind_group(0, &image.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.texture_buffer.slice(..));
                        pass.draw(0..6, instance..instance + 1);
                    }
                    if let Some(batch) = &glyph_batch
                        && let Some(range) = &batch.command_ranges[command_index]
                    {
                        pass.set_pipeline(&self.glyph_pipeline);
                        pass.set_bind_group(0, &batch.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
                        pass.draw(0..6, range.clone());
                    }
                    for range in &path_batch.command_ranges[command_index] {
                        pass.set_pipeline(&self.path_pipeline);
                        pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.path_vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.path_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(range.clone(), 0, 0..1);
                    }
                }
            }
        }
        let frame = if let Some(surface) = &mut self.surface {
            let frame = acquire_frame(&self.device, surface)?;
            let frame_view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mold surface composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&surface.pipeline);
                pass.set_bind_group(0, &surface.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            Some(frame)
        } else {
            None
        };
        self.queue.submit(Some(encoder.finish()));
        if let Some(frame) = frame {
            self.queue.present(frame);
        }
        Ok(())
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    instances: bool,
) -> wgpu::RenderPipeline {
    let attributes = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4,
        7 => Float32x4,
        8 => Float32x4,
        9 => Float32x4,
        10 => Float32x4,
        11 => Float32x4,
        12 => Float32x4
    ];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<SdfQuadInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &attributes,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if instances {
            "mold SDF pipeline"
        } else {
            "mold clear pipeline"
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: if instances { &buffers } else { &[] },
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: if instances {
                    Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                } else {
                    None
                },
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy)]
struct PathVertex {
    position: [f32; 2],
    color: [f32; 4],
}

#[derive(Default)]
struct PathBatch {
    vertices: Vec<PathVertex>,
    indices: Vec<u32>,
    command_ranges: Vec<Vec<Range<u32>>>,
}

fn create_path_pipeline(
    device: &wgpu::Device,
    viewport_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mold path pipeline layout"),
        bind_group_layouts: &[Some(viewport_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mold path shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("path.wgsl").into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<PathVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mold path pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &buffers,
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_path_batch(
    cache: &mut PathCache,
    list: &DrawList,
    scale_120: u32,
) -> Result<PathBatch, String> {
    let mut batch = PathBatch {
        command_ranges: vec![Vec::new(); list.commands.len()],
        ..PathBatch::default()
    };
    let scale = scale_120.max(1) as f32 / 120.0;
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Path {
            bounds,
            path,
            fill_color,
            stroke_color,
            stroke_width,
            even_odd,
            ..
        } = command
        else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let mesh = cache
            .tessellate(path, *stroke_width, *even_odd, scale_120)?
            .clone();
        append_path_mesh(
            &mut batch,
            command_index,
            &mesh.fill,
            *bounds,
            *fill_color,
            scale,
        );
        append_path_mesh(
            &mut batch,
            command_index,
            &mesh.stroke,
            *bounds,
            *stroke_color,
            scale,
        );
    }
    Ok(batch)
}

fn append_path_mesh(
    batch: &mut PathBatch,
    command_index: usize,
    mesh: &crate::path::Mesh,
    bounds: mold_layout::Geometry,
    color: mold_scene::Color,
    scale: f32,
) {
    if mesh.indices.is_empty() || color.alpha <= 0.0 {
        return;
    }
    let vertex_offset = batch.vertices.len() as u32;
    let index_start = batch.indices.len() as u32;
    batch
        .vertices
        .extend(mesh.vertices.iter().map(|position| PathVertex {
            position: [
                (bounds.x as f32 + position[0]) * scale,
                (bounds.y as f32 + position[1]) * scale,
            ],
            color: [color.red, color.green, color.blue, color.alpha],
        }));
    batch
        .indices
        .extend(mesh.indices.iter().map(|index| index + vertex_offset));
    let index_end = batch.indices.len() as u32;
    batch.command_ranges[command_index].push(index_start..index_end);
}

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
    bounds: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
}

struct GlyphBatch {
    instances: Vec<GlyphInstance>,
    command_ranges: Vec<Option<Range<u32>>>,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

fn create_glyph_pipeline(
    device: &wgpu::Device,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mold glyph texture layout"),
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
        label: Some("mold glyph sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mold glyph pipeline layout"),
        bind_group_layouts: &[Some(&texture_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mold glyph shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("glyph.wgsl").into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<GlyphInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &attributes,
    })];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mold glyph pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &buffers,
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, texture_layout, sampler)
}

fn create_glyph_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    create_instance_buffer_for::<GlyphInstance>(device, capacity, "mold glyph instances")
}

fn create_instance_buffer_for<T>(
    device: &wgpu::Device,
    capacity: usize,
    label: &'static str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_vertex_buffer_for<T>(
    device: &wgpu::Device,
    capacity: usize,
    label: &'static str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_index_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mold path indices"),
        size: (capacity * mem::size_of::<u32>()) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[derive(Clone)]
struct TextureImage {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextureKey {
    source: String,
    theme: Option<String>,
    width: u32,
    height: u32,
    scale_120: u32,
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
    logical_width: u32,
    logical_height: u32,
    uv: [f32; 4],
}

type TextureBatchContext<'a> = GlyphBatchContext<'a>;

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
            source,
            icon_theme,
            opacity,
            fill_mode,
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
        let placement = texture_placement(*bounds, intrinsic, *fill_mode);
        let logical_width = placement.logical_width;
        let logical_height = placement.logical_height;
        let key = TextureKey {
            source: source.clone(),
            theme: icon_theme.clone(),
            width: logical_width,
            height: logical_height,
            scale_120,
        };
        if let Some(image) = textures.get(&key) {
            push_texture_instance(
                &mut batch,
                command_index,
                image.clone(),
                placement,
                *opacity,
                context.target_size,
                scale,
            );
            continue;
        }
        let loaded = match icon_theme {
            Some(theme) => {
                cache.load_icon_sized(source, theme, logical_width, logical_height, scale_120)
            }
            None => cache.load(source, logical_width, logical_height, scale_120),
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
            *opacity,
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
) -> TexturePlacement {
    let source_width = f64::from(intrinsic.0.max(1));
    let source_height = f64::from(intrinsic.1.max(1));
    match fill_mode {
        ImageFillMode::Stretch => TexturePlacement {
            bounds,
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

fn push_texture_instance(
    batch: &mut TextureBatch,
    command_index: usize,
    image: TextureImage,
    placement: TexturePlacement,
    opacity: f32,
    target_size: (u32, u32),
    scale: f64,
) {
    let (target_width, target_height) = target_size;
    let bounds = placement.bounds;
    batch.command_instances[command_index] = Some(batch.instances.len() as u32);
    batch.instances.push(GlyphInstance {
        bounds: [
            (bounds.x * scale) as f32 / target_width as f32 * 2.0 - 1.0,
            1.0 - (bounds.y * scale) as f32 / target_height as f32 * 2.0,
            (bounds.width * scale) as f32 / target_width as f32 * 2.0,
            -(bounds.height * scale) as f32 / target_height as f32 * 2.0,
        ],
        uv: placement.uv,
        color: [1.0, 1.0, 1.0, opacity],
    });
    batch.images.push(image);
}

struct GlyphBatchContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    target_size: (u32, u32),
}

fn create_glyph_batch(
    context: GlyphBatchContext<'_>,
    text_system: &mut TextSystem,
    list: &DrawList,
    scale_120: u32,
) -> Option<GlyphBatch> {
    let GlyphBatchContext {
        device,
        queue,
        layout,
        sampler,
        target_size: (target_width, target_height),
    } = context;
    let scale = scale_120.max(1) as f32 / 120.0;
    let mut glyphs = Vec::new();
    let mut command_ranges = vec![None; list.commands.len()];
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Text {
            node,
            bounds,
            text,
            family,
            size,
            color,
            wrap,
            elide,
            horizontal_alignment,
            vertical_alignment,
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
            },
        );
        let spare_height = (bounds.height - measured.height).max(0.0);
        let vertical_offset = match vertical_alignment {
            VerticalAlignment::Top => 0.0,
            VerticalAlignment::Center => spare_height / 2.0,
            VerticalAlignment::Bottom => spare_height,
        };
        let start = glyphs.len() as u32;
        for glyph in text_system.rasterize(
            *node,
            (
                bounds.x as f32 * scale,
                (bounds.y + vertical_offset) as f32 * scale,
            ),
            scale,
        ) {
            if glyph.width > 0 && glyph.height > 0 {
                glyphs.push((glyph, *color));
            }
        }
        let end = glyphs.len() as u32;
        if start != end {
            command_ranges[command_index] = Some(start..end);
        }
    }
    if glyphs.is_empty() {
        return None;
    }

    let widest = glyphs
        .iter()
        .map(|(glyph, _)| glyph.width)
        .max()
        .unwrap_or(1);
    let atlas_width = widest.max(1024).next_power_of_two();
    let mut placements = Vec::with_capacity(glyphs.len());
    let (mut x, mut y, mut row_height) = (0_u32, 0_u32, 0_u32);
    for (glyph, _) in &glyphs {
        if x + glyph.width > atlas_width {
            x = 0;
            y += row_height;
            row_height = 0;
        }
        placements.push((x, y));
        x += glyph.width;
        row_height = row_height.max(glyph.height);
    }
    let atlas_height = (y + row_height).max(1).next_power_of_two();
    let mut pixels = vec![0_u8; atlas_width as usize * atlas_height as usize * 4];
    let mut instances = Vec::with_capacity(glyphs.len());
    for ((glyph, color), (atlas_x, atlas_y)) in glyphs.into_iter().zip(placements) {
        let pixel_count = glyph.width as usize * glyph.height as usize;
        for index in 0..pixel_count {
            let source = match glyph.content {
                RasterContent::Mask if glyph.data.len() >= pixel_count * 3 => {
                    let at = index * 3;
                    [
                        255,
                        255,
                        255,
                        glyph.data[at..at + 3].iter().copied().max().unwrap_or(0),
                    ]
                }
                RasterContent::Mask => [255, 255, 255, glyph.data[index]],
                RasterContent::Color => {
                    let at = index * 4;
                    glyph.data[at..at + 4].try_into().unwrap()
                }
            };
            let source_x = index % glyph.width as usize;
            let source_y = index / glyph.width as usize;
            let destination = ((atlas_y as usize + source_y) * atlas_width as usize
                + atlas_x as usize
                + source_x)
                * 4;
            pixels[destination..destination + 4].copy_from_slice(&source);
        }
        let tint = match glyph.content {
            RasterContent::Mask => [color.red, color.green, color.blue, color.alpha],
            RasterContent::Color => [1.0, 1.0, 1.0, color.alpha],
        };
        instances.push(GlyphInstance {
            bounds: [
                glyph.x as f32 / target_width as f32 * 2.0 - 1.0,
                1.0 - glyph.y as f32 / target_height as f32 * 2.0,
                glyph.width as f32 / target_width as f32 * 2.0,
                -(glyph.height as f32 / target_height as f32 * 2.0),
            ],
            uv: [
                atlas_x as f32 / atlas_width as f32,
                atlas_y as f32 / atlas_height as f32,
                glyph.width as f32 / atlas_width as f32,
                glyph.height as f32 / atlas_height as f32,
            ],
            color: tint,
        });
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mold glyph atlas"),
        size: wgpu::Extent3d {
            width: atlas_width,
            height: atlas_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas_width * 4),
            rows_per_image: Some(atlas_height),
        },
        wgpu::Extent3d {
            width: atlas_width,
            height: atlas_height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mold glyph atlas bind group"),
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
    Some(GlyphBatch {
        instances,
        command_ranges,
        _texture: texture,
        bind_group,
    })
}

fn create_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mold persistent target"),
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

struct SurfaceState {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
}

fn create_surface_state(
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
        label: Some("mold composite texture layout"),
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
        label: Some("mold composite sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = create_composite_bind_group(device, &texture_layout, target_view, &sampler);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mold composite pipeline layout"),
        bind_group_layouts: &[Some(&texture_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mold composite shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mold composite pipeline"),
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
        surface,
        config,
        pipeline,
        texture_layout,
        sampler,
        bind_group,
    })
}

fn create_composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mold composite bind group"),
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

fn acquire_frame(
    device: &wgpu::Device,
    surface: &mut SurfaceState,
) -> Result<wgpu::SurfaceTexture, GpuError> {
    match surface.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => Ok(frame),
        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            surface.surface.configure(device, &surface.config);
            Ok(frame)
        }
        wgpu::CurrentSurfaceTexture::Outdated => {
            surface.surface.configure(device, &surface.config);
            match surface.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(frame),
                status => Err(GpuError(format!(
                    "could not acquire reconfigured GPU surface: {status:?}"
                ))),
            }
        }
        status => Err(GpuError(format!(
            "could not acquire GPU surface: {status:?}"
        ))),
    }
}

fn clamp_scissor(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_is_clamped_to_the_physical_target() {
        assert_eq!(
            clamp_scissor(
                DamageRect {
                    x: 8,
                    y: 9,
                    width: 20,
                    height: 20,
                },
                10,
                12,
            ),
            Some((8, 9, 2, 3))
        );
    }

    #[test]
    fn texture_fit_and_crop_preserve_aspect_ratio() {
        let bounds = Geometry {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 100.0,
        };
        let fit = texture_placement(bounds, (200, 100), ImageFillMode::PreserveAspectFit);
        assert_eq!(
            fit.bounds,
            Geometry {
                x: 10.0,
                y: 45.0,
                width: 100.0,
                height: 50.0
            }
        );
        assert_eq!(fit.uv, [0.0, 0.0, 1.0, 1.0]);

        let crop = texture_placement(bounds, (200, 100), ImageFillMode::PreserveAspectCrop);
        assert_eq!(crop.bounds, bounds);
        assert_eq!(crop.logical_width, 200);
        assert_eq!(crop.logical_height, 100);
        assert_eq!(crop.uv, [0.25, 0.0, 0.5, 1.0]);
    }
}
