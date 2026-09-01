//! Operators, constructors and constant folding.
//!
//! Where the type rules live: what may be added to what, how a scalar rides
//! along with a vector, and which spellings of a comparison a shader is allowed
//! to write at all.

use luna::compiler::parser::{BinaryOperator, Expression, UnaryOperator};

use crate::builtins;
use crate::ir::*;
use crate::lower::{Lowerer, Name};
use crate::types::*;

impl Lowerer<'_> {
    pub(crate) fn unary(&mut self, operator: UnaryOperator, value: Expr, line: u32) -> Expr {
        let ty = value.ty();
        match operator {
            UnaryOperator::Minus => {
                if !ty.is_numeric() && !ty.is_poison() {
                    self.error(line, format!("cannot negate {ty}"));
                    return Expr::poison();
                }
                Expr::Unary {
                    op: UnOp::Negate,
                    ty,
                    value: Box::new(value),
                }
            }
            UnaryOperator::Not => {
                if ty != Type::Bool && !ty.is_poison() {
                    self.error_note(
                        line,
                        format!("`not` needs a bool, not {ty}"),
                        "shaders have no truthiness: compare first",
                    );
                    return Expr::poison();
                }
                Expr::Unary {
                    op: UnOp::Not,
                    ty: Type::Bool,
                    value: Box::new(value),
                }
            }
            UnaryOperator::Len => {
                self.error_note(line, "`#` is not available", "use `length(v)`");
                Expr::poison()
            }
            UnaryOperator::BitNot => {
                self.error(line, "a shader has no bitwise operators");
                Expr::poison()
            }
        }
    }

    pub(crate) fn binary(
        &mut self,
        operator: BinaryOperator,
        left: Expr,
        right: Expr,
        line: u32,
    ) -> Expr {
        if !self.charge(line) {
            return Expr::poison();
        }
        let op = match operator {
            BinaryOperator::Add => BinOp::Add,
            BinaryOperator::Sub => BinOp::Sub,
            BinaryOperator::Mul => BinOp::Mul,
            BinaryOperator::Div => BinOp::Div,
            BinaryOperator::Mod => BinOp::Mod,
            BinaryOperator::Equal => BinOp::Equal,
            BinaryOperator::NotEqual => BinOp::NotEqual,
            BinaryOperator::LessThan => BinOp::Less,
            BinaryOperator::LessEqual => BinOp::LessEqual,
            BinaryOperator::GreaterThan => BinOp::Greater,
            BinaryOperator::GreaterEqual => BinOp::GreaterEqual,
            BinaryOperator::And => BinOp::And,
            BinaryOperator::Or => BinOp::Or,
            BinaryOperator::Pow => {
                return self.power(left, right, line);
            }
            BinaryOperator::IDiv => {
                return self.floor_div(left, right, line);
            }
            BinaryOperator::Concat => {
                self.error(line, "a shader has no string concatenation");
                return Expr::poison();
            }
            BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight => {
                self.error(line, "a shader has no bitwise operators");
                return Expr::poison();
            }
        };
        let (left_ty, right_ty) = (left.ty(), right.ty());
        if left_ty.is_poison() || right_ty.is_poison() {
            return Expr::poison();
        }
        if op.is_logical() {
            if left_ty != Type::Bool || right_ty != Type::Bool {
                self.error_note(
                    line,
                    format!("`and`/`or` need bools, not {left_ty} and {right_ty}"),
                    "a shader cannot use `a or b` to pick a value; use `select(b, a, cond)`",
                );
                return Expr::poison();
            }
            return Expr::Binary {
                op,
                ty: Type::Bool,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        if op.is_comparison() {
            if left_ty != right_ty || left_ty.is_vector() {
                self.error(line, format!("cannot compare {left_ty} with {right_ty}"));
                return Expr::poison();
            }
            return Expr::Binary {
                op,
                ty: Type::Bool,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        let Some(ty) = arithmetic_result(op, left_ty, right_ty) else {
            self.error(
                line,
                format!(
                    "cannot {} {left_ty} and {right_ty}",
                    match op {
                        BinOp::Add => "add",
                        BinOp::Sub => "subtract",
                        BinOp::Mul => "multiply",
                        BinOp::Div => "divide",
                        _ => "combine",
                    }
                ),
            );
            return Expr::poison();
        };
        Expr::Binary {
            op,
            ty,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn power(&mut self, left: Expr, right: Expr, line: u32) -> Expr {
        let types = [left.ty(), right.ty()];
        match builtins::resolve("^", builtins::Shape::Componentwise2, &types) {
            Ok(ty) => Expr::Call {
                builtin: Builtin::Pow,
                ty,
                args: vec![left, right],
            },
            Err(message) => {
                self.error(line, message);
                Expr::poison()
            }
        }
    }

    fn floor_div(&mut self, left: Expr, right: Expr, line: u32) -> Expr {
        let (left_ty, right_ty) = (left.ty(), right.ty());
        let Some(ty) = arithmetic_result(BinOp::Div, left_ty, right_ty) else {
            self.error(line, format!("cannot divide {left_ty} and {right_ty}"));
            return Expr::poison();
        };
        Expr::Call {
            builtin: Builtin::FloorDiv,
            ty,
            args: vec![Expr::Binary {
                op: BinOp::Div,
                ty,
                left: Box::new(left),
                right: Box::new(right),
            }],
        }
    }
}

/// The result type of componentwise arithmetic, with scalar broadcast.
///
/// A scalar rides along with a vector for every operator, not just `*` and `/`.
/// WGSL only defines the multiplicative pair that way, so `v + 1.0` is widened
/// during emission — but a configuration author means the obvious thing by it,
/// and refusing would be pedantry.
fn arithmetic_result(_op: BinOp, left: Type, right: Type) -> Option<Type> {
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
