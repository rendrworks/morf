use luna::compiler::parser::{
    CallSuffix, Expression, FieldSuffix, HeadExpression, PrimaryExpression, SimpleExpression,
    SuffixPart, SuffixedExpression,
};

use crate::builtins;
use crate::ir::*;
use crate::lower::{Lowerer, Name, text};
use crate::types::*;

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
            SimpleExpression::TableConstructor(_) => {
                self.error_note(
                    line,
                    "a shader has no tables",
                    "use a vector: `vec3(x, y, z)`",
                );
                Expr::poison()
            }
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
            SuffixPart::Field(FieldSuffix::Indexed(_)) => {
                self.error_note(
                    line,
                    "a shader cannot index with brackets",
                    "name the component instead: `v.x`",
                );
                Expr::poison()
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
        if let Some(ty) = Type::parse(name).filter(|ty| ty.is_vector()) {
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
        if lowered.len() != arity {
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
        if builtin == Builtin::Texture {
            self.samples_behind = true;
        }
        if matches!(builtin, Builtin::Dpdx | Builtin::Dpdy | Builtin::Fwidth) {
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
    fn construct(&mut self, ty: Type, args: Vec<Expr>, line: u32) -> Expr {
        let wanted = ty.components();
        if args.is_empty() {
            self.error(line, format!("{ty} needs at least one component"));
            return Expr::poison();
        }
        if args.iter().any(|arg| arg.ty().is_poison()) {
            return Expr::Construct { ty, args };
        }
        // One scalar fills every component: `vec3(0.5)` is grey, and so is
        // `vec3(1)` — an abstract literal is a scalar like any other.
        if args.len() == 1 && (args[0].ty() == Type::F32 || args[0].ty() == Type::AbstractInt) {
            return Expr::Construct {
                ty,
                args: args.into_iter().map(Lowerer::commit).collect(),
            };
        }
        let mut supplied = 0;
        for arg in &args {
            let arg_ty = arg.ty();
            if !arg_ty.is_numeric() || arg_ty == Type::I32 {
                self.error(
                    line,
                    format!("{ty} takes numbers and vectors, not {arg_ty}"),
                );
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
            args: args.into_iter().map(Lowerer::commit).collect(),
        }
    }

    /// A scalar conversion or a bitcast, if the name is one.
    fn conversion(&mut self, name: &str, args: &[Expr], line: u32) -> Option<Expr> {
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
    fn construct_matrix(&mut self, ty: Type, args: Vec<Expr>, line: u32) -> Expr {
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

    /// `v.x`, `v.xy`, `v.rgb` — component selection.
    fn swizzle(&mut self, value: Expr, field: &str, line: u32) -> Expr {
        let source = value.ty();
        if source.is_poison() {
            return value;
        }
        if !source.is_vector() {
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
        let ty = Type::vector(len).expect("a selection is one to four components");
        Expr::Swizzle {
            ty,
            value: Box::new(value),
            components,
            len,
        }
    }
}
