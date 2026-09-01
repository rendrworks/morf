//! The uniform block layout, computed once so the host and the shader cannot
//! disagree about it.
//!
//! WGSL's alignment rules are where this sort of code goes wrong quietly: a
//! wrong offset does not error, it shears. Every rule here is stated with the
//! reason it surprises somebody.

use crate::ir::Binding;
use crate::types::Type;

/// How many bytes the built-in header occupies before the first parameter.
///
/// `resolution` then `time`, padded to sixteen. Fixed rather than packed with
/// the rest so a host writing the clock does not have to know what a particular
/// shader declared.
pub const HEADER_BYTES: u32 = 16;

/// Where one parameter sits in the uniform block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamSlot {
    pub name: String,
    pub ty: Type,
    /// Byte offset from the start of the block.
    pub offset: u32,
}

/// Computes the uniform block layout.
///
/// WGSL's alignment rules, applied once here so the host writing the buffer and
/// the shader reading it cannot disagree: the offsets travel with the compiled
/// shader rather than being recomputed on the other side.
pub(crate) fn pack(params: &[Binding]) -> (Vec<ParamSlot>, u32) {
    let mut slots = Vec::with_capacity(params.len());
    // The frame's own values come first, at a fixed offset, so the host can
    // write them without consulting the layout of whatever the configuration
    // declared after them.
    let mut offset = HEADER_BYTES;
    for param in params {
        let (size, alignment) = param.ty.layout();
        offset = offset.next_multiple_of(alignment);
        slots.push(ParamSlot {
            name: param.name.clone(),
            ty: param.ty,
            offset,
        });
        offset += size;
    }
    // A uniform block is itself padded to sixteen, so a `vec3` at the end does
    // not leave the buffer short of what the binding expects.
    (slots, offset.next_multiple_of(16).max(16))
}
