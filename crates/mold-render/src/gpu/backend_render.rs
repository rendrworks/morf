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
                queue: &self.queue,
                mask_atlas: &mut self.glyph_mask_atlas,
                color_atlas: &mut self.glyph_color_atlas,
                target_size: (self.width, self.height),
            },
            &mut self.text,
            list,
            scale_120,
        )?;
        let mut texture_batch = create_texture_batch(
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
        let scale = scale_120.max(1) as f64 / 120.0;
        let mut layer_targets = Vec::with_capacity(list.layers.len());
        for layer in &list.layers {
            let (texture, view) = create_target(&self.device, self.width, self.height);
            let blur = (layer.blur > 0.0).then(|| {
                create_blur_chain(
                    &self.device,
                    &self.blur_layout,
                    &self.blur_sampler,
                    &view,
                    self.width,
                    self.height,
                    (layer.blur * scale as f32 / 4.0).max(0.5),
                )
            });
            let shadow = (layer.shadow_color.alpha > 0.0 && layer.shadow_blur > 0.0).then(|| {
                create_blur_chain(
                    &self.device,
                    &self.blur_layout,
                    &self.blur_sampler,
                    &view,
                    self.width,
                    self.height,
                    (layer.shadow_blur * scale as f32 / 4.0).max(0.5),
                )
            });
            let composite_view = blur.as_ref().map_or(&view, |chain| &chain.views[3]);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mold layer bind group"),
                layout: &self.glyph_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(composite_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.glyph_sampler),
                    },
                ],
            });
            let shadow_bind_group = (layer.shadow_color.alpha > 0.0).then(|| {
                let shadow_view = shadow.as_ref().map_or(&view, |chain| &chain.views[3]);
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mold layer shadow bind group"),
                    layout: &self.glyph_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(shadow_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.glyph_sampler),
                        },
                    ],
                })
            });
            let shadow_instance = shadow_bind_group.as_ref().map(|_| {
                let instance = texture_batch.instances.len() as u32;
                let (origin, axes) = transformed_quad(
                    Transform2D::IDENTITY,
                    Geometry {
                        x: layer.shadow_offset[0] as f64,
                        y: layer.shadow_offset[1] as f64,
                        width: self.width as f64 / scale,
                        height: self.height as f64 / scale,
                    },
                    scale,
                    (self.width, self.height),
                );
                texture_batch.instances.push(GlyphInstance {
                    origin,
                    axes,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    color: [1.0, 1.0, 1.0, layer.shadow_color.alpha * layer.opacity],
                    color_overlay: {
                        let mut color = color_array(layer.shadow_color);
                        color[3] = 1.0;
                        color
                    },
                    mode: [0.0; 4],
                    surface: [0.0; 4],
                    mask_bounds: [0.0; 4],
                    mask_inverse_0: [0.0; 4],
                    mask_inverse_1: [0.0; 4],
                    mask_radii: [0.0; 4],
                });
                instance
            });
            let instance = texture_batch.instances.len() as u32;
            let (mask_enabled, mask_bounds, mask_inverse_0, mask_inverse_1, mask_radii) =
                layer_mask_data(layer.mask);
            let (origin, axes) = transformed_quad(
                Transform2D::IDENTITY,
                Geometry {
                    x: 0.0,
                    y: 0.0,
                    width: self.width as f64 / scale,
                    height: self.height as f64 / scale,
                },
                scale,
                (self.width, self.height),
            );
            texture_batch.instances.push(GlyphInstance {
                origin,
                axes,
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [1.0, 1.0, 1.0, layer.opacity],
                color_overlay: [0.0; 4],
                mode: [1.0, mask_enabled, 0.0, 0.0],
                surface: [
                    0.0,
                    0.0,
                    self.width as f32 / scale as f32,
                    self.height as f32 / scale as f32,
                ],
                mask_bounds,
                mask_inverse_0,
                mask_inverse_1,
                mask_radii,
            });
            layer_targets.push(LayerTarget {
                _texture: texture,
                view,
                bind_group,
                instance,
                blur,
                shadow_bind_group,
                shadow_instance,
                shadow,
            });
        }
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
        let mut command_layers = vec![None; list.commands.len()];
        let mut child_layers = HashMap::new();
        for (layer_index, layer) in list.layers.iter().enumerate() {
            for owner in &mut command_layers[layer.commands.clone()] {
                *owner = Some(layer_index);
            }
            child_layers.insert((layer.parent, layer.commands.start), layer_index);
        }
        macro_rules! draw_command {
            ($pass:expr, $command_index:expr, $base_damage:expr) => {{
                let command_index = $command_index;
                let command_damage = if let Some(clip) = list.commands[command_index].clip() {
                    physical_damage(clip, scale_120)
                        .and_then(|clip| intersect_damage($base_damage, clip))
                } else {
                    Some($base_damage)
                };
                if let Some(command_damage) = command_damage
                    && let Some((x, y, width, height)) =
                        clamp_scissor(command_damage, self.width, self.height)
                {
                    $pass.set_scissor_rect(x, y, width, height);
                    if let Some(instance) = quad_indices[command_index] {
                        $pass.set_pipeline(&self.pipeline);
                        $pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                        $pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
                        $pass.draw(0..6, instance..instance + 1);
                    }
                    if let Some(instance) = texture_batch.command_instances[command_index] {
                        let image = &texture_batch.images[instance as usize];
                        $pass.set_pipeline(&self.glyph_pipeline);
                        $pass.set_bind_group(0, &image.bind_group, &[]);
                        $pass.set_vertex_buffer(0, self.texture_buffer.slice(..));
                        $pass.draw(0..6, instance..instance + 1);
                    }
                    if let Some(batch) = &glyph_batch {
                        for span in &batch.command_spans[command_index] {
                            $pass.set_pipeline(&self.glyph_pipeline);
                            let atlas = if span.color {
                                &self.glyph_color_atlas
                            } else {
                                &self.glyph_mask_atlas
                            };
                            $pass.set_bind_group(0, &atlas.bind_group, &[]);
                            $pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
                            $pass.draw(0..6, span.range.clone());
                        }
                    }
                    for range in &path_batch.command_ranges[command_index] {
                        $pass.set_pipeline(&self.path_pipeline);
                        $pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                        $pass.set_vertex_buffer(0, self.path_vertex_buffer.slice(..));
                        $pass.set_index_buffer(
                            self.path_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        $pass.draw_indexed(range.clone(), 0, 0..1);
                    }
                }
            }};
        }
        macro_rules! draw_layer {
            ($pass:expr, $layer_index:expr, $base_damage:expr) => {{
                let layer_index = $layer_index;
                if let Some(layer_damage) =
                    physical_damage(list.layers[layer_index].bounds, scale_120)
                        .and_then(|bounds| intersect_damage($base_damage, bounds))
                    && let Some((x, y, width, height)) =
                        clamp_scissor(layer_damage, self.width, self.height)
                {
                    let target = &layer_targets[layer_index];
                    $pass.set_scissor_rect(x, y, width, height);
                    $pass.set_pipeline(&self.glyph_pipeline);
                    if let (Some(bind_group), Some(instance)) =
                        (&target.shadow_bind_group, target.shadow_instance)
                    {
                        $pass.set_bind_group(0, bind_group, &[]);
                        $pass.set_vertex_buffer(0, self.texture_buffer.slice(..));
                        $pass.draw(0..6, instance..instance + 1);
                    }
                    $pass.set_bind_group(0, &target.bind_group, &[]);
                    $pass.set_vertex_buffer(0, self.texture_buffer.slice(..));
                    $pass.draw(0..6, target.instance..target.instance + 1);
                }
            }};
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mold frame encoder"),
            });
        let full_damage = DamageRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        };
        for layer_index in (0..list.layers.len()).rev() {
            let layer = &list.layers[layer_index];
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mold subtree layer"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &layer_targets[layer_index].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                let mut command_index = layer.commands.start;
                while command_index < layer.commands.end {
                    if let Some(child) = child_layers
                        .get(&(Some(layer_index), command_index))
                        .copied()
                    {
                        draw_layer!(pass, child, full_damage);
                        command_index = list.layers[child].commands.end.max(command_index + 1);
                    } else {
                        if command_layers[command_index] == Some(layer_index) {
                            draw_command!(pass, command_index, full_damage);
                        }
                        command_index += 1;
                    }
                }
            }
            let target = &layer_targets[layer_index];
            for blur in [target.blur.as_ref(), target.shadow.as_ref()]
                .into_iter()
                .flatten()
            {
                for (pass_index, blur_pass) in blur.passes.iter().enumerate() {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("mold dual-kawase pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &blur.views[pass_index],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    pass.set_pipeline(&self.blur_pipeline);
                    pass.set_bind_group(0, &blur_pass.bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }
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
                let mut command_index = 0;
                while command_index < list.commands.len() {
                    if let Some(layer) = child_layers.get(&(None, command_index)).copied() {
                        draw_layer!(pass, layer, *damage);
                        command_index = list.layers[layer].commands.end.max(command_index + 1);
                    } else {
                        if command_layers[command_index].is_none() {
                            draw_command!(pass, command_index, *damage);
                        }
                        command_index += 1;
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

