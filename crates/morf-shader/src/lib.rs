//! Shaders a configuration writes in Lua, compiled to WGSL.
//!
//! A configuration author writes a shader in Lua's syntax; this crate type
//! checks it and prints WGSL. The Lua never runs — it is parsed, lowered and
//! emitted, once, at load. What runs per pixel is compiled code on the GPU.
//!
//! # Why a compiler
//!
//! The alternative was to *trace*: run the function once with symbolic values
//! whose metamethods record a graph. Tracing is cheaper to build and safe by
//! construction, but Lua coerces `__lt` and `__eq` results to a boolean, so
//! `d > 0.5` cannot return a symbol — and a userdata is truthy, so
//! `if d > 0.5 then` would silently take the first branch and emit the wrong
//! shader. Nor can a traced loop carry a data-dependent `break`, which is the
//! whole algorithm in a raymarcher.
//!
//! Compiling costs more only in the part that was already unavoidable: Luna's
//! parser is public and hands over a line-annotated Lua AST, so there is no
//! lexer and no parser to write here. What is left is the type checker and the
//! printer.
//!
//! # Why that is still safe
//!
//! The reason to fear a compiler is that `while true do end` hangs the GPU,
//! wgpu loses the device, and the compositor dies with it — a session ended by
//! a typo in a config. But this crate decides what WGSL comes out, so every
//! emitted loop carries an iteration guard (see [`limits`]). The author writes
//! a natural `while`; it cannot run away, because they never chose what the
//! loop would become.
//!
//! # Boundaries
//!
//! This crate depends on `luna` and nothing else in the workspace. It does not
//! know what a scene is, what a node is, or that wgpu exists — which is what
//! lets the whole language be tested with `assert_eq!` on a string, without an
//! adapter and without a script engine.

use luna::compiler::{interning::BasicInterner, parser::parse_chunk};

mod builtins;
mod diagnostics;
mod emit;
mod ir;
mod limits;
mod lower;
mod lower_bits;
mod lower_expr;
mod lower_ops;
mod lower_stmt;
mod types;
mod validate;

pub use diagnostics::{Diagnostic, report};
pub use emit::{HEADER_BYTES, ParamSlot};
pub use ir::Binding;
pub use limits::*;
pub use types::{ShaderKind, Type, Value};

#[cfg(test)]
mod tests;

/// What a shader is compiled against.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShaderSpec {
    pub kind: ShaderKind,
    /// Values the host supplies every frame, in the order the entry point takes
    /// them: `uv`, `time`, `resolution`, and whatever the mode adds.
    pub inputs: Vec<Binding>,
    /// Values the configuration declares, animatable as node properties.
    pub params: Vec<Binding>,
    /// The function to compile. Conventionally `fragment`.
    pub entry: String,
}

impl ShaderSpec {
    /// The inputs every mode provides.
    pub fn default_inputs(kind: ShaderKind) -> Vec<Binding> {
        let mut inputs = vec![
            Binding {
                name: "uv".to_owned(),
                ty: Type::Vec2,
            },
            Binding {
                name: "time".to_owned(),
                ty: Type::F32,
            },
            Binding {
                name: "resolution".to_owned(),
                ty: Type::Vec2,
            },
        ];
        if kind == ShaderKind::Material {
            // How much of this pixel the shape covers. A material shader can
            // read it to fade its own effect out at the edge rather than
            // fighting the antialiasing the field already did.
            inputs.push(Binding {
                name: "coverage".to_owned(),
                ty: Type::F32,
            });
        }
        inputs
    }
}

/// A shader, ready for a pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compiled {
    /// The generated module: a `morf_shader_main` function and its uniforms,
    /// meant to be concatenated into a host shader, not compiled alone.
    pub wgsl: String,
    /// Where each parameter sits in the uniform block.
    pub params: Vec<ParamSlot>,
    /// Size of that block, padded as WGSL requires.
    pub uniform_size: u32,
    /// Whether the shader read the frame clock, and so has to repaint forever.
    pub reads_time: bool,
    /// Whether the shader sampled what is underneath it.
    pub samples_behind: bool,
    /// Pipeline cache key, over the emitted WGSL rather than the Lua, so two
    /// shaders differing only in comments share one pipeline.
    pub hash: u64,
}

/// Compiles a shader.
///
/// Pure: no filesystem, no globals, no Lua VM and no GPU. Every error is
/// returned rather than raised, and errors accumulate — an author sees
/// everything wrong with a shader in one run instead of discovering the second
/// mistake after fixing the first.
pub fn compile(source: &str, spec: &ShaderSpec) -> Result<Compiled, Vec<Diagnostic>> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(vec![Diagnostic::new(
            1,
            format!(
                "shader source is {} bytes, over the {MAX_SOURCE_BYTES} limit",
                source.len()
            ),
        )]);
    }
    if spec.params.len() > MAX_PARAMS {
        return Err(vec![Diagnostic::new(
            1,
            format!(
                "shader declares {} parameters, over the {MAX_PARAMS} limit",
                spec.params.len()
            ),
        )]);
    }
    let chunk = parse_chunk(source.as_bytes(), BasicInterner::default())
        .map_err(|error| vec![Diagnostic::new(1, error.to_string())])?;

    let mut diagnostics = Vec::new();
    let functions = lower::functions(&chunk, &spec.entry, &mut diagnostics);
    let Some(definition) = functions.entry else {
        return Err(diagnostics);
    };

    let mut lowerer = lower::Lowerer::new(&spec.inputs, &spec.params);
    lowerer.helpers = functions.helpers;
    lowerer.diagnostics = diagnostics;
    bind_parameters(&mut lowerer, definition, spec);
    let mut body = lowerer.block(&definition.body);
    body.resolve_mutability();

    let program = ir::Program {
        entry: ir::Function {
            returns: Type::Vec4,
            body,
        },
        // Helpers come out in the order they were first needed, which is also
        // an order WGSL accepts: a helper cannot call one declared after it,
        // because recursion is refused and a later helper is only lowered when
        // an earlier one reaches it.
        helpers: lowerer.lowered,
        inputs: spec.inputs.clone(),
        params: spec.params.clone(),
        reads_time: lowerer.reads_time,
        samples_behind: lowerer.samples_behind,
    };
    let mut diagnostics = lowerer.diagnostics;
    validate::check(&program, spec, &mut diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let (params, uniform_size) = emit::pack(&spec.params);
    let wgsl = emit::emit(&program, &params, uniform_size);
    let hash = hash(&wgsl);
    Ok(Compiled {
        wgsl,
        params,
        uniform_size,
        reads_time: program.reads_time,
        samples_behind: program.samples_behind,
        hash,
    })
}

/// Binds the entry function's parameter list to the declared inputs and params.
///
/// The signature lives in the specification rather than in Lua syntax, because
/// Lua has none to annotate with. The function's parameter names are checked
/// against that declaration by position, so a mismatch is caught at the
/// declaration rather than as a confusing "not defined" deeper in the body.
fn bind_parameters(
    lowerer: &mut lower::Lowerer<'_>,
    definition: &luna::compiler::parser::FunctionDefinition<lower::Name>,
    spec: &ShaderSpec,
) {
    let declared: Vec<&Binding> = spec.inputs.iter().chain(spec.params.iter()).collect();
    if definition.has_varargs {
        lowerer.error(1, "a shader entry point cannot take `...`");
    }
    if definition.parameters.len() > declared.len() {
        lowerer.error_note(
            1,
            format!(
                "`{}` takes {} arguments, but only {} are declared",
                spec.entry,
                definition.parameters.len(),
                declared.len()
            ),
            "add them to the shader's `inputs` or `params`",
        );
    }
    // Only the names are checked. Every input is bound inside the emitted
    // function whether or not the Lua named it, so the host's call site is the
    // same shape for every shader — a shader that ignores `resolution` still
    // gets called with one.
    for (index, name) in definition.parameters.iter().enumerate() {
        let Some(binding) = declared.get(index) else {
            break;
        };
        let written = lower::text(name);
        if written != binding.name {
            lowerer.error_note(
                1,
                format!(
                    "argument {} is `{written}`, but `{}` was declared there",
                    index + 1,
                    binding.name
                ),
                "arguments come in declaration order: inputs first, then params",
            );
        }
    }
}

/// FNV-1a over the emitted source.
fn hash(source: &str) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in source.as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}
