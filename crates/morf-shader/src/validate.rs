use crate::diagnostics::Diagnostic;
use crate::ir::*;
use crate::types::*;
use crate::{ShaderKind, ShaderSpec};

/// Checks what lowering could not check locally.
///
/// Two things need the whole program in view: that every path returns, and that
/// no texture is sampled under non-uniform control flow.
pub(crate) fn check(program: &Program, spec: &ShaderSpec, diagnostics: &mut Vec<Diagnostic>) {
    returned_types(&program.entry.body, diagnostics);
    if !returns(&program.entry.body) {
        diagnostics.push(
            Diagnostic::new(1, "this shader can finish without returning a colour")
                .note("every branch needs its own `return`, or move one to the end"),
        );
    }
    if program.samples_behind && spec.kind != ShaderKind::Effect {
        diagnostics.push(
            Diagnostic::new(1, "only an effect shader can read what is underneath").note(format!(
                "this one is `{}`; declare `kind = \"effect\"`",
                spec.kind.name()
            )),
        );
    }
    // WGSL forbids sampling under non-uniform control flow, and naga's own
    // message for it is close to unreadable. Catching it here costs a walk and
    // buys a diagnostic that names the rule.
    if program.samples_behind {
        let mut offender = false;
        sampling_under_branch(&program.entry.body, false, &mut offender);
        if offender {
            diagnostics.push(
                Diagnostic::new(1, "`texture` cannot be called inside an `if` or a loop")
                    .note("sample first into a local, then branch on the result"),
            );
        }
    }
}

/// Whether every path through a block ends in a `return`.
fn returns(block: &Block) -> bool {
    block.0.iter().any(|statement| match statement {
        Stmt::Return(_) => true,
        Stmt::If { arms, otherwise } => {
            // A branch only settles the question when it has an else: without
            // one, falling through is a path that reaches the end.
            otherwise.as_ref().is_some_and(returns) && arms.iter().all(|(_, body)| returns(body))
        }
        _ => false,
    })
}

fn sampling_under_branch(block: &Block, inside: bool, offender: &mut bool) {
    for statement in &block.0 {
        match statement {
            Stmt::If { arms, otherwise } => {
                for (condition, body) in arms {
                    if inside && samples(condition) {
                        *offender = true;
                    }
                    sampling_under_branch(body, true, offender);
                }
                if let Some(body) = otherwise {
                    sampling_under_branch(body, true, offender);
                }
            }
            Stmt::Loop { body, .. } => sampling_under_branch(body, true, offender),
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Return(value) => {
                if inside && samples(value) {
                    *offender = true;
                }
            }
            Stmt::Break => {}
        }
    }
}

fn samples(expression: &Expr) -> bool {
    match expression {
        Expr::Call { builtin, args, .. } => {
            *builtin == Builtin::Texture || args.iter().any(samples)
        }
        Expr::Unary { value, .. } | Expr::Swizzle { value, .. } => samples(value),
        Expr::Binary { left, right, .. } => samples(left) || samples(right),
        Expr::Construct { args, .. } => args.iter().any(samples),
        Expr::Literal(_) | Expr::Local { .. } | Expr::Param { .. } | Expr::Input { .. } => false,
    }
}

/// Reports every `return` whose value is not a colour.
///
/// Checked over the finished program rather than at each return, so a shader
/// with three branches gets three messages instead of stopping at the first.
fn returned_types(block: &Block, diagnostics: &mut Vec<Diagnostic>) {
    for statement in &block.0 {
        match statement {
            Stmt::Return(value) => {
                let ty = value.ty();
                if ty != Type::Vec4 && !ty.is_poison() {
                    diagnostics.push(
                        Diagnostic::new(1, format!("a shader returns a vec4 colour, not {ty}"))
                            .note("widen it: `vec4(value, 1.0)`"),
                    );
                }
            }
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    returned_types(body, diagnostics);
                }
                if let Some(body) = otherwise {
                    returned_types(body, diagnostics);
                }
            }
            Stmt::Loop { body, .. } => returned_types(body, diagnostics),
            Stmt::Let { .. } | Stmt::Assign { .. } | Stmt::Break => {}
        }
    }
}
