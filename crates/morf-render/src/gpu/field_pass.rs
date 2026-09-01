use super::FORMAT;
use crate::{SdfFieldInstance, SdfFieldLayer, SdfFieldMaterial};
use std::mem;

use super::shaders::*;

/// Builds the pipeline that resolves composed distance fields.
///
/// The layers live in a storage buffer rather than in instance attributes: a
/// composition of sixteen layers is far past the vertex-attribute limit, and
/// keeping them in one buffer lets every field in a frame share it.
/// The bind group layouts every field pipeline shares.
///
/// Group zero is the field's own data; group one is a shader's uniforms. The
/// second exists whether or not a pipeline has a shader, so one layout serves
/// both and a shader pipeline is not a different kind of thing.
pub(crate) fn create_field_layouts(
    device: &wgpu::Device,
) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("morf field layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // One material per instance: the gradient, border, shadow and
            // overlay that used to belong to the quad pipeline alone.
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shader_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("morf field shader layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    (layout, shader_layout)
}

/// Builds one field pipeline, optionally with a configuration's shader in it.
///
/// `None` back means the generated WGSL did not compile, which is a bug in the
/// compiler rather than in the configuration — the configuration's own mistakes
/// were caught and reported before anything reached here.
pub(crate) fn build_field_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    shader_layout: &wgpu::BindGroupLayout,
    user: Option<&str>,
    owns_coverage: bool,
) -> Option<wgpu::RenderPipeline> {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("morf field pipeline layout"),
        bind_group_layouts: &[Some(layout), Some(shader_layout)],
        immediate_size: 0,
    });
    let source = field_shader_source(include_str!("../field.wgsl"), user, owns_coverage)?;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("morf field shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let attributes = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x4
    ];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: mem::size_of::<SdfFieldInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &attributes,
    })];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("morf field pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &buffers,
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
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
    Some(pipeline)
}

pub(crate) fn create_field_layer_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("morf field layers"),
        size: (capacity.max(1) * mem::size_of::<SdfFieldLayer>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn create_field_material_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("morf field materials"),
        size: (capacity.max(1) * mem::size_of::<SdfFieldMaterial>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn create_field_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    viewport: &wgpu::Buffer,
    layers: &wgpu::Buffer,
    materials: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("morf field bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: layers.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: materials.as_entire_binding(),
            },
        ],
    })
}

/// The uniform buffer one shader's parameters live in.
pub(crate) fn create_shader_uniform_buffer(device: &wgpu::Device, size: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("morf shader uniforms"),
        // A uniform binding has a minimum size whatever the shader declared, so
        // a parameterless shader still gets a block rather than a zero-length
        // buffer wgpu will refuse to bind.
        size: u64::from(size.max(16)),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn create_shader_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("morf shader bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    })
}
