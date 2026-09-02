use morf_svg::SvgOutlines;
use morf_text::TextSystem;

use crate::{DrawList, SdfFieldInstance, SdfFieldLayer, SdfFieldMaterial, ShaderBinding};

/// Everything one frame's fields need, gathered in one walk of the list.
pub(crate) struct FieldBatch {
    /// Instance index per draw command, or `None` if the command is not a field.
    pub(crate) indices: Vec<Option<u32>>,
    pub(crate) instances: Vec<SdfFieldInstance>,
    pub(crate) layers: Vec<SdfFieldLayer>,
    pub(crate) materials: Vec<SdfFieldMaterial>,
    /// Outline points for every polygon layer in the frame, end to end. A
    /// layer records where its own run begins and how long it is.
    pub(crate) outlines: Vec<[f32; 2]>,
    /// Parallel to `instances`: which pipeline draws each, which is not
    /// instance data because it selects the pipeline rather than riding in it.
    pub(crate) shaders: Vec<Option<ShaderBinding>>,
}

/// Groups every field and quad command, and the layers and materials they
/// carry, into one set of buffers.
///
/// A rectangle is a field of one layer, so there is one collector rather than
/// two: each instance records where its own run of layers begins, and its
/// material is found by its own instance index.
pub(crate) fn collect_field_instances(
    list: &DrawList,
    scale_120: u32,
    text: &mut TextSystem,
    drawings: &mut SvgOutlines,
) -> FieldBatch {
    let mut indices = vec![None; list.commands.len()];
    let mut instances = Vec::new();
    let mut layers = Vec::new();
    let mut materials = Vec::new();
    let mut outlines = Vec::new();
    // Parallel to `instances`, because which pipeline draws an instance is not
    // instance data: it selects the pipeline itself.
    let mut shaders = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        if let Some(instance) = SdfFieldInstance::from_command(
            command,
            scale_120,
            &mut layers,
            &mut materials,
            &mut outlines,
            text,
            drawings,
        ) {
            indices[command_index] = Some(instances.len() as u32);
            instances.push(instance);
            // Both a field and a rectangle can carry one: a rectangle is a
            // field of one layer, and that is the shape most configurations
            // reach for first.
            shaders.push(match command {
                crate::DrawCommand::Field { shader, .. }
                | crate::DrawCommand::Quad { shader, .. } => shader.clone(),
                _ => None,
            });
        }
    }
    debug_assert_eq!(
        instances.len(),
        materials.len(),
        "the shader finds a material by instance index, so they march together",
    );
    FieldBatch {
        indices,
        instances,
        layers,
        materials,
        outlines,
        shaders,
    }
}
