use crate::SdfFieldInstance;
use morf_image::ImageCache;
use morf_text::{RasterContent, TextSystem};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

use super::{
    backend_types::*, clear_pipeline::*, field_pass::*, glyphs::*, pipelines::*, shaders::*,
    targets::*,
};

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
                label: Some("morf device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter_limits,
                ..Default::default()
            })
            .await
            .map_err(|error| GpuError(format!("could not create GPU device: {error}")))?;
        let viewport_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("morf viewport layout"),
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
            label: Some("morf viewport"),
            contents: bytemuck::cast_slice(&viewport),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let viewport_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("morf viewport bind group"),
            layout: &viewport_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport_buffer.as_entire_binding(),
            }],
        });
        let clear_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("morf damage clear shader"),
            source: wgpu::ShaderSource::Wgsl(
                fullscreen_source(include_str!("../clear.wgsl")).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("morf clear pipeline layout"),
            bind_group_layouts: &[Some(&viewport_layout)],
            immediate_size: 0,
        });
        let clear_pipeline = create_clear_pipeline(&device, &pipeline_layout, &clear_shader);
        let (glyph_pipeline, glyph_layout, glyph_sampler) = create_glyph_pipeline(&device);
        let glyph_mask_atlas =
            GlyphAtlas::new(&device, &glyph_layout, &glyph_sampler, RasterContent::Mask);
        let glyph_color_atlas =
            GlyphAtlas::new(&device, &glyph_layout, &glyph_sampler, RasterContent::Color);
        let (blur_pipeline, blur_layout, blur_sampler) = create_blur_pipeline(&device);
        let glyph_capacity = 1;
        let glyph_buffer = create_glyph_buffer(&device, glyph_capacity);
        let texture_capacity = 1;
        let texture_buffer = create_instance_buffer_for::<GlyphInstance>(
            &device,
            texture_capacity,
            "morf texture instances",
        );
        let (field_layout, field_shader_layout) = create_field_layouts(&device);
        let field_pipeline = build_field_pipeline(
            &device,
            FieldPipeline {
                layout: &field_layout,
                shader_layout: &field_shader_layout,
                user: None,
                owns_coverage: false,
                vertex: None,
                textures: None,
                data: None,
            },
        )
        .expect("the field shader carries its own hook");
        let field_shader_default = create_shader_bind_group(
            &device,
            &field_shader_layout,
            &create_shader_uniform_buffer(&device, morf_shader::HEADER_BYTES),
        );
        let field_capacity = 1;
        let field_buffer = create_instance_buffer_for::<SdfFieldInstance>(
            &device,
            field_capacity,
            "morf field instances",
        );
        let field_layer_capacity = 1;
        let field_layer_buffer = create_field_layer_buffer(&device, field_layer_capacity);
        let field_material_capacity = 1;
        let field_material_buffer = create_field_material_buffer(&device, field_material_capacity);
        let field_outline_capacity = 1;
        let field_outline_buffer = create_field_outline_buffer(&device, field_outline_capacity);
        let field_bind_group = create_field_bind_group(
            &device,
            &field_layout,
            &viewport_buffer,
            &field_layer_buffer,
            &field_material_buffer,
            &field_outline_buffer,
        );
        let (texture, view) = create_target(&device, width, height);
        let surface = surface
            .map(|surface| create_surface_state(&device, &adapter, surface, &view, width, height))
            .transpose()?;

        Ok(Self {
            device,
            queue,
            clear_pipeline,
            viewport_buffer,
            viewport_bind_group,
            glyph_pipeline,
            glyph_layout,
            glyph_sampler,
            glyph_mask_atlas,
            glyph_color_atlas,
            blur_pipeline,
            blur_layout,
            blur_sampler,
            glyph_buffer,
            glyph_capacity,
            texture_buffer,
            texture_capacity,
            field_pipeline,
            field_layout,
            field_buffer,
            field_capacity,
            field_layer_buffer,
            field_layer_capacity,
            field_material_buffer,
            field_material_capacity,
            field_outline_capacity,
            field_outline_buffer,
            field_bind_group,
            field_shader_layout,
            field_shader_default,
            shaders: HashMap::new(),
            effect_shaders: HashMap::new(),
            elapsed: 0.0,
            images: ImageCache::default(),
            image_textures: HashMap::new(),
            layer_target_pool: Vec::new(),
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
    pub(crate) fn resize_target(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        (self.texture, self.view) = create_target(&self.device, self.width, self.height);
        // The pooled layer targets are surface-sized, so a resize retires them.
        self.layer_target_pool.clear();
        let viewport = [self.width as f32, self.height as f32, self.elapsed, 0.0];
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

    /// Registers a compiled shader, building its pipeline.
    ///
    /// Called when a configuration loads, never while rendering: compiling a
    /// pipeline costs tens of milliseconds, and a compositor cannot spend that
    /// at paint time. Registering the same program twice is a no-op, so a
    /// configuration that attaches one shader to fifty nodes builds one
    /// pipeline.
    /// Advances the clock shaders read.
    ///
    /// Called once per frame by the host, which owns the frame clock; the
    /// backend only needs the number a shader will see.
    pub fn set_elapsed(&mut self, seconds: f32) {
        self.elapsed = seconds;
    }

    /// Whether a program has been registered, in either registry.
    pub fn has_shader(&self, program: u64) -> bool {
        self.shaders.contains_key(&program) || self.effect_shaders.contains_key(&program)
    }

    /// Grows the field instance, layer, material and outline buffers, rebinding
    /// whenever one of the storage buffers moves.
    pub(crate) fn ensure_fields(
        &mut self,
        instances: usize,
        layers: usize,
        materials: usize,
        outlines: usize,
    ) {
        if instances > self.field_capacity {
            self.field_capacity = instances.next_power_of_two();
            self.field_buffer = create_instance_buffer_for::<SdfFieldInstance>(
                &self.device,
                self.field_capacity,
                "morf field instances",
            );
        }
        let mut rebind = false;
        if layers > self.field_layer_capacity {
            self.field_layer_capacity = layers.next_power_of_two();
            self.field_layer_buffer =
                create_field_layer_buffer(&self.device, self.field_layer_capacity);
            rebind = true;
        }
        if materials > self.field_material_capacity {
            self.field_material_capacity = materials.next_power_of_two();
            self.field_material_buffer =
                create_field_material_buffer(&self.device, self.field_material_capacity);
            rebind = true;
        }
        if outlines > self.field_outline_capacity {
            self.field_outline_capacity = outlines.next_power_of_two();
            self.field_outline_buffer =
                create_field_outline_buffer(&self.device, self.field_outline_capacity);
            rebind = true;
        }
        if rebind {
            // The bind group holds the old buffers, so it has to be rebuilt
            // whenever either storage grows or the shader reads freed memory.
            self.field_bind_group = create_field_bind_group(
                &self.device,
                &self.field_layout,
                &self.viewport_buffer,
                &self.field_layer_buffer,
                &self.field_material_buffer,
                &self.field_outline_buffer,
            );
        }
    }

    pub(crate) fn ensure_glyphs(&mut self, required: usize) {
        if required <= self.glyph_capacity {
            return;
        }
        self.glyph_capacity = required.next_power_of_two();
        self.glyph_buffer = create_glyph_buffer(&self.device, self.glyph_capacity);
    }

    pub(crate) fn ensure_textures(&mut self, required: usize) {
        if required <= self.texture_capacity {
            return;
        }
        self.texture_capacity = required.next_power_of_two();
        self.texture_buffer = create_instance_buffer_for::<GlyphInstance>(
            &self.device,
            self.texture_capacity,
            "morf texture instances",
        );
    }
}

impl WgpuBackend {
    /// A full-surface render target for one offscreen layer, reused each frame.
    ///
    /// The handles are reference counted, so the clone is a pointer bump rather
    /// than an allocation; the pool grows to the deepest layer stack a frame has
    /// needed and is emptied only by a resize.
    pub(crate) fn layer_target(&mut self, index: usize) -> (wgpu::Texture, wgpu::TextureView) {
        while self.layer_target_pool.len() <= index {
            self.layer_target_pool
                .push(create_target(&self.device, self.width, self.height));
        }
        self.layer_target_pool[index].clone()
    }
}
