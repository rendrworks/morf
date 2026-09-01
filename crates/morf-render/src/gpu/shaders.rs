use morf_scene::NodeHandle;

use crate::ShaderBinding;

use super::backend_types::*;

// Shader sources, with the shared distance functions in front of them.

/// Builds a shader source with `shape.wgsl` prepended.
///
/// WGSL has no `#include`, so a function every shader needs has to be pasted
/// into every shader — either by hand, which is how three copies of the rounded
/// box drifted apart, or here, once, at the point the module is created.
pub(crate) fn shader_source(body: &str) -> String {
    let mut source = String::with_capacity(SHARED_SHAPES.len() + body.len() + 1);
    source.push_str(SHARED_SHAPES);
    source.push('\n');
    source.push_str(body);
    source
}

pub(crate) const SHARED_SHAPES: &str = include_str!("../shape.wgsl");

/// The default hook body, replaced when a configuration attaches a shader.
const SHADER_HOOK: &str = "    return base;";

/// Builds the field shader with a configuration's own shader spliced in.
///
/// The generated module is appended and the hook's body is replaced with a call
/// into it. A distinct pipeline per shader rather than a branch on a uniform:
/// WGSL cannot swap a function at run time, and a branch would make every node
/// without a shader pay for the ones that have one.
///
/// Returns `None` when the hook is missing, which can only mean `field.wgsl`
/// was edited without this — better a clear failure at startup than a shader
/// that silently never runs.
pub(crate) fn field_shader_source(body: &str, shader: Option<&str>) -> Option<String> {
    let base = shader_source(body);
    let Some(shader) = shader else {
        return Some(base);
    };
    if !base.contains(SHADER_HOOK) {
        return None;
    }
    let hooked = base.replacen(
        SHADER_HOOK,
        "    return morf_shader_main(uv, local, coverage, base);",
        1,
    );
    Some(format!("{shader}\n{hooked}"))
}

impl WgpuBackend {
    /// Releases everything this backend holds for nodes that no longer exist.
    ///
    /// A shaped text buffer is not small — cosmic-text keeps the runs and the
    /// per-glyph vectors, and the cache holds one per Text node ever measured.
    /// Nothing in the scene can reach in here to drop one, so without this call
    /// every view switch, Loader swap and list recycle leaks one per node for
    /// the life of the process.
    pub fn forget_nodes(&mut self, nodes: &[NodeHandle]) {
        for node in nodes {
            self.text.remove(*node);
        }
    }
}

/// Builds a shader source for a pass that covers the whole target.
///
/// These want the full-screen triangle rather than the shape functions, and
/// define their own `VertexOutput`, so they take a different prelude.
pub(crate) fn fullscreen_source(body: &str) -> String {
    let mut source = String::with_capacity(FULLSCREEN.len() + body.len() + 1);
    source.push_str(FULLSCREEN);
    source.push('\n');
    source.push_str(body);
    source
}

pub(crate) const FULLSCREEN: &str = include_str!("../fullscreen.wgsl");

impl WgpuBackend {
    /// Writes each shader's uniform block for this frame.
    ///
    /// The frame's own values — the clock and the surface size — sit at a fixed
    /// offset the compiler reserved, so writing them does not depend on what a
    /// particular shader declared afterwards. Parameters go at the offsets the
    /// compiler computed, which is the same computation the generated WGSL was
    /// built from, so the two cannot drift.
    ///
    /// One write per distinct program rather than per node: several nodes
    /// sharing a shader share its uniforms, which is right for the clock and
    /// wrong for per-node parameters — the last node's values win. Making them
    /// per-node needs a dynamic offset into one buffer, which is the next step
    /// and not this one.
    pub(crate) fn write_shader_uniforms(
        &mut self,
        bindings: &[Option<ShaderBinding>],
        scale_120: u32,
    ) {
        if self.shaders.is_empty() {
            return;
        }
        let scale = scale_120.max(1) as f32 / 120.0;
        let header = [
            self.width as f32 / scale,
            self.height as f32 / scale,
            self.elapsed,
            0.0,
        ];
        for binding in bindings.iter().flatten() {
            let Some(program) = self.shaders.get(&binding.program) else {
                continue;
            };
            let mut block = vec![0u8; program.size as usize];
            block[..16].copy_from_slice(bytemuck::cast_slice(&header));
            for (index, offset) in program.offsets.iter().enumerate() {
                let Some(value) = binding.params.get(index) else {
                    break;
                };
                let start = *offset as usize;
                if start + 4 <= block.len() {
                    block[start..start + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            self.queue.write_buffer(&program.uniforms, 0, &block);
        }
    }
}
