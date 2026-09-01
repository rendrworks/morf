use std::fmt::Write as _;

use crate::ir::*;
use crate::pack::{HEADER_BYTES, ParamSlot};
use crate::types::*;

/// Prints a type-checked program as WGSL.
///
/// The emitter makes no decisions — every type is already resolved — so it is a
/// direct print. Anything it would have to infer is a sign lowering left work
/// undone.
pub(crate) fn emit(program: &Program, slots: &[ParamSlot], size: u32) -> String {
    let mut out = String::with_capacity(2048);
    // A vertex displacement declares no uniform block. It takes the clock as an
    // argument and has no parameters of its own, and when both stages are
    // spliced into one module two blocks of the same name is a redefinition.
    let vertex = program.entry.returns == Type::Vec2;
    if !vertex {
        emit_uniforms(&mut out, slots, size);
    }
    // Textures and data first: a declaration has to be in scope before the
    // function that names it, and both can appear inside a helper.
    for (slot, name) in program.textures.iter().enumerate() {
        let _ = writeln!(
            out,
            "// {name}\n@group(2) @binding({}) var morf_tex{slot}: texture_2d<f32>;",
            slot * 2
        );
        let _ = writeln!(
            out,
            "@group(2) @binding({}) var morf_tex_sampler{slot}: sampler;",
            slot * 2 + 1
        );
    }
    if !program.textures.is_empty() {
        out.push('\n');
    }
    for (slot, (name, element, _)) in program.data.iter().enumerate() {
        // Read-only on purpose: a fragment shader writing shared storage is a
        // race between every pixel of the node.
        let _ = writeln!(
            out,
            "// {name}\n@group(3) @binding({slot}) var<storage, read> morf_data{slot}: array<{}>;",
            element.wgsl_owned()
        );
    }
    if !program.data.is_empty() {
        out.push('\n');
    }
    // Record declarations first: WGSL needs a struct in scope before the
    // function that names it, and a record can appear in a helper's signature.
    //
    // Collected from this program rather than from the interner: the interner
    // is process-wide, so reading it would put every record any shader ever
    // used into every shader after it.
    let mut records = Vec::new();
    for helper in &program.helpers {
        collect_records(&helper.body, &mut records);
    }
    collect_records(&program.entry.body, &mut records);
    for record in records {
        let _ = writeln!(out, "struct {} {{", record.name);
        for (field, ty) in &record.fields {
            let _ = writeln!(out, "    {field}: {},", ty.wgsl_owned());
        }
        out.push_str("};\n\n");
    }
    for helper in &program.helpers {
        let signature = helper
            .params
            .iter()
            .map(|(name, ty)| format!("{name}: {}", ty.wgsl_owned()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "fn {}({signature}) -> {} {{",
            helper.name,
            helper.returns.wgsl_owned()
        );
        let mut state = Emitter {
            out,
            depth: 1,
            loops: 0,
        };
        state.block(&helper.body);
        out = state.out;
        out.push_str("}\n\n");
    }
    // One fixed signature, whatever the shader declared, so the host's call
    // site never varies. The declared inputs are bound inside from wherever
    // they actually come from — the fragment stage, or the uniform header.
    if program.entry.returns == Type::Vec2 {
        // A vertex displacement: one corner in, one corner out.
        out.push_str(
            "fn morf_shader_main(\n    \
             corner: vec2<f32>,\n    \
             size: vec2<f32>,\n    \
             time: f32,\n\
             ) -> vec2<f32> {\n",
        );
    } else {
        out.push_str(
            "fn morf_shader_main(\n    \
             uv: vec2<f32>,\n    \
             local: vec2<f32>,\n    \
             coverage: f32,\n    \
             base: vec4<f32>,\n\
             ) -> vec4<f32> {\n",
        );
    }
    for (index, input) in program.inputs.iter().enumerate() {
        let source = match input.name.as_str() {
            "corner" => "corner".to_owned(),
            "size" => "size".to_owned(),
            "uv" => "uv".to_owned(),
            "coverage" => "coverage".to_owned(),
            "local" => "local".to_owned(),
            // A vertex shader has no uniform block of its own: it runs before
            // the material is looked up, so the clock comes in as an argument.
            "time" if program.entry.returns == Type::Vec2 => "time".to_owned(),
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
            input.ty.wgsl_owned()
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
            slot.ty.wgsl_owned(),
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

pub(crate) struct Emitter {
    pub(crate) out: String,
    pub(crate) depth: usize,
    pub(crate) loops: u32,
}

impl Emitter {
    pub(crate) fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
    }

    pub(crate) fn block(&mut self, block: &Block) {
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
                // A `modf` or `frexp` result is a struct naga names itself, and
                // the name is internal and version-specific. WGSL infers a
                // `let`'s type, so the annotation is simply left off — which
                // keeps `local parts = modf(x)` working without this compiler
                // having to guess what the struct is called.
                if *ty == Type::Split {
                    let _ = write!(self.out, "let {name} = ");
                } else {
                    let _ = write!(self.out, "{keyword} {name}: {} = ", ty.wgsl_owned());
                }
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
            Stmt::Loop {
                guard,
                body,
                continuing,
            } => self.loop_(*guard, body, continuing),
            Stmt::Break => {
                self.indent();
                self.out.push_str("break;\n");
            }
            Stmt::Continue => {
                self.indent();
                self.out.push_str("continue;\n");
            }
            Stmt::Discard => {
                self.indent();
                self.out.push_str("discard;\n");
            }
            Stmt::Return(value) => {
                self.indent();
                self.out.push_str("return ");
                self.expression(value, value.ty());
                self.out.push_str(";\n");
            }
        }
    }

    /// A loop, with the guard that makes it terminate.
    ///
    /// The counter is not an optimisation and not advice: it is the only reason
    /// a configuration cannot take the compositor down with `while true do end`.
    fn loop_(&mut self, guard: u32, body: &Block, continuing: &Block) {
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
        // Whatever has to happen on the way round, including after a
        // `continue`. A counting loop's increment lives here or it is skipped.
        if !continuing.0.is_empty() {
            self.indent();
            self.out.push_str("continuing {\n");
            self.depth += 1;
            self.block(continuing);
            self.depth -= 1;
            self.indent();
            self.out.push_str("}\n");
        }
        self.depth -= 1;
        self.indent();
        self.out.push_str("}\n");
    }

    /// Prints an expression, widening a scalar where the context wants a vector.
    pub(crate) fn expression(&mut self, expression: &Expr, wanted: Type) {
        let actual = expression.ty();
        // An abstract *expression* — arithmetic over literals that nothing has
        // decided about — takes the wanted type all the way down. `0 - 2` in a
        // `vec4u` has to print as `0u - 2u`, not as floats WGSL then refuses to
        // convert.
        if actual == Type::AbstractInt && wanted.is_integer() {
            match expression {
                Expr::Binary {
                    op, left, right, ..
                } => {
                    self.out.push('(');
                    self.expression(left, wanted);
                    let _ = write!(self.out, " {} ", op.wgsl());
                    self.expression(right, wanted);
                    self.out.push(')');
                    return;
                }
                Expr::Unary { op, value, .. } => {
                    self.out.push_str(match op {
                        UnOp::Negate => "-(",
                        UnOp::Not => "!(",
                        UnOp::BitNot => "~(",
                    });
                    self.expression(value, wanted);
                    self.out.push(')');
                    return;
                }
                _ => {}
            }
        }
        // An abstract integer prints as whatever the surrounding code wanted.
        // This is the only place that knows, which is why it is the only place
        // that decides.
        if let Expr::Literal(Value::Int(value)) = expression {
            match wanted {
                Type::U32 => {
                    let _ = write!(self.out, "{}u", *value as u32);
                }
                Type::I32 => {
                    let _ = write!(self.out, "{}", *value as i32);
                }
                other if other.is_vector() => {
                    let _ = write!(self.out, "{}({})", other.wgsl(), float(*value as f32));
                }
                _ => {
                    let _ = write!(self.out, "{}", float(*value as f32));
                }
            }
            return;
        }
        // A scalar never widens into a matrix. `m * 2.0` is a scaled matrix in
        // WGSL already, and `mat3x3<f32>(2.0)` is not the same thing — it is
        // not even legal.
        if wanted.is_matrix() && actual != wanted {
            self.raw(expression);
            return;
        }
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
}

/// Gathers the record types a block actually uses, in declaration order.
fn collect_records(block: &Block, out: &mut Vec<&'static StructType>) {
    let note = |ty: Type, out: &mut Vec<&'static StructType>| {
        if let Type::Struct(record) = ty
            && !out.iter().any(|known| std::ptr::eq(*known, record))
        {
            out.push(record);
        }
    };
    for statement in &block.0 {
        match statement {
            Stmt::Let { ty, value, .. } => {
                note(*ty, out);
                note(value.ty(), out);
            }
            Stmt::Assign { value, .. } | Stmt::Return(value) => note(value.ty(), out),
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    collect_records(body, out);
                }
                if let Some(body) = otherwise {
                    collect_records(body, out);
                }
            }
            Stmt::Loop {
                body, continuing, ..
            } => {
                collect_records(body, out);
                collect_records(continuing, out);
            }
            Stmt::Break | Stmt::Continue | Stmt::Discard => {}
        }
    }
}

/// Prints a float WGSL will read back as a float.
///
/// `1` is an integer literal in WGSL and will not coerce, so every value needs
/// a decimal point whether or not it has a fraction.
pub(crate) fn float(value: f32) -> String {
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

/// The scalar a vector or matrix is built from.
pub(crate) fn element_of(ty: Type) -> Type {
    match ty {
        Type::Vec2I | Type::Vec3I | Type::Vec4I => Type::I32,
        Type::Vec2U | Type::Vec3U | Type::Vec4U => Type::U32,
        _ => Type::F32,
    }
}
