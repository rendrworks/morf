use std::collections::HashMap;

use luna::compiler::parser::{Chunk, FunctionDefinition, Statement};

use crate::diagnostics::Diagnostic;
use crate::ir::*;
use crate::limits::*;
use crate::types::*;

/// The interned string type Luna's parser produces.
pub(crate) type Name = std::rc::Rc<[u8]>;

/// Luna counts lines from zero and a configuration author counts from one.
///
/// Converted at the two places a line enters the compiler rather than at the
/// dozens where one is reported, so a diagnostic can never be off by one.
pub(crate) fn line_of(line: luna::compiler::lexer::LineNumber) -> u32 {
    line.0 as u32 + 1
}

pub(crate) fn text(name: &Name) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// One binding in scope, and whether it is ever written to.
pub(crate) struct Local {
    pub(crate) ty: Type,
    pub(crate) emitted: String,
    pub(crate) mutable: bool,
}

pub(crate) struct Lowerer<'a> {
    pub(crate) scopes: Vec<HashMap<Vec<u8>, Local>>,
    pub(crate) inputs: &'a [Binding],
    pub(crate) params: &'a [Binding],
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) loop_depth: u32,
    pub(crate) nodes: u32,
    pub(crate) scope_id: u32,
    pub(crate) reads_time: bool,
    pub(crate) samples_behind: bool,
}

impl<'a> Lowerer<'a> {
    pub(crate) fn new(inputs: &'a [Binding], params: &'a [Binding]) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            inputs,
            params,
            diagnostics: Vec::new(),
            loop_depth: 0,
            nodes: 0,
            scope_id: 0,
            reads_time: false,
            samples_behind: false,
        }
    }

    pub(crate) fn error(&mut self, line: u32, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(line, message));
    }

    pub(crate) fn error_note(
        &mut self,
        line: u32,
        message: impl Into<String>,
        note: impl Into<String>,
    ) {
        self.diagnostics
            .push(Diagnostic::new(line, message).note(note));
    }

    /// Counts one IR node, reporting once when the budget runs out.
    ///
    /// The cap exists because shader compilation is superlinear in program
    /// size: a configuration that generates a million nodes would not fail, it
    /// would simply stall the session for a very long time.
    pub(crate) fn charge(&mut self, line: u32) -> bool {
        self.nodes += 1;
        if self.nodes == MAX_IR_NODES + 1 {
            self.error(
                line,
                format!("shader is too large: more than {MAX_IR_NODES} operations"),
            );
        }
        self.nodes <= MAX_IR_NODES
    }

    pub(crate) fn push_scope(&mut self) {
        self.scope_id += 1;
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn lookup(&self, name: &[u8]) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Declares a local, shadowing any outer one as Lua does.
    ///
    /// The emitted name carries the scope number so WGSL — which has no
    /// shadowing — still sees a unique identifier for each.
    pub(crate) fn declare(&mut self, name: &Name, ty: Type) -> String {
        let emitted = format!("{}_{}", sanitize(name), self.scope_id);
        let local = Local {
            ty,
            emitted: emitted.clone(),
            mutable: false,
        };
        self.scopes
            .last_mut()
            .expect("a scope is always open")
            .insert(name.to_vec(), local);
        emitted
    }

    pub(crate) fn mark_mutable(&mut self, name: &[u8]) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(local) = scope.get_mut(name) {
                local.mutable = true;
                return;
            }
        }
    }
}

/// Makes a Lua name safe to emit, and keeps it away from our own prefixes.
fn sanitize(name: &Name) -> String {
    let mut out = String::with_capacity(name.len() + 1);
    for byte in name.iter() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => out.push(*byte as char),
            _ => out.push('_'),
        }
    }
    if out.starts_with("morf") || out.starts_with('_') {
        out.insert(0, 'u');
    }
    out
}

/// Finds the entry function in a parsed chunk.
///
/// A shader is one function. The chunk may declare it as `function name(...)`
/// or `local function name(...)`; anything else at the top level is a
/// diagnostic, because a shader has no host to run statements against.
pub(crate) fn entry_function<'c>(
    chunk: &'c Chunk<Name>,
    wanted: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'c FunctionDefinition<Name>> {
    let mut found = None;
    for statement in &chunk.block.statements {
        let line = line_of(statement.line_number);
        match &statement.inner {
            Statement::Function(function) if function.fields.is_empty() => {
                if text(&function.name) == wanted {
                    found = Some(&function.definition);
                }
            }
            Statement::LocalFunction(function) => {
                if text(&function.name) == wanted {
                    found = Some(&function.definition);
                }
            }
            _ => diagnostics.push(
                Diagnostic::new(line, "a shader holds one function and nothing else")
                    .note(format!("move this inside `function {wanted}(...)`")),
            ),
        }
    }
    if found.is_none() {
        diagnostics.push(Diagnostic::new(
            1,
            format!("no `function {wanted}(...)` in this shader"),
        ));
    }
    found
}
