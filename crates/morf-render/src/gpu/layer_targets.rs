use super::{backend_types::*, glyphs::*, targets::*, textures::*};
use crate::DrawList;
use crate::effects::color_array;
use morf_layout::{Geometry, Transform2D};

impl WgpuBackend {
    /// Renders every layer's subtree into its own target and prepares the quad
    /// that composites it back.
    ///
    /// Split out of `render` because it is self-contained and long: a layer's
    /// blur chain, its shadow chain, its mask and the instance that draws it
    /// are all decided here, and none of it interacts with the command loop
    /// that follows.
    pub(crate) fn build_layer_targets(
        &mut self,
        list: &DrawList,
        texture_batch: &mut TextureBatch,
        scale: f64,
    ) -> Vec<LayerTarget> {
        let mut layer_targets = Vec::with_capacity(list.layers.len());
        for (layer_index, layer) in list.layers.iter().enumerate() {
            let (texture, view) = self.layer_target(layer_index);
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
                label: Some("morf layer bind group"),
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
                    label: Some("morf layer shadow bind group"),
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
                    ..GlyphInstance::default()
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
                ..GlyphInstance::default()
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
        layer_targets
    }
}
