use std::fmt::Write as _;

use crate::ir::*;
use crate::types::*;

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

/// Prints a type-checked program as WGSL.
///
/// The emitter makes no decisions — every type is already resolved — so it is a
/// direct print. Anything it would have to infer is a sign lowering left work
/// undone.
pub(crate) fn emit(program: &Program, slots: &[ParamSlot], size: u32) -> String {
    let mut out = String::with_capacity(2048);
    emit_uniforms(&mut out, slots, size);
    // One fixed signature, whatever the shader declared, so the host's call
    // site never varies. The declared inputs are bound inside from wherever
    // they actually come from — the fragment stage, or the uniform header.
    out.push_str(
        "fn morf_shader_main(\n    \
         uv: vec2<f32>,\n    \
         local: vec2<f32>,\n    \
         coverage: f32,\n    \
         base: vec4<f32>,\n\
         ) -> vec4<f32> {\n",
    );
    for (index, input) in program.inputs.iter().enumerate() {
        let source = match input.name.as_str() {
            "uv" => "uv".to_owned(),
            "coverage" => "coverage".to_owned(),
            "local" => "local".to_owned(),
            "time" => "morf_u.morf_time".to_owned(),
            "resolution" => "morf_u.morf_resolution".to_owned(),
            // An input the host does not supply is still bound, so a shader
            // naming one gets zero rather than a WGSL compile error the author
            // has no way to read.
            _ => format!("{}()", zero(input.ty)),
        };
        let _ = writeln!(
            out,
            "    let morf_in{index}: {} = {source};",
            input.ty.wgsl()
        );
    }
    let mut state = Emitter {
        out,
        depth: 1,
        loops: 0,
    };
    state.block(&program.entry.body);
    state.out.push_str("}\n");
    state.out
}

/// The zero constructor for a type, for an input nothing supplies.
fn zero(ty: Type) -> &'static str {
    match ty {
        Type::Vec2 => "vec2<f32>",
        Type::Vec3 => "vec3<f32>",
        Type::Vec4 => "vec4<f32>",
        _ => "f32",
    }
}

fn emit_uniforms(out: &mut String, slots: &[ParamSlot], size: u32) {
    out.push_str(
        "struct MorfShaderUniforms {\n    \
         morf_resolution: vec2<f32>,\n    \
         morf_time: f32,\n    \
         morf_header_pad: f32,\n",
    );
    let mut offset = HEADER_BYTES;
    for (index, slot) in slots.iter().enumerate() {
        let (slot_size, alignment) = slot.ty.layout();
        let aligned = offset.next_multiple_of(alignment);
        pad(out, offset, aligned);
        // Named by index rather than by what the configuration called it: a
        // parameter called `sin` or `let` would not be a legal WGSL member, and
        // the author's own name is no help inside generated code anyway.
        let _ = writeln!(
            out,
            "    morf_param{index}: {}, // {}",
            slot.ty.wgsl(),
            slot.name
        );
        offset = aligned + slot_size;
    }
    pad(out, offset, size);
    out.push_str("};\n@group(1) @binding(0) var<uniform> morf_u: MorfShaderUniforms;\n\n");
}

/// Emits padding members between two offsets.
///
/// One `f32` per four bytes rather than an `array<f32, n>`: in the uniform
/// address space an array's stride has to be a multiple of sixteen, so the
/// obvious spelling is rejected outright by the validator.
fn pad(out: &mut String, from: u32, to: u32) {
    for slot in (from..to).step_by(4) {
        let _ = writeln!(out, "    morf_pad{slot}: f32,");
    }
}

struct Emitter {
    out: String,
    depth: usize,
    loops: u32,
}

impl Emitter {
    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
    }

    fn block(&mut self, block: &Block) {
        for statement in &block.0 {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Let {
                name,
                ty,
                value,
                mutable,
            } => {
                self.indent();
                let keyword = if *mutable { "var" } else { "let" };
                let _ = write!(self.out, "{keyword} {name}: {} = ", ty.wgsl());
                self.expression(value, *ty);
                self.out.push_str(";\n");
            }
            Stmt::Assign { target, value } => {
                self.indent();
                let _ = write!(self.out, "{target} = ");
                self.expression(value, value.ty());
                self.out.push_str(";\n");
            }
            Stmt::If { arms, otherwise } => self.branch(arms, otherwise.as_ref()),
            Stmt::Loop { guard, body } => self.loop_(*guard, body),
            Stmt::Break => {
                self.indent();
                self.out.push_str("break;\n");
            }
            Stmt::Return(value) => {
                self.indent();
                self.out.push_str("return ");
                self.expression(value, value.ty());
                self.out.push_str(";\n");
            }
        }
    }

    fn branch(&mut self, arms: &[(Expr, Block)], otherwise: Option<&Block>) {
        for (index, (condition, body)) in arms.iter().enumerate() {
            self.indent();
            self.out
                .push_str(if index == 0 { "if (" } else { "} else if (" });
            self.expression(condition, Type::Bool);
            self.out.push_str(") {\n");
            self.depth += 1;
            self.block(body);
            self.depth -= 1;
        }
        if let Some(body) = otherwise {
            self.indent();
            self.out.push_str("} else {\n");
            self.depth += 1;
            self.block(body);
            self.depth -= 1;
        }
        self.indent();
        self.out.push_str("}\n");
    }

    /// A loop, with the guard that makes it terminate.
    ///
    /// The counter is not an optimisation and not advice: it is the only reason
    /// a configuration cannot take the compositor down with `while true do end`.
    fn loop_(&mut self, guard: u32, body: &Block) {
        let counter = format!("morf_guard{}", self.loops);
        self.loops += 1;
        self.indent();
        let _ = writeln!(self.out, "var {counter}: u32 = 0u;");
        self.indent();
        self.out.push_str("loop {\n");
        self.depth += 1;
        self.indent();
        let _ = writeln!(self.out, "if ({counter} >= {guard}u) {{ break; }}");
        self.indent();
        let _ = writeln!(self.out, "{counter} = {counter} + 1u;");
        self.block(body);
        self.depth -= 1;
        self.indent();
        self.out.push_str("}\n");
    }

    /// Prints an expression, widening a scalar where the context wants a vector.
    fn expression(&mut self, expression: &Expr, wanted: Type) {
        let actual = expression.ty();
        if wanted.is_vector() && actual == Type::F32 && !matches!(expression, Expr::Literal(_)) {
            let _ = write!(self.out, "{}(", wanted.wgsl());
            self.raw(expression);
            self.out.push(')');
            return;
        }
        if wanted.is_vector()
            && actual == Type::F32
            && let Expr::Literal(Value::F32(value)) = expression
        {
            let _ = write!(self.out, "{}({})", wanted.wgsl(), float(*value));
            return;
        }
        self.raw(expression);
    }

    fn raw(&mut self, expression: &Expr) {
        match expression {
            Expr::Literal(Value::F32(value)) => {
                let _ = write!(self.out, "{}", float(*value));
            }
            Expr::Literal(Value::I32(value)) => {
                let _ = write!(self.out, "{value}");
            }
            Expr::Literal(Value::Bool(value)) => {
                self.out.push_str(if *value { "true" } else { "false" });
            }
            Expr::Local { name, .. } => self.out.push_str(name),
            Expr::Param { index, .. } => {
                let _ = write!(self.out, "morf_u.morf_param{index}");
            }
            Expr::Input { index, .. } => {
                let _ = write!(self.out, "morf_in{index}");
            }
            Expr::Unary { op, value, .. } => {
                self.out.push_str(match op {
                    UnOp::Negate => "-(",
                    UnOp::Not => "!(",
                });
                self.raw(value);
                self.out.push(')');
            }
            Expr::Binary {
                op,
                ty,
                left,
                right,
            } => {
                self.out.push('(');
                // A comparison keeps its operands' own type; arithmetic widens
                // a scalar to the result so WGSL sees two matching sides.
                let context = if op.is_comparison() || op.is_logical() {
                    left.ty()
                } else {
                    *ty
                };
                self.expression(left, context);
                let _ = write!(self.out, " {} ", op.wgsl());
                let right_context = if op.is_comparison() || op.is_logical() {
                    right.ty()
                } else {
                    *ty
                };
                self.expression(right, right_context);
                self.out.push(')');
            }
            Expr::Call { builtin, ty, args } => self.call(*builtin, *ty, args),
            Expr::Construct { ty, args } => {
                let _ = write!(self.out, "{}(", ty.wgsl());
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.raw(arg);
                }
                self.out.push(')');
            }
            Expr::Swizzle {
                value,
                components,
                len,
                ..
            } => {
                self.raw(value);
                self.out.push('.');
                for slot in &components[..*len as usize] {
                    self.out.push(match slot {
                        0 => 'x',
                        1 => 'y',
                        2 => 'z',
                        _ => 'w',
                    });
                }
            }
        }
    }

    fn call(&mut self, builtin: Builtin, ty: Type, args: &[Expr]) {
        if builtin == Builtin::Texture {
            // A function the host shader provides, rather than a binding this
            // crate declares. Which texture is underneath, and in which bind
            // group, is the renderer's business: a compiler that named one
            // would have to know how every pass is wired.
            self.out.push_str("morf_sample(");
            self.raw(&args[0]);
            self.out.push(')');
            return;
        }
        let _ = write!(self.out, "{}(", builtin.wgsl());
        for (index, arg) in args.iter().enumerate() {
            if index > 0 {
                self.out.push_str(", ");
            }
            // `select`'s condition and the fold builtins keep their own types;
            // everything else widens to the call's result.
            let context = match (builtin, index) {
                (Builtin::Select, 2) => Type::Bool,
                (Builtin::Length | Builtin::Dot | Builtin::Distance, _) => arg.ty(),
                _ => ty,
            };
            self.expression(arg, context);
        }
        self.out.push(')');
    }
}

/// Prints a float WGSL will read back as a float.
///
/// `1` is an integer literal in WGSL and will not coerce, so every value needs
/// a decimal point whether or not it has a fraction.
fn float(value: f32) -> String {
    if value.is_nan() || value.is_infinite() {
        // Neither is expressible as a WGSL literal, and both come only from a
        // configuration doing something it should be told about elsewhere.
        return "0.0".to_owned();
    }
    let printed = format!("{value:?}");
    if printed.contains(['.', 'e', 'E']) {
        printed
    } else {
        format!("{printed}.0")
    }
}
