use morf_scene::NodeHandle;

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
