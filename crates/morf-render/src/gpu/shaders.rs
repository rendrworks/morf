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

/// The default hook bodies, replaced when a configuration attaches a shader.
const FILL_HOOK: &str = "    return base;";
const COVERAGE_HOOK: &str = "    return filled;";
const VERTEX_HOOK: &str = "    return corner;";

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
pub(crate) fn field_shader_source(
    body: &str,
    shader: Option<&str>,
    owns_coverage: bool,
    vertex: Option<&str>,
) -> Option<String> {
    let base = shader_source(body);
    // A vertex displacement is spliced whether or not there is also a fragment
    // shader: moving a node and colouring it are separate things a
    // configuration may want either of.
    let base = match vertex {
        None => base,
        Some(vertex) => {
            if !base.contains(VERTEX_HOOK) {
                return None;
            }
            // Both stages compile to a function called `morf_shader_main`, and
            // a module cannot hold two. The vertex one is renamed on the way
            // in, which is cheaper than teaching the compiler which stage it is
            // emitting for a second time.
            let vertex = vertex.replace("morf_shader_main", "morf_vertex_main");
            let hooked = base.replacen(
                VERTEX_HOOK,
                "    return morf_vertex_main(corner, size, time);",
                1,
            );
            format!("{vertex}\n{hooked}")
        }
    };
    let Some(shader) = shader else {
        return Some(base);
    };
    if !base.contains(FILL_HOOK) || !base.contains(COVERAGE_HOOK) {
        return None;
    }
    let mut hooked = base.replacen(
        FILL_HOOK,
        "    return morf_shader_main(uv, local, coverage, base);",
        1,
    );
    if owns_coverage {
        // A surface shader decides its own alpha, so the field's coverage is
        // not consulted at all — which is what stops a shaped node and a
        // surface shader from each half-deciding what is drawn.
        hooked = hooked.replacen(COVERAGE_HOOK, "    return shader_alpha;", 1);
    }
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
    /// Layers are passed alongside the commands because an effect shader is a
    /// different thing in two ways that both land here: its parameters live on
    /// the layer it turned into rather than on any command, and it is
    /// registered in its own table because it splices into a different
    /// pipeline. Looking only at the commands and only at `shaders` left every
    /// effect in the session reading zeros for everything it declared, which
    /// looks exactly like an effect that does nothing.
    pub(crate) fn write_shader_uniforms(
        &mut self,
        bindings: &[Option<ShaderBinding>],
        layers: &[crate::Layer],
        scale_120: u32,
    ) {
        // The clock rides in the viewport uniform's third slot so the *vertex*
        // stage can read it: a shader that displaces a corner needs the time
        // before there is a fragment to hand it to.
        let viewport = [self.width as f32, self.height as f32, self.elapsed, 0.0];
        self.queue
            .write_buffer(&self.viewport_buffer, 0, bytemuck::cast_slice(&viewport));
        if self.shaders.is_empty() && self.effect_shaders.is_empty() {
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
            if let Some(program) = self.shaders.get(&binding.program) {
                Self::write_shader_block(&self.queue, program, binding, &header);
            }
        }
        for binding in layers.iter().filter_map(|layer| layer.shader.as_ref()) {
            if let Some(program) = self.effect_shaders.get(&binding.program) {
                Self::write_shader_block(&self.queue, program, binding, &header);
            }
        }
    }

    /// Fills one program's uniform block: the frame's own values, then whatever
    /// the configuration declared, at the offsets the compiler computed.
    fn write_shader_block(
        queue: &wgpu::Queue,
        program: &ShaderProgram,
        binding: &ShaderBinding,
        header: &[f32; 4],
    ) {
        let mut block = vec![0u8; program.size as usize];
        block[..16].copy_from_slice(bytemuck::cast_slice(header));
        for (index, offset) in program.offsets.iter().enumerate() {
            let Some(value) = binding.params.get(index) else {
                break;
            };
            let start = *offset as usize;
            if start + 4 <= block.len() {
                block[start..start + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_buffer(&program.uniforms, 0, &block);
        // Data blocks are the configuration's own numbers, written whole each
        // frame: they are small, and a diff would cost more to track than the
        // write costs to do.
        if let Some((buffers, _)) = &program.data {
            for (buffer, values) in buffers.iter().zip(&binding.data) {
                if !values.is_empty() {
                    queue.write_buffer(buffer, 0, bytemuck::cast_slice(values));
                }
            }
        }
    }
}

/// The default effect hook body, replaced when a layer carries a shader.
const EFFECT_HOOK: &str = "    return sampled;";

/// Builds the glyph shader with a configuration's effect shader spliced in.
///
/// The effect shader sees what the layer already rendered and returns what
/// should be composited instead, so a distortion samples around the point it
/// was asked about rather than only at it.
pub(crate) fn effect_shader_source(body: &str, shader: Option<&str>) -> Option<String> {
    let base = shader_source(body);
    let Some(shader) = shader else {
        return Some(base);
    };
    if !base.contains(EFFECT_HOOK) {
        return None;
    }
    let hooked = base.replacen(
        EFFECT_HOOK,
        "    return morf_shader_main(uv, uv, sampled.a, sampled);",
        1,
    );
    Some(format!("{shader}\n{hooked}"))
}
