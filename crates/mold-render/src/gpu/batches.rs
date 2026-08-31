use crate::{DrawList, SdfFieldInstance, SdfFieldLayer, SdfFieldMaterial};

/// Groups every field and quad command, and the layers and materials they
/// carry, into one set of buffers.
///
/// A rectangle is a field of one layer, so there is one collector rather than
/// two: each instance records where its own run of layers begins, and its
/// material is found by its own instance index.
pub(crate) fn collect_field_instances(
    list: &DrawList,
    scale_120: u32,
) -> (
    Vec<Option<u32>>,
    Vec<SdfFieldInstance>,
    Vec<SdfFieldLayer>,
    Vec<SdfFieldMaterial>,
) {
    let mut indices = vec![None; list.commands.len()];
    let mut instances = Vec::new();
    let mut layers = Vec::new();
    let mut materials = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        if let Some(instance) =
            SdfFieldInstance::from_command(command, scale_120, &mut layers, &mut materials)
        {
            indices[command_index] = Some(instances.len() as u32);
            instances.push(instance);
        }
    }
    debug_assert_eq!(
        instances.len(),
        materials.len(),
        "the shader finds a material by instance index, so they march together",
    );
    (indices, instances, layers, materials)
}
