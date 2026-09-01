/// The one surface format every target and pipeline agrees on.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

mod backend_init;
mod backend_render;
mod backend_types;
mod batches;
mod clear_pipeline;
mod field_pass;
mod glyph_batch;
mod glyphs;
mod layer_targets;
mod pipelines;
mod shader_registry;
mod shaders;
mod targets;
mod textures;

pub use backend_types::*;
#[cfg(test)]
mod field_agreement_tests;
#[cfg(test)]
mod field_color_tests;
#[cfg(test)]
mod field_shape_tests;
#[cfg(test)]
mod field_tests;
#[cfg(test)]
mod shader_host_tests;
#[cfg(test)]
mod shader_language_tests;
#[cfg(test)]
mod shader_mode_tests;
#[cfg(test)]
mod shader_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_field_tests;
