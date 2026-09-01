//! Bitwise operators, constant folding, and the rules that make a hash
//! writable.
//!
//! One idea: whole numbers behave differently from the rest, and an integer
//! literal has not decided which kind of number it is until something asks.

use luna::compiler::parser::Expression;

use crate::ir::*;
use crate::lower::{Instance, Lowerer, Name};
use crate::types::*;

impl Lowerer<'_> {
    /// Lowers `& | ~ << >>`.
    ///
    /// These are what make a real hash possible, and a hash is what makes noise
    /// possible: without them the only noise available is the
    /// `sin(dot(p, k)) * 43758.5453` trick, which every shader author knows is
    /// a workaround for exactly this gap.
    pub(crate) fn bitwise(&mut self, op: BinOp, left: Expr, right: Expr, line: u32) -> Expr {
        let (left_ty, right_ty) = (left.ty(), right.ty());
        if !left_ty.is_integer() || !right_ty.is_integer() {
            self.error_note(
                line,
                format!(
                    "`{}` works on whole numbers, not {left_ty} and {right_ty}",
                    op.wgsl()
                ),
                "convert first: `u32(x)` or `i32(x)`",
            );
            return Expr::poison();
        }
        // A shift's right operand is a count, not a second value: WGSL wants a
        // `u32` there whatever is being shifted.
        if op.is_shift() {
            let ty = if left_ty == Type::AbstractInt {
                Type::I32
            } else {
                left_ty
            };
            return Expr::Binary {
                op,
                ty,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        // Otherwise both sides are the same integer type, and an abstract
        // literal takes the other side's.
        let ty = match (left_ty, right_ty) {
            (Type::AbstractInt, Type::AbstractInt) => Type::I32,
            (Type::AbstractInt, concrete) | (concrete, Type::AbstractInt) => concrete,
            (a, b) if a == b => a,
            _ => {
                self.error(
                    line,
                    format!("cannot combine {left_ty} and {right_ty} bitwise"),
                );
                return Expr::poison();
            }
        };
        Expr::Binary {
            op,
            ty,
            left: Box::new(left),
            right: Box::new(right),
        }
    }
}

/// The result type of arithmetic, with scalar broadcast and matrix products.
///
/// A scalar rides along with a vector for every operator, not just `*` and `/`.
/// WGSL only defines the multiplicative pair that way, so `v + 1.0` is widened
/// during emission — but a configuration author means the obvious thing by it,
/// and refusing would be pedantry.
///
/// A matrix is different in kind. `m * v` is a linear map applied to a vector,
/// not a componentwise multiply, so it is only defined for `*` and only for the
/// shapes that line up — and `m + v` has no meaning worth guessing at.
pub(crate) fn arithmetic_result(op: BinOp, left: Type, right: Type) -> Option<Type> {
    // An undecided literal takes whatever it is combined with.
    if left == Type::AbstractInt && right != Type::AbstractInt {
        return arithmetic_result(op, right, right).filter(|_| right.is_numeric());
    }
    if right == Type::AbstractInt && left != Type::AbstractInt {
        return arithmetic_result(op, left, left).filter(|_| left.is_numeric());
    }
    if left.is_matrix() || right.is_matrix() {
        if op != BinOp::Mul {
            return None;
        }
        return match (left, right) {
            // A square matrix times a matching one is a matrix.
            (a, b) if a == b => Some(a),
            // Applied to a vector, from either side: WGSL defines both, and
            // `v * m` is the row-vector form rather than a mistake.
            (matrix, vector) if matrix.is_matrix() && Some(vector) == matrix.column() => {
                Some(vector)
            }
            (vector, matrix) if matrix.is_matrix() && Some(vector) == matrix.column() => {
                Some(vector)
            }
            // Scaled.
            (matrix, Type::F32) | (Type::F32, matrix) if matrix.is_matrix() => Some(matrix),
            _ => None,
        };
    }
    if !left.is_numeric() || !right.is_numeric() {
        return None;
    }
    if left == right {
        return Some(left);
    }
    match (left, right) {
        (Type::F32, other) | (other, Type::F32) if other.is_vector() => Some(other),
        _ => None,
    }
}

impl Lowerer<'_> {
    /// Folds an expression the compiler needs to know at compile time.
    ///
    /// Only a loop step needs this. Lua evaluates the step once and its sign
    /// decides which way the comparison runs; reproducing that faithfully at
    /// runtime would need a branch inside every loop, so the language asks for
    /// a constant instead and says so when it does not get one.
    pub(crate) fn constant(&mut self, expression: &Expression<Name>, line: u32) -> Option<f32> {
        let lowered = self.expression(expression, line);
        fold(&lowered)
    }
}

fn fold(expression: &Expr) -> Option<f32> {
    match expression {
        Expr::Literal(Value::F32(value)) => Some(*value),
        Expr::Literal(Value::Int(value)) => Some(*value as f32),
        Expr::Literal(Value::I32(value)) => Some(*value as f32),
        Expr::Unary {
            op: UnOp::Negate,
            value,
            ..
        } => fold(value).map(|value| -value),
        Expr::Binary {
            op, left, right, ..
        } => {
            let (left, right) = (fold(left)?, fold(right)?);
            Some(match op {
                BinOp::Add => left + right,
                BinOp::Sub => left - right,
                BinOp::Mul => left * right,
                BinOp::Div => left / right,
                _ => return None,
            })
        }
        _ => None,
    }
}

impl Lowerer<'_> {
    /// Lowers a call to a helper the shader declared.
    ///
    /// The helper's parameter types come from the call, because Lua has nowhere
    /// to declare them. Two calls with different argument types produce two
    /// functions rather than one that tries to be both: monomorphising is the
    /// only honest reading of an untyped signature, and it costs a little
    /// generated code in exchange for exact types and exact diagnostics.
    pub(crate) fn helper_call(&mut self, name: &str, args: Vec<Expr>, line: u32) -> Option<Expr> {
        let definition = *self
            .helpers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, definition)| definition)?;
        if args.iter().any(|arg| arg.ty().is_poison()) {
            return Some(Expr::poison());
        }
        if definition.parameters.len() != args.len() {
            self.error(
                line,
                format!(
                    "`{name}` takes {} argument{}, not {}",
                    definition.parameters.len(),
                    if definition.parameters.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    args.len()
                ),
            );
            return Some(Expr::poison());
        }
        if definition.has_varargs {
            self.error(line, format!("`{name}` cannot take `...`"));
            return Some(Expr::poison());
        }
        // Recursion has no bottom to monomorphise towards, and a shader has no
        // stack to run it on either.
        if self.in_progress.iter().any(|active| active == name) {
            self.error_note(
                line,
                format!("`{name}` calls itself"),
                "a shader has no call stack; write the repetition as a loop",
            );
            return Some(Expr::poison());
        }

        let types: Vec<Type> = args.iter().map(Expr::ty).collect();
        let key = (name.to_owned(), types.clone());
        let (emitted, returns) = match self
            .instances
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, emitted)| emitted.clone())
        {
            Some(emitted) => {
                let returns = self
                    .lowered
                    .iter()
                    .find(|instance| instance.name == emitted)
                    .map_or(Type::Poison, |instance| instance.returns);
                (emitted, returns)
            }
            None => self.lower_helper(name, definition, &types, line)?,
        };
        Some(Expr::Call {
            builtin: Builtin::Helper,
            ty: returns,
            args: std::iter::once(Expr::Local {
                name: emitted,
                ty: returns,
            })
            .chain(args)
            .collect(),
        })
    }

    /// Lowers one helper for one set of argument types.
    fn lower_helper(
        &mut self,
        name: &str,
        definition: &luna::compiler::parser::FunctionDefinition<Name>,
        types: &[Type],
        line: u32,
    ) -> Option<(String, Type)> {
        let emitted = format!("morf_fn_{}_{}", sanitized(name), self.lowered.len());
        self.in_progress.push(name.to_owned());
        // A helper sees its own parameters and nothing else: no locals from the
        // caller, and no inputs the entry point happened to bind. Swapping the
        // scope stack out is what enforces that.
        let outer = std::mem::replace(&mut self.scopes, vec![std::collections::HashMap::new()]);
        let mut params = Vec::new();
        for (index, parameter) in definition.parameters.iter().enumerate() {
            let bound = self.declare(parameter, types[index]);
            params.push((bound, types[index]));
        }
        let mut body = self.block(&definition.body);
        body.resolve_mutability();
        self.scopes = outer;
        self.in_progress.pop();

        let returns = returned_type(&body).unwrap_or_else(|| {
            self.error(line, format!("`{name}` never returns a value"));
            Type::Poison
        });
        self.instances
            .push(((name.to_owned(), types.to_vec()), emitted.clone()));
        self.lowered.push(Instance {
            name: emitted.clone(),
            params,
            returns,
            body,
        });
        Some((emitted, returns))
    }
}

/// The type a body returns, if every `return` in it agrees.
fn returned_type(block: &Block) -> Option<Type> {
    let mut found = None;
    walk_returns(block, &mut found);
    found
}

fn walk_returns(block: &Block, found: &mut Option<Type>) {
    for statement in &block.0 {
        match statement {
            Stmt::Return(value) => {
                if found.is_none() {
                    *found = Some(value.ty());
                }
            }
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    walk_returns(body, found);
                }
                if let Some(body) = otherwise {
                    walk_returns(body, found);
                }
            }
            Stmt::Loop { body, .. } => walk_returns(body, found),
            Stmt::Let { .. } | Stmt::Assign { .. } | Stmt::Break => {}
        }
    }
}

/// A helper's name, made safe to build an identifier from.
fn sanitized(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}
