use luna::compiler::parser::{
    CallSuffix, Expression, FieldSuffix, HeadExpression, PrimaryExpression, SimpleExpression,
    SuffixPart, SuffixedExpression,
};

use crate::builtins;
use crate::ir::*;
use crate::lower::{Lowerer, Name, text};
use crate::types::*;

/// Where texture and data bindings start in the input index space.
///
/// They share `Expr::Input` with the frame's own values because they are the
/// same kind of thing — something the host supplies — and separating them into
/// their own expression would mean four more match arms saying nothing.
pub(crate) const TEXTURE_BASE: usize = 1000;
pub(crate) const DATA_BASE: usize = 2000;

impl Lowerer<'_> {
    /// Lowers an expression.
    ///
    /// `Expression` is a head plus a tail of `(operator, right)`, but the
    /// parser has *already* resolved precedence by nesting higher-precedence
    /// subexpressions into each tail entry's right-hand side. A plain left fold
    /// is therefore correct, and it is exactly what Luna's own compiler does.
    /// Writing a precedence climber here would produce the wrong tree.
    pub(crate) fn expression(&mut self, expression: &Expression<Name>, line: u32) -> Expr {
        let mut left = self.head(&expression.head, line);
        for (operator, right) in &expression.tail {
            let right = self.expression(right, line);
            left = self.binary(*operator, left, right, line);
        }
        left
    }

    fn head(&mut self, head: &HeadExpression<Name>, line: u32) -> Expr {
        match head {
            HeadExpression::Simple(simple) => self.simple(simple, line),
            HeadExpression::UnaryOperator(operator, value) => {
                let value = self.expression(value, line);
                self.unary(*operator, value, line)
            }
        }
    }

    fn simple(&mut self, simple: &SimpleExpression<Name>, line: u32) -> Expr {
        if !self.charge(line) {
            return Expr::poison();
        }
        match simple {
            SimpleExpression::Float(value) => Expr::Literal(Value::F32(*value as f32)),
            // An integer literal is abstract until something decides. `1 / 2`
            // still ends up `0.5`, because nothing in it asks for an integer
            // and the default is `f32` — but `1 << 2` can now be four.
            SimpleExpression::Integer(value) => Expr::Literal(Value::Int(*value)),
            SimpleExpression::True => Expr::Literal(Value::Bool(true)),
            SimpleExpression::False => Expr::Literal(Value::Bool(false)),
            SimpleExpression::Suffixed(suffixed) => self.suffixed(suffixed, line),
            SimpleExpression::Nil => {
                self.error_note(line, "a shader has no `nil`", "every value is a number");
                Expr::poison()
            }
            SimpleExpression::String(_) => {
                self.error(line, "a shader has no strings");
                Expr::poison()
            }
            // A Lua list becomes a fixed-length array, which is what it looks
            // like and what a palette or a convolution kernel wants to be.
            SimpleExpression::TableConstructor(table) => self.array(table, line),
            SimpleExpression::Function(_) => {
                self.error(line, "a shader cannot define functions inside itself");
                Expr::poison()
            }
            SimpleExpression::VarArgs => {
                self.error(line, "a shader has no `...`");
                Expr::poison()
            }
        }
    }

    /// A name, then any run of field accesses and calls after it.
    fn suffixed(&mut self, suffixed: &SuffixedExpression<Name>, line: u32) -> Expr {
        let PrimaryExpression::Name(head) = &suffixed.primary else {
            let PrimaryExpression::GroupedExpression(inner) = &suffixed.primary else {
                unreachable!("a primary expression is a name or a group");
            };
            let mut value = self.expression(inner, line);
            for suffix in &suffixed.suffixes {
                value = self.suffix(value, suffix, line, "");
            }
            return value;
        };

        // A call whose head is a plain name is a builtin: `sin(x)`. A call
        // whose head is `math` is the same builtin under its other spelling.
        if let Some(SuffixPart::Call(call)) = suffixed.suffixes.first() {
            let name = text(head);
            let value = self.call(&name, call, line);
            return self.trailing(value, &suffixed.suffixes[1..], line, &name);
        }
        if let (Some(SuffixPart::Field(FieldSuffix::Named(field))), Some(SuffixPart::Call(call))) =
            (suffixed.suffixes.first(), suffixed.suffixes.get(1))
            && text(head) == "math"
        {
            let name = format!("math.{}", text(field));
            let value = self.call(&name, call, line);
            return self.trailing(value, &suffixed.suffixes[2..], line, &name);
        }
        if text(head) == "math"
            && let Some(SuffixPart::Field(FieldSuffix::Named(field))) = suffixed.suffixes.first()
        {
            let constant = match text(field).as_str() {
                "pi" => Some(std::f32::consts::PI),
                "huge" => Some(f32::MAX),
                _ => None,
            };
            if let Some(constant) = constant {
                let value = Expr::Literal(Value::F32(constant));
                return self.trailing(value, &suffixed.suffixes[1..], line, "math");
            }
        }

        let name = text(head);
        let value = self.name(head, &name, line);
        self.trailing(value, &suffixed.suffixes, line, &name)
    }

    fn trailing(
        &mut self,
        mut value: Expr,
        suffixes: &[SuffixPart<Name>],
        line: u32,
        what: &str,
    ) -> Expr {
        for suffix in suffixes {
            value = self.suffix(value, suffix, line, what);
        }
        value
    }

    fn suffix(&mut self, value: Expr, suffix: &SuffixPart<Name>, line: u32, what: &str) -> Expr {
        match suffix {
            SuffixPart::Field(FieldSuffix::Named(field)) => self.swizzle(value, &text(field), line),
            SuffixPart::Field(FieldSuffix::Indexed(index)) => {
                let index = self.expression(index, line);
                self.index(value, index, line)
            }
            SuffixPart::Call(_) => {
                self.error(line, format!("`{what}` is not a function"));
                Expr::poison()
            }
        }
    }

    /// Resolves a bare name to an input, a parameter or a local.
    fn name(&mut self, raw: &Name, name: &str, line: u32) -> Expr {
        if let Some(local) = self.lookup(raw) {
            return Expr::Local {
                name: local.emitted.clone(),
                ty: local.ty,
            };
        }
        if let Some(index) = self.inputs.iter().position(|input| input.name == name) {
            if name == "time" {
                // Recorded here, where it cannot be forgotten, because this
                // flag decides whether the node repaints every frame forever.
                self.reads_time = true;
            }
            return Expr::Input {
                index,
                ty: self.inputs[index].ty,
            };
        }
        if let Some(index) = self.textures.iter().position(|texture| texture == name) {
            return Expr::Input {
                index: TEXTURE_BASE + index,
                ty: Type::Texture,
            };
        }
        if let Some(index) = self.data.iter().position(|(block, ..)| block == name) {
            let (_, element, length) = self.data[index];
            return Expr::Input {
                index: DATA_BASE + index,
                ty: Type::data(element, length),
            };
        }
        if let Some(index) = self.params.iter().position(|param| param.name == name) {
            return Expr::Param {
                index,
                ty: self.params[index].ty,
            };
        }
        // A constructor is a name too, but only ever as a call, so reaching
        // here with one means it was used as a value.
        let note = if Type::parse(name).is_some() {
            format!("`{name}` builds a value: write `{name}(...)`")
        } else {
            "a shader sees only its inputs, its params and its own locals".to_owned()
        };
        self.error_note(line, format!("`{name}` is not defined here"), note);
        Expr::poison()
    }

    fn call(&mut self, name: &str, call: &CallSuffix<Name>, line: u32) -> Expr {
        let CallSuffix::Function(arguments) = call else {
            self.error_note(
                line,
                "a shader has no methods",
                "call it as a function: `length(v)`",
            );
            return Expr::poison();
        };
        let lowered: Vec<Expr> = arguments
            .iter()
            .map(|argument| self.expression(argument, line))
            .collect();
        if !self.charge(line) {
            return Expr::poison();
        }
        if let Some(ty) = Type::parse(name).filter(|ty| ty.is_any_vector()) {
            return self.construct(ty, lowered, line);
        }
        if let Some(ty) = Type::parse(name).filter(|ty| ty.is_matrix()) {
            return self.construct_matrix(ty, lowered, line);
        }
        // `f32(x)`, `i32(x)`, `u32(x)` convert the value; `bitcast_u32(x)` and
        // friends reinterpret the bits, which is where every hash starts.
        if let Some(converted) = self.conversion(name, &lowered, line) {
            return converted;
        }
        // A helper the shader declared wins over nothing at all, but never over
        // a builtin: shadowing `sin` would be a trap, not a feature.
        if builtins::lookup(name).is_none()
            && let Some(call) = self.helper_call(name, lowered.clone(), line)
        {
            return call;
        }
        let Some((builtin, shape)) = builtins::lookup(name) else {
            self.error_note(
                line,
                format!("`{name}` is not a shader function"),
                format!("available: {}", builtins::available()),
            );
            return Expr::poison();
        };
        let arity = builtins::arity(shape);
        // `texture` is the one builtin whose arity varies: one argument samples
        // what is underneath, two sample a declared texture.
        let arity_ok = if builtin == Builtin::Texture {
            lowered.len() == 1 || lowered.len() == 2
        } else {
            lowered.len() == arity
        };
        if !arity_ok {
            self.error(
                line,
                format!(
                    "{name} takes {arity} argument{}, not {}",
                    if arity == 1 { "" } else { "s" },
                    lowered.len()
                ),
            );
            return Expr::poison();
        }
        // Only the one-argument form reads what is underneath; sampling a
        // declared texture does not make a node into a layer.
        if builtin == Builtin::Texture && lowered.len() == 1 {
            self.samples_behind = true;
        }
        if matches!(
            builtin,
            Builtin::Dpdx
                | Builtin::DpdxCoarse
                | Builtin::DpdxFine
                | Builtin::Dpdy
                | Builtin::DpdyCoarse
                | Builtin::DpdyFine
                | Builtin::Fwidth
                | Builtin::FwidthCoarse
                | Builtin::FwidthFine
        ) {
            self.takes_derivative = true;
        }
        let types: Vec<Type> = lowered.iter().map(Expr::ty).collect();
        match builtins::resolve(name, shape, &types) {
            Ok(ty) => Expr::Call {
                builtin,
                ty,
                args: lowered,
            },
            Err(message) => {
                self.error(line, message);
                Expr::poison()
            }
        }
    }

    /// `vec3(...)`, with the scalar broadcast Lua authors expect.
    /// `v.x`, `v.xy`, `v.rgb` — component selection.
    fn swizzle(&mut self, value: Expr, field: &str, line: u32) -> Expr {
        let source = value.ty();
        if source.is_poison() {
            return value;
        }
        // The only thing anybody does with a `modf` or `frexp` result is read
        // one of its two parts, so that is the only thing the type supports.
        // A data block is read by index, not by name, and a texture is not a
        // value at all — both are worth saying plainly rather than falling
        // through to "has no components".
        if source == Type::Texture {
            self.error_note(
                line,
                "a texture is not a value",
                "sample it: `texture(name, uv)`",
            );
            return Expr::poison();
        }
        if let Some(element) = source.field(field) {
            return Expr::Call {
                builtin: Builtin::ResultField,
                ty: element,
                args: vec![
                    value,
                    Expr::Local {
                        name: field.to_owned(),
                        ty: element,
                    },
                ],
            };
        }
        if source.is_record() {
            self.error_note(
                line,
                format!("{source} has no `{field}`"),
                "a record has only the fields it was written with",
            );
            return Expr::poison();
        }
        if source == Type::Split {
            let element = match field {
                "fract" | "whole" => Type::F32,
                "exp" => Type::I32,
                other => {
                    self.error_note(
                        line,
                        format!("a split result has no `{other}`"),
                        "`modf` gives `.fract` and `.whole`; `frexp` gives `.fract` and `.exp`",
                    );
                    return Expr::poison();
                }
            };
            return Expr::Call {
                builtin: Builtin::ResultField,
                ty: element,
                args: vec![
                    value,
                    Expr::Local {
                        name: field.to_owned(),
                        ty: element,
                    },
                ],
            };
        }
        if !source.is_any_vector() {
            self.error(line, format!("{source} has no components to select"));
            return Expr::poison();
        }
        if field.is_empty() || field.len() > 4 {
            self.error(line, format!("`{field}` is not a component selection"));
            return Expr::poison();
        }
        let mut components = [0u8; 4];
        let mut position_set = false;
        let mut colour_set = false;
        for (index, character) in field.chars().enumerate() {
            let (slot, colour) = match character {
                'x' => (0, false),
                'y' => (1, false),
                'z' => (2, false),
                'w' => (3, false),
                'r' => (0, true),
                'g' => (1, true),
                'b' => (2, true),
                'a' => (3, true),
                _ => {
                    self.error(line, format!("`{field}` is not a component selection"));
                    return Expr::poison();
                }
            };
            // WGSL forbids it and so do we: `v.xg` reads as a typo, and
            // accepting it would hide one.
            if colour {
                colour_set = true;
            } else {
                position_set = true;
            }
            if position_set && colour_set {
                self.error_note(
                    line,
                    format!("`{field}` mixes xyzw with rgba"),
                    "pick one set and stay in it",
                );
                return Expr::poison();
            }
            if slot >= source.components() {
                self.error(line, format!("{source} has no `{character}` component"));
                return Expr::poison();
            }
            components[index] = slot;
        }
        let len = field.len() as u8;
        // A selection off an integer vector is an integer, and off a float
        // vector a float. One component is the scalar itself.
        let ty = match (source, len) {
            (_, 1) => scalar_of(source),
            (Type::Vec2I | Type::Vec3I | Type::Vec4I, n) => integer_vector(n, true),
            (Type::Vec2U | Type::Vec3U | Type::Vec4U, n) => integer_vector(n, false),
            (_, n) => Type::vector(n).expect("a selection is one to four components"),
        };
        Expr::Swizzle {
            ty,
            value: Box::new(value),
            components,
            len,
        }
    }
}

/// The scalar a vector is made of.
pub(crate) fn scalar_of(vector: Type) -> Type {
    match vector {
        Type::Vec2I | Type::Vec3I | Type::Vec4I => Type::I32,
        Type::Vec2U | Type::Vec3U | Type::Vec4U => Type::U32,
        _ => Type::F32,
    }
}

/// The integer vector of this many components.
fn integer_vector(components: u8, signed: bool) -> Type {
    match (components, signed) {
        (2, true) => Type::Vec2I,
        (3, true) => Type::Vec3I,
        (4, true) => Type::Vec4I,
        (2, false) => Type::Vec2U,
        (3, false) => Type::Vec3U,
        _ => Type::Vec4U,
    }
}
