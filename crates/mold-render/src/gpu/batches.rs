use crate::{DrawList, SdfFieldInstance, SdfFieldLayer, SdfQuadInstance};

/// Groups every quad command in the list into one instance buffer.
pub(crate) fn collect_quad_instances(
    list: &DrawList,
    scale_120: u32,
) -> (Vec<Option<u32>>, Vec<SdfQuadInstance>) {
    let mut indices = vec![None; list.commands.len()];
    let mut instances = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        if let Some(instance) = SdfQuadInstance::from_command(command, scale_120) {
            indices[command_index] = Some(instances.len() as u32);
            instances.push(instance);
        }
    }
    (indices, instances)
}

/// Groups every field command, and the layers they compose, into one pair of
/// buffers. Each field records where its own run of layers begins.
pub(crate) fn collect_field_instances(
    list: &DrawList,
    scale_120: u32,
) -> (Vec<Option<u32>>, Vec<SdfFieldInstance>, Vec<SdfFieldLayer>) {
    let mut indices = vec![None; list.commands.len()];
    let mut instances = Vec::new();
    let mut layers = Vec::new();
    for (command_index, command) in list.commands.iter().enumerate() {
        if let Some(instance) = SdfFieldInstance::from_command(command, scale_120, &mut layers) {
            indices[command_index] = Some(instances.len() as u32);
            instances.push(instance);
        }
    }
    (indices, instances, layers)
}
