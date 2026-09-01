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
    // WGSL forbids both sampling and derivatives under non-uniform control
    // flow — they read the neighbouring pixels, which have to have taken the
    // same path — and naga's own message for it is close to unreadable.
    // Catching it here costs a walk and buys a diagnostic that names the rule.
    //
    // One check for both, because it is one rule. A check per builtin is how
    // the two would drift.
    if program.samples_behind || program.takes_derivative {
        let mut offender = None;
        uniformity(&program.entry.body, false, &mut offender);
        if let Some(name) = offender {
            diagnostics.push(
                Diagnostic::new(
                    1,
                    format!("`{name}` cannot be called inside an `if` or a loop"),
                )
                .note(
                    "it reads the neighbouring pixels, which have to have taken the                      same path; compute it first, then branch on the result",
                ),
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

fn uniformity(block: &Block, inside: bool, offender: &mut Option<&'static str>) {
    for statement in &block.0 {
        match statement {
            Stmt::If { arms, otherwise } => {
                for (condition, body) in arms {
                    if inside && let Some(name) = non_uniform(condition) {
                        *offender = Some(name);
                    }
                    uniformity(body, true, offender);
                }
                if let Some(body) = otherwise {
                    uniformity(body, true, offender);
                }
            }
            Stmt::Loop { body, .. } => uniformity(body, true, offender),
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } | Stmt::Return(value) => {
                if inside && let Some(name) = non_uniform(value) {
                    *offender = Some(name);
                }
            }
            Stmt::Break => {}
        }
    }
}

/// The name of the first call in this expression that needs uniform control
/// flow, if there is one.
fn non_uniform(expression: &Expr) -> Option<&'static str> {
    match expression {
        Expr::Call { builtin, args, .. } => {
            let own = match builtin {
                Builtin::Texture => Some("texture"),
                Builtin::Dpdx => Some("dpdx"),
                Builtin::Dpdy => Some("dpdy"),
                Builtin::Fwidth => Some("fwidth"),
                _ => None,
            };
            own.or_else(|| args.iter().find_map(non_uniform))
        }
        Expr::Unary { value, .. } | Expr::Swizzle { value, .. } => non_uniform(value),
        Expr::Binary { left, right, .. } => non_uniform(left).or_else(|| non_uniform(right)),
        Expr::Construct { args, .. } | Expr::Array { elements: args, .. } => {
            args.iter().find_map(non_uniform)
        }
        Expr::Index { value, index, .. } => non_uniform(value).or_else(|| non_uniform(index)),
        Expr::Literal(_) | Expr::Local { .. } | Expr::Param { .. } | Expr::Input { .. } => None,
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
