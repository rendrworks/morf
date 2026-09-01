//! Backend-independent draw lists, damage tracking, and GPU instance data.

mod gpu;

pub use gpu::{GpuError, GpuInfo, ShaderRegistration, WgpuBackend};

mod commands;
mod damage;
mod effects;
mod field;
mod paint;
mod paint_fields;
mod sdf;

pub use commands::*;
pub use damage::*;
pub use field::*;
pub use sdf::*;
#[cfg(test)]
mod tests;
