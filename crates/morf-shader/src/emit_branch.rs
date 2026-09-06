//! Branches, and the one place a `switch` can come from.
//!
//! Lua has no `switch` and no syntax to spell one, so it is recognised rather
//! than written: an `if` chain testing one whole number against constants is
//! the shape somebody writes anyway, and the driver gets a jump table instead
//! of a ladder of comparisons.

use std::fmt::Write as _;

use crate::emit::Emitter;
use crate::ir::*;
use crate::types::*;

/// One arm of a recognised switch: the constant, and what it runs.
type Case<'a> = (i64, &'a Block);

/// A recognised switch: what is tested, its cases, and where else to land.
type Switch<'a> = (&'a Expr, Vec<Case<'a>>, &'a Block);

impl Emitter {
    /// Whether an `if` chain is really a switch: every arm comparing one whole
    /// number against a constant, and an `else` to land in.
    ///
    /// Lua has no `switch` and no syntax to spell one, so this is where one can
    /// come from — the author writes the `if` chain they would have written
    /// anyway and the emitter recognises the shape. Nothing new to learn, and
    /// the driver gets a jump table instead of a ladder of comparisons.
    fn as_switch<'a>(
        arms: &'a [(Expr, Block)],
        otherwise: Option<&'a Block>,
    ) -> Option<Switch<'a>> {
        // Two arms is a plain `if`/`else`; a switch earns its keep from three.
        if arms.len() < 3 {
            return None;
        }
        let otherwise = otherwise?;
        let mut subject = None;
        let mut cases = Vec::with_capacity(arms.len());
        for (condition, body) in arms {
            let Expr::Binary {
                op: BinOp::Equal,
                left,
                right,
                ..
            } = condition
            else {
                return None;
            };
            let value = match right.as_ref() {
                Expr::Literal(Value::Int(value)) => *value,
                Expr::Literal(Value::I32(value)) => i64::from(*value),
                _ => return None,
            };
            if !left.ty().is_integer() {
                return None;
            }
            match &subject {
                None => subject = Some(left.as_ref()),
                // Every arm has to test the same thing, or it is a chain of
                // unrelated questions that happens to be written as one.
                Some(known) if *known == left.as_ref() => {}
                Some(_) => return None,
            }
            cases.push((value, body));
        }
        // A repeated case would be a WGSL error; in an `if` chain the first
        // simply wins, so the shape is not a switch after all.
        let mut seen: Vec<i64> = cases.iter().map(|(value, _)| *value).collect();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != cases.len() {
            return None;
        }
        subject.map(|subject| (subject, cases, otherwise))
    }

    pub(crate) fn branch(&mut self, arms: &[(Expr, Block)], otherwise: Option<&Block>) {
        if let Some((subject, cases, otherwise)) = Self::as_switch(arms, otherwise) {
            self.indent();
            self.out.push_str("switch (");
            self.expression(subject, subject.ty());
            self.out.push_str(") {\n");
            self.depth += 1;
            for (value, body) in cases {
                self.indent();
                let suffix = if subject.ty() == Type::U32 { "u" } else { "" };
                let _ = writeln!(self.out, "case {value}{suffix}: {{");
                self.depth += 1;
                self.block(body);
                self.depth -= 1;
                self.indent();
                self.out.push_str("}\n");
            }
            self.indent();
            self.out.push_str("default: {\n");
            self.depth += 1;
            self.block(otherwise);
            self.depth -= 1;
            self.indent();
            self.out.push_str("}\n");
            self.depth -= 1;
            self.indent();
            self.out.push_str("}\n");
            return;
        }
        for (index, (condition, body)) in arms.iter().enumerate() {
            self.indent();
            self.out
                .push_str(if index == 0 { "if (" } else { "} else if (" });
            self.expression(condition, Type::Bool);
            self.out.push_str(") {\n");
            self.depth += 1;
            self.block(body);
            self.depth -= 1;
        }
        if let Some(body) = otherwise {
            self.indent();
            self.out.push_str("} else {\n");
            self.depth += 1;
            self.block(body);
            self.depth -= 1;
        }
        self.indent();
        self.out.push_str("}\n");
    }
}
