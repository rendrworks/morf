//! Registering a configuration's shaders: pipelines, textures and data blocks.
//!
//! All of it happens when the configuration loads and none of it during a
//! frame. Building a pipeline costs tens of milliseconds and decoding an image
//! costs more, and a compositor has neither to spend at paint time.

use super::backend_types::*;
use super::field_pass::*;
use super::pipelines::build_glyph_pipeline;

impl WgpuBackend {
    pub fn register_shader(&mut self, shader: ShaderRegistration<'_>) -> Result<(), GpuError> {
        let registry = if shader.effect {
            &self.effect_shaders
        } else {
            &self.shaders
        };
        if registry.contains_key(&shader.program) {
            return Ok(());
        }
        // A shader's own textures and data blocks, bound once here rather than
        // per frame: an image is decoded and uploaded when the configuration
        // says so, not while a frame is being drawn.
        let textures = self.build_shader_textures(shader.textures)?;
        let data = self.build_shader_data(shader.data);
        let pipeline = if shader.effect {
            build_glyph_pipeline(
                &self.device,
                &self.glyph_layout,
                Some(&self.field_shader_layout),
                shader.wgsl,
            )
            .ok_or_else(|| {
                GpuError("the glyph shader has no hook to splice an effect into".to_owned())
            })?
        } else {
            build_field_pipeline(
                &self.device,
                FieldPipeline {
                    layout: &self.field_layout,
                    shader_layout: &self.field_shader_layout,
                    user: shader.wgsl,
                    owns_coverage: shader.owns_coverage,
                    vertex: shader.vertex,
                    textures: textures.as_ref().map(|(_, layout)| layout),
                    data: data.as_ref().map(|(_, _, layout)| layout),
                },
            )
            .ok_or_else(|| {
                GpuError("the field shader has no hook to splice a shader into".to_owned())
            })?
        };
        let uniforms = create_shader_uniform_buffer(&self.device, shader.uniform_size);
        let bind_group =
            create_shader_bind_group(&self.device, &self.field_shader_layout, &uniforms);
        let registry = if shader.effect {
            &mut self.effect_shaders
        } else {
            &mut self.shaders
        };
        registry.insert(
            shader.program,
            ShaderProgram {
                pipeline,
                uniforms,
                bind_group,
                offsets: shader.offsets.to_vec(),
                size: shader.uniform_size,
                textures: textures.map(|(group, _)| group),
                data: data.map(|(buffers, group, _)| (buffers, group)),
            },
        );
        Ok(())
    }

    /// Decodes and uploads a shader's declared textures.
    ///
    /// `None` when it declared none, which is the common case and costs nothing
    /// — an empty bind group would still be a group to create and bind.
    fn build_shader_textures(
        &mut self,
        paths: &[String],
    ) -> Result<Option<(wgpu::BindGroup, wgpu::BindGroupLayout)>, GpuError> {
        if paths.is_empty() {
            return Ok(None);
        }
        let mut entries = Vec::with_capacity(paths.len() * 2);
        for slot in 0..paths.len() as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: slot * 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: slot * 2 + 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("morf shader textures"),
                entries: &entries,
            });
        let mut views = Vec::with_capacity(paths.len());
        for path in paths {
            let image = self
                .images
                .load(path, 0, 0, 120)
                .map_err(|error| GpuError(format!("shader texture `{path}`: {error}")))?;
            views.push(self.upload_shader_texture(&image));
        }
        let mut bindings = Vec::with_capacity(views.len() * 2);
        for (slot, view) in views.iter().enumerate() {
            bindings.push(wgpu::BindGroupEntry {
                binding: slot as u32 * 2,
                resource: wgpu::BindingResource::TextureView(view),
            });
            bindings.push(wgpu::BindGroupEntry {
                binding: slot as u32 * 2 + 1,
                resource: wgpu::BindingResource::Sampler(&self.glyph_sampler),
            });
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("morf shader textures"),
            layout: &layout,
            entries: &bindings,
        });
        Ok(Some((group, layout)))
    }

    /// Creates the storage buffers a shader's data blocks are read from.
    fn build_shader_data(
        &self,
        blocks: &[(String, u32)],
    ) -> Option<(Vec<wgpu::Buffer>, wgpu::BindGroup, wgpu::BindGroupLayout)> {
        if blocks.is_empty() {
            return None;
        }
        let entries: Vec<_> = (0..blocks.len() as u32)
            .map(|slot| wgpu::BindGroupLayoutEntry {
                binding: slot,
                visibility: wgpu::ShaderStages::FRAGMENT,
                // Read-only on purpose: every pixel of a node runs this shader,
                // so a writable block would be a race between all of them.
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("morf shader data"),
                entries: &entries,
            });
        let buffers: Vec<_> = blocks
            .iter()
            .map(|(name, length)| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(name),
                    size: u64::from(*length).max(1) * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let bindings: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(slot, buffer)| wgpu::BindGroupEntry {
                binding: slot as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("morf shader data"),
            layout: &layout,
            entries: &bindings,
        });
        Some((buffers, group, layout))
    }
}
