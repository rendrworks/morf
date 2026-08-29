use std::error::Error as StdError;
use std::fmt;
use std::mem;

use wgpu::util::DeviceExt;

use crate::{DamageRect, DrawList, RenderBackend, SdfQuadInstance};

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
    texture: wgpu::Texture,
    view: wgpu::TextureView,
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
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| GpuError(format!("no compatible GPU adapter: {error}")))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mold device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
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
        let instance_capacity = 1;
        let instance_buffer = create_instance_buffer(&device, instance_capacity);
        let (texture, view) = create_target(&device, width, height);

        Ok(Self {
            device,
            queue,
            pipeline,
            clear_pipeline,
            viewport_buffer,
            viewport_bind_group,
            instance_buffer,
            instance_capacity,
            texture,
            view,
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
    }

    /// Returns the persistent target for copying or diagnostics.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    fn ensure_instances(&mut self, required: usize) {
        if required <= self.instance_capacity {
            return;
        }
        self.instance_capacity = required.next_power_of_two();
        self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
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
        let instances: Vec<_> = list
            .commands
            .iter()
            .filter_map(|command| SdfQuadInstance::from_command(command, scale_120))
            .collect();
        self.ensure_instances(instances.len().max(1));
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
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
            pass.set_bind_group(0, &self.viewport_bind_group, &[]);
            for damage in damage {
                let Some((x, y, width, height)) = clamp_scissor(*damage, self.width, self.height)
                else {
                    continue;
                };
                pass.set_scissor_rect(x, y, width, height);
                pass.set_pipeline(&self.clear_pipeline);
                pass.draw(0..3, 0..1);
                if !instances.is_empty() {
                    pass.set_pipeline(&self.pipeline);
                    pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                    pass.draw(0..6, 0..instances.len() as u32);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
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
        4 => Float32x4
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

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mold SDF instances"),
        size: (capacity * mem::size_of::<SdfQuadInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
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
}
