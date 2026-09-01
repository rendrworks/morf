//! Operators and constructors: what may be combined with what.
//!
//! The type rules for arithmetic and comparison, and how a scalar rides along
//! with a vector. Bitwise operators live in `lower_bits`, because whole numbers
//! follow different rules and the two together crossed the line gate.

use luna::compiler::parser::{BinaryOperator, UnaryOperator};

use crate::builtins;
use crate::ir::*;
use crate::lower::Lowerer;
use crate::lower_bits::arithmetic_result;
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
                if !ty.is_integer() && !ty.is_poison() {
                    self.error_note(
                        line,
                        format!("`~` works on whole numbers, not {ty}"),
                        "convert first: `~u32(x)`",
                    );
                    return Expr::poison();
                }
                let ty = if ty == Type::AbstractInt {
                    Type::I32
                } else {
                    ty
                };
                Expr::Unary {
                    op: UnOp::BitNot,
                    ty,
                    value: Box::new(value),
                }
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
            BinaryOperator::BitAnd => BinOp::BitAnd,
            BinaryOperator::BitOr => BinOp::BitOr,
            BinaryOperator::BitXor => BinOp::BitXor,
            BinaryOperator::ShiftLeft => BinOp::ShiftLeft,
            BinaryOperator::ShiftRight => BinOp::ShiftRight,
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
        if op.is_bitwise() {
            return self.bitwise(op, left, right, line);
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
