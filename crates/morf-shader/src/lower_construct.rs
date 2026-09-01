//! Building values: vectors, matrices, and the conversions between scalars.
//!
//! What each of these has in common is that the *result* type is known and the
//! arguments have to be made to fit it — which is the opposite direction from
//! everywhere else in the lowerer, and the reason they sit together.

use crate::ir::*;
use crate::lower::Lowerer;
use crate::lower_expr::scalar_of;
use crate::types::*;

impl Lowerer<'_> {
    pub(crate) fn construct(&mut self, ty: Type, args: Vec<Expr>, line: u32) -> Expr {
        let wanted = ty.components();
        // What the components have to be. A `vec4u` is built from whole
        // numbers, and committing its arguments to `f32` — which is what
        // happens to every undecided literal elsewhere — would emit a
        // conversion WGSL refuses.
        let element = scalar_of(ty);
        if args.is_empty() {
            self.error(line, format!("{ty} needs at least one component"));
            return Expr::poison();
        }
        if args.iter().any(|arg| arg.ty().is_poison()) {
            return Expr::Construct { ty, args };
        }
        let settle = |arg: Expr| {
            if element == Type::F32 {
                Lowerer::commit(arg)
            } else {
                arg
            }
        };
        // One scalar fills every component: `vec3(0.5)` is grey, and so is
        // `vec3(1)` — an abstract literal is a scalar like any other.
        if args.len() == 1 && args[0].ty().fits(element) {
            return Expr::Construct {
                ty,
                args: args.into_iter().map(settle).collect(),
            };
        }
        let mut supplied = 0;
        for arg in &args {
            let arg_ty = arg.ty();
            // Each piece is either the element itself or a vector of it.
            let ok = arg_ty.fits(element) || scalar_of(arg_ty) == element && arg_ty.is_any_vector();
            if !ok {
                self.error(line, format!("{ty} is built from {element}, not {arg_ty}"));
                return Expr::poison();
            }
            supplied += u32::from(arg_ty.components());
        }
        if supplied != u32::from(wanted) {
            self.error(
                line,
                format!("{ty} needs {wanted} components, but {supplied} were given"),
            );
            return Expr::poison();
        }
        Expr::Construct {
            ty,
            args: args.into_iter().map(settle).collect(),
        }
    }

    /// A scalar conversion or a bitcast, if the name is one.
    pub(crate) fn conversion(&mut self, name: &str, args: &[Expr], line: u32) -> Option<Expr> {
        let (builtin, target) = match name {
            "f32" | "float" => (Builtin::Convert, Type::F32),
            "i32" | "int" => (Builtin::Convert, Type::I32),
            "u32" | "uint" => (Builtin::Convert, Type::U32),
            "bitcast_f32" => (Builtin::Bitcast, Type::F32),
            "bitcast_i32" => (Builtin::Bitcast, Type::I32),
            "bitcast_u32" => (Builtin::Bitcast, Type::U32),
            _ => return None,
        };
        if args.len() != 1 {
            self.error(
                line,
                format!("{name} takes one argument, not {}", args.len()),
            );
            return Some(Expr::poison());
        }
        let from = args[0].ty();
        if from.is_poison() {
            return Some(Expr::poison());
        }
        if from.is_vector() || from.is_matrix() {
            self.error(line, format!("{name} converts a single number, not {from}"));
            return Some(Expr::poison());
        }
        // A bitcast only makes sense between things of the same width, which
        // for this language means the four-byte scalars and nothing else.
        if builtin == Builtin::Bitcast && from == Type::Bool {
            self.error(line, "a bool has no bits to reinterpret");
            return Some(Expr::poison());
        }
        Some(Expr::Call {
            builtin,
            ty: target,
            args: vec![args[0].clone()],
        })
    }

    /// `mat3(c0, c1, c2)`, from columns or from every component at once.
    ///
    /// WGSL accepts both spellings and so does this: columns are how a rotation
    /// is usually written, and the flat form is how one gets pasted out of
    /// somebody else's shader.
    pub(crate) fn construct_matrix(&mut self, ty: Type, args: Vec<Expr>, line: u32) -> Expr {
        if args.iter().any(|arg| arg.ty().is_poison()) {
            return Expr::Construct { ty, args };
        }
        let columns = usize::from(ty.columns());
        let column = ty.column().expect("a matrix has a column type");
        if args.len() == columns && args.iter().all(|arg| arg.ty() == column) {
            return Expr::Construct { ty, args };
        }
        if args.len() == columns * columns && args.iter().all(|arg| arg.ty() == Type::F32) {
            return Expr::Construct { ty, args };
        }
        let given = args
            .iter()
            .map(|arg| arg.ty().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        self.error_note(
            line,
            format!("{ty} cannot be built from ({given})"),
            format!(
                "give it {columns} {column} columns, or {} numbers",
                columns * columns
            ),
        );
        Expr::poison()
    }
}
