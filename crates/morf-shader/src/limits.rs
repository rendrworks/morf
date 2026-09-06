//! What a shader is not allowed to exceed.
//!
//! A hung shader loses the wgpu device, and losing the device kills the
//! compositor — the bar, the lock screen, the session. There is no "restart the
//! app". Every number here exists because the blast radius of getting it wrong
//! is the user's whole desktop, so each is enforced as a diagnostic naming the
//! cap and the measured value, never as a panic and never silently.

/// How many times any one loop may run before it is cut off.
///
/// Emitted as a counter the shader cannot reach around, because the author
/// never writes the loop — they write a `while`, and the compiler decides what
/// it becomes.
pub const MAX_ITERATIONS: u32 = 4096;

/// How deeply loops may nest.
///
/// Past this the product of the per-loop guards stops meaning anything: four
/// levels at the ceiling is already far more work than a frame can afford.
pub const MAX_LOOP_NESTING: u32 = 4;

/// How many operations one shader may hold.
///
/// Shader compilation is superlinear in program size, so an enormous shader
/// does not fail — it stalls the session while the driver thinks.
pub const MAX_IR_NODES: u32 = 100_000;

/// How long a shader's source may be.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;

/// How many parameters one shader may declare.
pub const MAX_PARAMS: usize = 32;
