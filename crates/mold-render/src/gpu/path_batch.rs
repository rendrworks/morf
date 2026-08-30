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
        12 => Float32x4,
        13 => Float32x4,
        14 => Float32x4,
        15 => Float32x2
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
    /// Coverage position: solid inside the shape, ramping across the edge band.
    coverage: f32,
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
        source: wgpu::ShaderSource::Wgsl(include_str!("../path.wgsl").into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32];
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
    let morph_nodes = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::Path {
                node,
                morph: Some(_),
                ..
            } => Some(*node),
            _ => None,
        })
        .collect::<HashSet<_>>();
    cache.retain_morphs(&morph_nodes);
    let mut batch = PathBatch {
        command_ranges: vec![Vec::new(); list.commands.len()],
        ..PathBatch::default()
    };
    let scale = scale_120.max(1) as f32 / 120.0;
    for (command_index, command) in list.commands.iter().enumerate() {
        let DrawCommand::Path {
            node,
            bounds,
            transform,
            path,
            morph,
            fill_color,
            stroke_color,
            stroke_width,
            even_odd,
            ..
        } = command
        else {
            continue;
        };
        if path.is_empty() && morph.is_none() {
            continue;
        }
        let transform_scale = transform.matrix[0].hypot(transform.matrix[1]);
        let tessellation_scale = (f64::from(scale_120) * transform_scale)
            .ceil()
            .clamp(1.0, f64::from(u32::MAX)) as u32;
        let mesh = if let Some(morph) = morph {
            cache.tessellate_morph(
                *node,
                morph,
                *bounds,
                *stroke_width,
                *even_odd,
                tessellation_scale,
            )?
        } else {
            cache.tessellate(path, *stroke_width, *even_odd, tessellation_scale)?
        }
        .clone();
        append_path_mesh(
            &mut batch,
            command_index,
            &mesh.fill,
            *bounds,
            *transform,
            *fill_color,
            scale,
        );
        append_path_mesh(
            &mut batch,
            command_index,
            &mesh.stroke,
            *bounds,
            *transform,
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
    transform: mold_layout::Transform2D,
    color: mold_scene::Color,
    scale: f32,
) {
    if mesh.indices.is_empty() || color.alpha <= 0.0 {
        return;
    }
    let vertex_offset = batch.vertices.len() as u32;
    let index_start = batch.indices.len() as u32;
    batch.vertices.extend(mesh.vertices.iter().map(|position| {
        let point = transform.point(
            bounds.x + f64::from(position[0]),
            bounds.y + f64::from(position[1]),
        );
        PathVertex {
            position: [(point.0 as f32) * scale, (point.1 as f32) * scale],
            color: color_array(color),
            coverage: position[2],
        }
    }));
    batch
        .indices
        .extend(mesh.indices.iter().map(|index| index + vertex_offset));
    let index_end = batch.indices.len() as u32;
    batch.command_ranges[command_index].push(index_start..index_end);
}
