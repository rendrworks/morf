//! Printing expressions.
//!
//! Split from the statement printer when the two crossed the line gate. Every
//! decision here is about *context*: what type the surrounding code wanted, and
//! which arguments are meant to be a different one. Getting that wrong is how a
//! shader that type-checked gets refused by a driver with no line number, which
//! has happened three times and been caught three times by the GPU tests.

use std::fmt::Write as _;

use crate::emit::{Emitter, element_of, float};
use crate::ir::*;
use crate::types::*;

impl Emitter {
    pub(crate) fn raw(&mut self, expression: &Expr) {
        match expression {
            Expr::Literal(Value::F32(value)) => {
                let _ = write!(self.out, "{}", float(*value));
            }
            Expr::Literal(Value::I32(value)) => {
                let _ = write!(self.out, "{value}");
            }
            // Reached only when nothing decided what the literal was, which
            // means `f32` — `expression` prints the other cases, because only
            // it knows what the surrounding code wanted.
            Expr::Literal(Value::Int(value)) => {
                let _ = write!(self.out, "{}", float(*value as f32));
            }
            Expr::Literal(Value::Bool(value)) => {
                self.out.push_str(if *value { "true" } else { "false" });
            }
            Expr::Local { name, .. } => self.out.push_str(name),
            Expr::Param { index, .. } => {
                let _ = write!(self.out, "morf_u.morf_param{index}");
            }
            Expr::Input { index, .. } => match *index {
                slot if slot >= crate::lower_expr::DATA_BASE => {
                    let _ = write!(self.out, "morf_data{}", slot - crate::lower_expr::DATA_BASE);
                }
                slot if slot >= crate::lower_expr::TEXTURE_BASE => {
                    let _ = write!(
                        self.out,
                        "morf_tex{}",
                        slot - crate::lower_expr::TEXTURE_BASE
                    );
                }
                slot => {
                    let _ = write!(self.out, "morf_in{slot}");
                }
            },
            Expr::Unary { op, value, .. } => {
                self.out.push_str(match op {
                    UnOp::Negate => "-(",
                    UnOp::Not => "!(",
                    UnOp::BitNot => "~(",
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
                // A matrix product's operands keep their own types: the
                // result of `m * v` is a vector, and widening `m` to it would
                // be nonsense.
                let matrix = left.ty().is_matrix() || right.ty().is_matrix();
                let paired = op.is_comparison() || op.is_logical() || matrix;
                // An undecided literal has no type of its own to keep, so it
                // takes the other side's: `stripe == 0` is two `i32`, and
                // printing the literal as `0.0` would hand the driver an `i32`
                // against an abstract float, which is not a comparison at all.
                let (left_context, right_context) = if paired {
                    (
                        left.ty().decided_or(right.ty()),
                        right.ty().decided_or(left.ty()),
                    )
                } else {
                    (*ty, *ty)
                };
                self.expression(left, left_context);
                let _ = write!(self.out, " {} ", op.wgsl());
                self.expression(right, right_context);
                self.out.push(')');
            }
            Expr::Call { builtin, ty, args } => self.call(*builtin, *ty, args),
            Expr::Construct { ty, args } => {
                let _ = write!(self.out, "{}(", ty.wgsl_owned());
                // Each component takes the vector's own scalar type, so an
                // undecided literal inside a `vec4u` prints as `1u` rather than
                // as a float WGSL will not convert.
                let element = element_of(*ty);
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    self.expression(arg, element);
                }
                self.out.push(')');
            }
            Expr::Index { value, index, .. } => {
                self.raw(value);
                self.out.push('[');
                self.expression(index, Type::I32);
                self.out.push(']');
            }
            // Arrays and records share a shape: a type name and a run of
            // values in a fixed order. A record's order is its sorted field
            // names, which is why the lowerer sorts the values to match.
            Expr::Array { ty, elements } => {
                let _ = write!(self.out, "{}(", ty.wgsl_owned());
                for (index, value) in elements.iter().enumerate() {
                    if index > 0 {
                        self.out.push_str(", ");
                    }
                    let wanted = ty.element().unwrap_or_else(|| value.ty());
                    self.expression(value, wanted);
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

    pub(crate) fn call(&mut self, builtin: Builtin, ty: Type, args: &[Expr]) {
        if builtin == Builtin::Helper {
            // The first argument is the emitted function's name, not a value.
            let Some(Expr::Local { name, .. }) = args.first() else {
                unreachable!("a helper call carries its own name");
            };
            let _ = write!(self.out, "{name}(");
            for (index, arg) in args[1..].iter().enumerate() {
                if index > 0 {
                    self.out.push_str(", ");
                }
                self.raw(arg);
            }
            self.out.push(')');
            return;
        }
        if builtin == Builtin::Outer {
            // WGSL has no `outer`: naga carries one for its GLSL frontend and
            // the WGSL grammar does not name it. It is a column of scaled
            // copies, so it is emitted as exactly that rather than as a call
            // to something no driver will find.
            let columns = ty.columns();
            let _ = write!(self.out, "{}(", ty.wgsl());
            for column in 0..columns {
                if column > 0 {
                    self.out.push_str(", ");
                }
                self.raw(&args[0]);
                self.out.push_str(" * ");
                self.raw(&args[1]);
                self.out.push('.');
                self.out.push(['x', 'y', 'z', 'w'][column as usize]);
            }
            self.out.push(')');
            return;
        }
        if builtin == Builtin::ResultField {
            // `modf(x).fract` in WGSL, which is exactly what was written.
            let Some(Expr::Local { name, .. }) = args.get(1) else {
                unreachable!("a field read carries its own field name");
            };
            self.raw(&args[0]);
            let _ = write!(self.out, ".{name}");
            return;
        }
        if builtin == Builtin::Convert {
            // A literal that had not decided what it was simply becomes the
            // target type. Wrapping it would emit `u32(2654435769u)`, which is
            // legal and silly, and going through `f32` on the way would lose
            // the constant — twenty-four bits of mantissa cannot hold a
            // thirty-two-bit hash multiplier.
            if matches!(args[0], Expr::Literal(Value::Int(_))) {
                self.expression(&args[0], ty);
                return;
            }
            let _ = write!(self.out, "{}(", ty.wgsl());
            self.expression(&args[0], args[0].ty());
            self.out.push(')');
            return;
        }
        if builtin == Builtin::Bitcast {
            let _ = write!(self.out, "bitcast<{}>(", ty.wgsl());
            self.expression(&args[0], args[0].ty());
            self.out.push(')');
            return;
        }
        // The reads that name a binding rather than take one as a value. The
        // sampler is ours, not the shader's, so it is supplied here.
        if let Some(form) = match builtin {
            Builtin::TextureDimensions => Some(TextureRead::Dimensions),
            Builtin::TextureLoad => Some(TextureRead::Load),
            Builtin::TextureSampleLevel => Some(TextureRead::Level),
            _ => None,
        } {
            let Some(Expr::Input { index, .. }) = args.first() else {
                unreachable!("a texture read is checked before it is printed");
            };
            let slot = index - crate::lower_expr::TEXTURE_BASE;
            match form {
                TextureRead::Dimensions => {
                    let _ = write!(self.out, "textureDimensions(morf_tex{slot})");
                    return;
                }
                TextureRead::Load => {
                    let _ = write!(self.out, "textureLoad(morf_tex{slot}, ");
                }
                TextureRead::Level => {
                    let _ = write!(
                        self.out,
                        "textureSampleLevel(morf_tex{slot}, morf_tex_sampler{slot}, "
                    );
                }
            }
            for (index, arg) in args[1..].iter().enumerate() {
                if index > 0 {
                    self.out.push_str(", ");
                }
                self.expression(arg, arg.ty());
            }
            self.out.push(')');
            return;
        }
        if builtin == Builtin::Texture {
            match args {
                // What is underneath, through a function the host shader
                // provides rather than a binding this crate declares. Which
                // texture is underneath, and in which bind group, is the
                // renderer's business: a compiler that named one would have to
                // know how every pass is wired.
                [coordinate] => {
                    self.out.push_str("morf_sample(");
                    self.raw(coordinate);
                }
                // A texture the configuration declared, by its own binding.
                [Expr::Input { index, .. }, coordinate] => {
                    let slot = index - crate::lower_expr::TEXTURE_BASE;
                    let _ = write!(
                        self.out,
                        "textureSample(morf_tex{slot}, morf_tex_sampler{slot}, "
                    );
                    self.raw(coordinate);
                }
                _ => unreachable!("a texture call is checked before it is printed"),
            }
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
            // Most builtins take their own result type, so a scalar written
            // against a vector call widens. The exceptions are the arguments
            // that are *meant* to be a different type: widening one of those
            // produces WGSL a driver rejects with no line number, which is the
            // worst failure this compiler can hand somebody.
            let context = match (builtin, index) {
                (Builtin::Select, 2) => Type::Bool,
                (Builtin::Refract, 2) => Type::F32,
                (Builtin::Length | Builtin::Dot | Builtin::Distance, _) => arg.ty(),
                _ => ty,
            };
            self.expression(arg, context);
        }
        self.out.push(')');
    }
}

/// Which of the reads that name a texture binding this is.
enum TextureRead {
    Dimensions,
    Load,
    Level,
}
