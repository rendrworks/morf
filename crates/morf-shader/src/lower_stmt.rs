//! Statement lowering: the shapes a shader's body can take.
//!
//! Every loop here — `while`, numeric `for`, `repeat` — funnels through
//! `push_loop` into one `Stmt::Loop`, so the iteration guard that keeps a
//! configuration from hanging the GPU has a single place to live rather than
//! three that could drift apart.

use luna::compiler::parser::{Block as AstBlock, Expression, ForStatement, Statement};

use crate::ir::*;
use crate::limits::*;
use crate::lower::{Lowerer, Name, line_of, text};
use crate::types::*;

impl Lowerer<'_> {
    /// Lowers a block, opening a scope for it.
    pub(crate) fn block(&mut self, block: &AstBlock<Name>) -> Block {
        self.push_scope();
        let lowered = self.statements(block);
        self.pop_scope();
        lowered
    }

    fn statements(&mut self, block: &AstBlock<Name>) -> Block {
        let mut out = Vec::new();
        for statement in &block.statements {
            let line = line_of(statement.line_number);
            self.statement(&statement.inner, line, &mut out);
        }
        if let Some(returned) = &block.return_statement {
            let line = line_of(returned.line_number);
            match returned.returns.len() {
                1 => {
                    let value = self.expression(&returned.returns[0], line);
                    out.push(Stmt::Return(value));
                }
                0 => self.error(line, "a shader must return a value"),
                count => self.error(line, format!("a shader returns one value, not {count}")),
            }
        }
        Block(out)
    }

    fn statement(&mut self, statement: &Statement<Name>, line: u32, out: &mut Vec<Stmt>) {
        match statement {
            Statement::LocalStatement(local) => self.local(local, line, out),
            Statement::Assignment(assignment) => self.assignment(assignment, line, out),
            Statement::If(branch) => self.branch(branch, line, out),
            Statement::While(loop_) => self.while_loop(loop_, line, out),
            Statement::For(loop_) => self.for_loop(loop_, line, out),
            Statement::Repeat(loop_) => self.repeat_loop(loop_, line, out),
            Statement::Do(block) => {
                let body = self.block(block);
                out.extend(body.0);
            }
            Statement::Break => {
                if self.loop_depth == 0 {
                    self.error(line, "`break` outside a loop");
                } else {
                    out.push(Stmt::Break);
                }
            }
            // `discard()` is spelled as a call because Lua has no keyword to
            // spare, and it is the one call whose whole point is its effect.
            Statement::FunctionCall(call)
                if matches!(
                    &call.head.primary,
                    luna::compiler::parser::PrimaryExpression::Name(name)
                        if text(name) == "discard"
                ) =>
            {
                out.push(Stmt::Discard);
            }
            Statement::FunctionCall(_) => self.error_note(
                line,
                "a call on its own does nothing in a shader",
                "every value a shader computes has to reach the return,                  unless it is `discard()`",
            ),
            Statement::Function(_) | Statement::LocalFunction(_) => self.error_note(
                line,
                "a shader cannot define functions inside itself",
                "write the arithmetic where it is used",
            ),
            // Lua has no `continue`, and the idiom every Lua author already
            // writes is `goto continue` with a `::continue::` label at the end
            // of the loop body. That is real Lua syntax rather than something
            // invented here, so it is what a shader spells `continue` with.
            Statement::Goto(target) if text(&target.name) == "continue" => {
                if self.loop_depth == 0 {
                    self.error(line, "`goto continue` outside a loop");
                } else {
                    out.push(Stmt::Continue);
                }
            }
            // The label the idiom pairs with. WGSL's `continue` needs no
            // landing site, so it is accepted and does nothing.
            Statement::Label(label) if text(&label.name) == "continue" => {}
            Statement::Label(_) | Statement::Goto(_) => self.error_note(
                line,
                "`goto` is not available in shaders",
                "`goto continue` is, as the way Lua spells a loop continuation",
            ),
        }
    }

    fn local(
        &mut self,
        local: &luna::compiler::parser::LocalStatement<Name>,
        line: u32,
        out: &mut Vec<Stmt>,
    ) {
        if local.names.len() != local.values.len() {
            self.error_note(
                line,
                "every local in a shader needs its own value",
                "there is no `nil` to hold an empty one",
            );
            return;
        }
        for (index, (name, _)) in local.names.iter().enumerate() {
            // A local has to have a type, so an abstract literal decides here.
            let value = Lowerer::commit(self.expression(&local.values[index], line));
            let ty = value.ty();
            // Matrices are not "numeric" — `m * v` is a product, not a
            // componentwise multiply — but they are perfectly good values to
            // hold, which is a different question.
            if ty == Type::Bool
                || ty.is_numeric()
                || ty.is_any_vector()
                || ty.is_matrix()
                || ty.is_array()
                || ty == Type::Split
                || ty.is_record()
                || ty.is_poison()
            {
                let emitted = self.declare(name, ty);
                out.push(Stmt::Let {
                    name: emitted,
                    ty,
                    value,
                    mutable: false,
                });
            } else {
                self.error(line, format!("a shader local cannot hold {ty}"));
            }
        }
    }

    fn assignment(
        &mut self,
        assignment: &luna::compiler::parser::AssignmentStatement<Name>,
        line: u32,
        out: &mut Vec<Stmt>,
    ) {
        use luna::compiler::parser::AssignmentTarget;
        if assignment.targets.len() != assignment.values.len() {
            self.error(line, "a shader assigns one value per name");
            return;
        }
        for (index, target) in assignment.targets.iter().enumerate() {
            let AssignmentTarget::Name(name) = target else {
                self.error_note(
                    line,
                    "a shader cannot assign into a field",
                    "build a new vector instead: `v = vec3(v.x, 1.0, v.z)`",
                );
                continue;
            };
            let value = self.expression(&assignment.values[index], line);
            let Some(local) = self.lookup(name) else {
                self.error_note(
                    line,
                    format!("`{}` was never declared", text(name)),
                    "a shader has no globals; write `local` first",
                );
                continue;
            };
            let (declared, emitted) = (local.ty, local.emitted.clone());
            if !declared.is_poison() && !value.ty().is_poison() && value.ty() != declared {
                self.error(
                    line,
                    format!(
                        "`{}` holds {declared}, so it cannot take {}",
                        text(name),
                        value.ty()
                    ),
                );
                continue;
            }
            self.mark_mutable(name);
            out.push(Stmt::Assign {
                target: emitted,
                value,
            });
        }
    }

    fn branch(
        &mut self,
        branch: &luna::compiler::parser::IfStatement<Name>,
        line: u32,
        out: &mut Vec<Stmt>,
    ) {
        let mut arms = Vec::new();
        let condition = self.condition(&branch.if_part.0, line);
        arms.push((condition, self.block(&branch.if_part.1)));
        for (expression, block) in &branch.else_if_parts {
            let condition = self.condition(expression, line);
            arms.push((condition, self.block(block)));
        }
        let otherwise = branch.else_part.as_ref().map(|block| self.block(block));
        out.push(Stmt::If { arms, otherwise });
    }

    /// Lowers a condition, insisting it is a `bool`.
    ///
    /// This is the most important diagnostic in the compiler. Lua treats every
    /// value but `nil` and `false` as true, so a configuration author will
    /// write `if x then` meaning `if x > 0.0 then` — and a shader has no
    /// truthiness to give them. Coercing quietly would produce a wrong image
    /// with no error at all, so it is refused, by name, with the fix.
    pub(crate) fn condition(&mut self, expression: &Expression<Name>, line: u32) -> Expr {
        let value = self.expression(expression, line);
        let ty = value.ty();
        if ty == Type::Bool || ty.is_poison() {
            return value;
        }
        self.error_note(
            line,
            format!("a shader condition must be a bool, not {ty}"),
            "shaders have no truthiness: write a comparison, like `x > 0.0`",
        );
        Expr::Literal(Value::Bool(true))
    }

    fn while_loop(
        &mut self,
        loop_: &luna::compiler::parser::WhileStatement<Name>,
        line: u32,
        out: &mut Vec<Stmt>,
    ) {
        let condition = self.condition(&loop_.condition, line);
        self.loop_depth += 1;
        let mut body = vec![Stmt::If {
            arms: vec![(
                Expr::Unary {
                    op: UnOp::Not,
                    ty: Type::Bool,
                    value: Box::new(condition),
                },
                Block(vec![Stmt::Break]),
            )],
            otherwise: None,
        }];
        body.extend(self.block(&loop_.block).0);
        self.loop_depth -= 1;
        self.push_loop(out, MAX_ITERATIONS, Block(body), Block::default(), line);
    }

    fn repeat_loop(
        &mut self,
        loop_: &luna::compiler::parser::RepeatStatement<Name>,
        line: u32,
        out: &mut Vec<Stmt>,
    ) {
        self.loop_depth += 1;
        // `repeat` shares the body's scope with its condition, so the block and
        // the `until` are lowered together rather than through `block`.
        self.push_scope();
        let mut body = self.statements(&loop_.body).0;
        let condition = self.condition(&loop_.until, line);
        self.pop_scope();
        self.loop_depth -= 1;
        body.push(Stmt::If {
            arms: vec![(condition, Block(vec![Stmt::Break]))],
            otherwise: None,
        });
        self.push_loop(out, MAX_ITERATIONS, Block(body), Block::default(), line);
    }

    fn for_loop(&mut self, loop_: &ForStatement<Name>, line: u32, out: &mut Vec<Stmt>) {
        let ForStatement::Numeric {
            name,
            initial,
            limit,
            step,
            body,
        } = loop_
        else {
            self.error_note(
                line,
                "a shader has no `for ... in`",
                "count with `for i = 1, n do`",
            );
            return;
        };
        let initial = self.numeric(initial, line, "a loop start");
        let limit = self.numeric(limit, line, "a loop limit");
        // Lua evaluates the step once and its sign decides the comparison.
        // Reproducing that faithfully needs a runtime branch; requiring a
        // constant keeps the emitted loop honest and the surface small.
        let (increment, descending) = match step {
            None => (Expr::Literal(Value::F32(1.0)), false),
            Some(expression) => match self.constant(expression, line) {
                Some(value) if value != 0.0 => (Expr::Literal(Value::F32(value)), value < 0.0),
                Some(_) => {
                    self.error(line, "a loop step cannot be zero");
                    return;
                }
                None => {
                    self.error_note(
                        line,
                        "a loop step must be a constant",
                        "its sign decides which way the loop counts",
                    );
                    return;
                }
            },
        };
        self.push_scope();
        let counter = self.declare(name, Type::F32);
        self.loop_depth += 1;
        let inner = self.block(body);
        self.loop_depth -= 1;
        self.pop_scope();
        let test = Expr::Binary {
            op: if descending {
                BinOp::Less
            } else {
                BinOp::Greater
            },
            ty: Type::Bool,
            left: Box::new(Expr::Local {
                name: counter.clone(),
                ty: Type::F32,
            }),
            right: Box::new(limit),
        };
        let mut statements = vec![Stmt::If {
            arms: vec![(test, Block(vec![Stmt::Break]))],
            otherwise: None,
        }];
        statements.extend(inner.0);
        // The counter advances in the `continuing` block, not at the end of the
        // body: a `continue` jumps past the body's tail, and an increment left
        // there would turn a counting loop into one that never advances.
        let advance = Block(vec![Stmt::Assign {
            target: counter.clone(),
            value: Expr::Binary {
                op: BinOp::Add,
                ty: Type::F32,
                left: Box::new(Expr::Local {
                    name: counter.clone(),
                    ty: Type::F32,
                }),
                right: Box::new(increment),
            },
        }]);
        out.push(Stmt::Let {
            name: counter,
            ty: Type::F32,
            value: initial,
            mutable: true,
        });
        self.push_loop(out, MAX_ITERATIONS, Block(statements), advance, line);
    }

    /// Emits a loop, rejecting a nest deeper than the guard can bound.
    fn push_loop(
        &mut self,
        out: &mut Vec<Stmt>,
        guard: u32,
        body: Block,
        continuing: Block,
        line: u32,
    ) {
        if self.loop_depth >= MAX_LOOP_NESTING {
            self.error(line, format!("loops nested deeper than {MAX_LOOP_NESTING}"));
            return;
        }
        out.push(Stmt::Loop {
            guard,
            body,
            continuing,
        });
    }

    fn numeric(&mut self, expression: &Expression<Name>, line: u32, what: &str) -> Expr {
        let value = Lowerer::commit(self.expression(expression, line));
        let ty = value.ty();
        if ty == Type::F32 || ty == Type::I32 || ty.is_poison() {
            return value;
        }
        self.error(line, format!("{what} must be a number, not {ty}"));
        Expr::Literal(Value::F32(0.0))
    }
}
