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
            source: wgpu::ShaderSource::Wgsl(include_str!("../sdf.wgsl").into()),
        });
        let clear_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mold damage clear shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../clear.wgsl").into()),
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
        let glyph_mask_atlas =
            GlyphAtlas::new(&device, &glyph_layout, &glyph_sampler, RasterContent::Mask);
        let glyph_color_atlas =
            GlyphAtlas::new(&device, &glyph_layout, &glyph_sampler, RasterContent::Color);
        let (blur_pipeline, blur_layout, blur_sampler) = create_blur_pipeline(&device);
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
        let (field_pipeline, field_layout) = create_field_pipeline(&device);
        let field_capacity = 1;
        let field_buffer = create_instance_buffer_for::<SdfFieldInstance>(
            &device,
            field_capacity,
            "mold field instances",
        );
        let field_layer_capacity = 1;
        let field_layer_buffer = create_field_layer_buffer(&device, field_layer_capacity);
        let field_bind_group = create_field_bind_group(
            &device,
            &field_layout,
            &viewport_buffer,
            &field_layer_buffer,
        );
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
            glyph_mask_atlas,
            glyph_color_atlas,
            blur_pipeline,
            blur_layout,
            blur_sampler,
            glyph_buffer,
            glyph_capacity,
            texture_buffer,
            texture_capacity,
            path_pipeline,
            path_vertex_buffer,
            path_vertex_capacity,
            path_index_buffer,
            path_index_capacity,
            field_pipeline,
            field_layout,
            field_buffer,
            field_capacity,
            field_layer_buffer,
            field_layer_capacity,
            field_bind_group,
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

    /// Grows the field instance and layer buffers, rebinding when either moves.
    fn ensure_fields(&mut self, instances: usize, layers: usize) {
        if instances > self.field_capacity {
            self.field_capacity = instances.next_power_of_two();
            self.field_buffer = create_instance_buffer_for::<SdfFieldInstance>(
                &self.device,
                self.field_capacity,
                "mold field instances",
            );
        }
        if layers > self.field_layer_capacity {
            self.field_layer_capacity = layers.next_power_of_two();
            self.field_layer_buffer =
                create_field_layer_buffer(&self.device, self.field_layer_capacity);
            // The bind group holds the old buffer, so it has to be rebuilt
            // whenever the storage grows or the shader reads freed memory.
            self.field_bind_group = create_field_bind_group(
                &self.device,
                &self.field_layout,
                &self.viewport_buffer,
                &self.field_layer_buffer,
            );
        }
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
