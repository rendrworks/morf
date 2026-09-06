use crate::types::*;

pub use crate::builtin_names::Builtin;

/// A shader, type-checked, with every Lua-ism resolved away.
///
/// The IR is statement-structured rather than SSA, because WGSL is a structured
/// language: keeping `if` and `loop` as statements makes emission a direct
/// print instead of a control-flow reconstruction problem.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub entry: Function,
    /// Helpers the shader declared, monomorphised, in emission order.
    pub helpers: Vec<crate::lower::Instance>,
    /// Declared inputs, in the order the entry point takes them.
    pub inputs: Vec<Binding>,
    /// User parameters, in uniform-block order.
    pub params: Vec<Binding>,
    /// Whether anything in the body read the frame clock.
    ///
    /// Derived while lowering, never declared: a flag an author has to remember
    /// to set is a flag that will be wrong, and this one decides whether a node
    /// repaints every frame or never again.
    pub reads_time: bool,
    /// Whether anything sampled what is underneath.
    pub samples_behind: bool,
    /// Whether anything took a screen-space derivative.
    pub takes_derivative: bool,
    /// Textures the shader declared, in binding order.
    pub textures: Vec<String>,
    /// Data blocks it declared: name, element type, length.
    pub data: Vec<(String, Type, u32)>,
}

/// One named value the shader can read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq)]
/// The entry point.
///
/// It has no parameter list of its own: the emitted signature is fixed so the
/// host's call site never varies with what a shader happened to declare, and
/// the declared inputs are bound inside the body from wherever they come from.
pub struct Function {
    pub returns: Type,
    pub body: Block,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block(pub Vec<Stmt>);

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// A binding. `mutable` decides `var` against `let` on emission, which is
    /// worth getting right: a `let` the driver knows never changes optimises
    /// better, and most shader locals never change.
    Let {
        name: String,
        ty: Type,
        value: Expr,
        mutable: bool,
    },
    Assign {
        target: String,
        value: Expr,
    },
    If {
        arms: Vec<(Expr, Block)>,
        otherwise: Option<Block>,
    },
    /// Every loop shape in the language lowers to this one, so the iteration
    /// guard that keeps a shader from hanging the GPU has exactly one place to
    /// live rather than three that could drift.
    Loop {
        /// Hard iteration ceiling, emitted as a counter the shader cannot
        /// reach around.
        guard: u32,
        body: Block,
        /// What runs before every next turn, including after a `continue`.
        ///
        /// This is what WGSL's `continuing` block is for, and why a numeric
        /// `for` needs one: its counter increment must happen on the way round
        /// even when the body jumped out early, or `continue` turns a counting
        /// loop into one that never advances.
        continuing: Block,
    },
    Break,
    Continue,
    /// Throws this fragment away entirely. Only meaningful where the shader
    /// owns its own coverage.
    Discard,
    Return(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(Value),
    Local {
        name: String,
        ty: Type,
    },
    /// A uniform parameter, by index into `Program::params`.
    Param {
        index: usize,
        ty: Type,
    },
    /// A built-in input — `uv`, `time`, `resolution` — by index into
    /// `Program::inputs`.
    Input {
        index: usize,
        ty: Type,
    },
    Unary {
        op: UnOp,
        ty: Type,
        value: Box<Expr>,
    },
    Binary {
        op: BinOp,
        ty: Type,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        builtin: Builtin,
        ty: Type,
        args: Vec<Expr>,
    },
    /// `vec3(x, y, z)` and friends, after scalar broadcast has been resolved.
    Construct {
        ty: Type,
        args: Vec<Expr>,
    },
    /// `a[i]` on an array, a vector or a matrix column.
    Index {
        ty: Type,
        value: Box<Expr>,
        index: Box<Expr>,
    },
    /// `array<f32, 3>(a, b, c)`, from a Lua table constructor.
    Array {
        ty: Type,
        elements: Vec<Expr>,
    },
    Swizzle {
        ty: Type,
        value: Box<Expr>,
        /// Component indices, `len` of them are meaningful.
        components: [u8; 4],
        len: u8,
    },
}

impl Block {
    /// Marks every local that is assigned to as needing `var` rather than `let`.
    ///
    /// Mutability cannot be known when the declaration is lowered — the
    /// assignment that proves it may be twenty lines further down — so it is
    /// resolved here, once, over the finished body. Getting it wrong the other
    /// way would emit an assignment to a `let`, which WGSL rejects outright.
    pub fn resolve_mutability(&mut self) {
        let mut assigned = Vec::new();
        collect_assigned(self, &mut assigned);
        mark(self, &assigned);
    }
}

fn collect_assigned(block: &Block, out: &mut Vec<String>) {
    for statement in &block.0 {
        match statement {
            Stmt::Assign { target, .. } => out.push(target.clone()),
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    collect_assigned(body, out);
                }
                if let Some(body) = otherwise {
                    collect_assigned(body, out);
                }
            }
            Stmt::Loop {
                body, continuing, ..
            } => {
                collect_assigned(body, out);
                collect_assigned(continuing, out);
            }
            Stmt::Let { .. } | Stmt::Break | Stmt::Continue | Stmt::Discard | Stmt::Return(_) => {}
        }
    }
}

fn mark(block: &mut Block, assigned: &[String]) {
    for statement in &mut block.0 {
        match statement {
            Stmt::Let { name, mutable, .. } => {
                if assigned.iter().any(|target| target == name) {
                    *mutable = true;
                }
            }
            Stmt::If { arms, otherwise } => {
                for (_, body) in arms {
                    mark(body, assigned);
                }
                if let Some(body) = otherwise {
                    mark(body, assigned);
                }
            }
            Stmt::Loop {
                body, continuing, ..
            } => {
                mark(body, assigned);
                mark(continuing, assigned);
            }
            Stmt::Assign { .. }
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Discard
            | Stmt::Return(_) => {}
        }
    }
}

impl Expr {
    /// The type lowering resolved for this expression.
    ///
    /// Every node carries its own type because the checker computed it once.
    /// The emitter never infers and `validate` never guesses.
    pub fn ty(&self) -> Type {
        match self {
            Self::Literal(value) => value.ty(),
            Self::Local { ty, .. }
            | Self::Param { ty, .. }
            | Self::Input { ty, .. }
            | Self::Unary { ty, .. }
            | Self::Binary { ty, .. }
            | Self::Call { ty, .. }
            | Self::Construct { ty, .. }
            | Self::Index { ty, .. }
            | Self::Array { ty, .. }
            | Self::Swizzle { ty, .. } => *ty,
        }
    }

    /// A stand-in for an expression that already produced a diagnostic.
    pub fn poison() -> Self {
        Self::Literal(Value::F32(0.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnOp {
    Negate,
    Not,
    /// `~x`, the bitwise complement. Lua spells it the same way.
    BitNot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl BinOp {
    /// The WGSL operator, for the arithmetic and logical cases.
    pub fn wgsl(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::And => "&&",
            Self::Or => "||",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
        }
    }

    /// Whether the result is a `bool` rather than the operands' type.
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Less
                | Self::LessEqual
                | Self::Greater
                | Self::GreaterEqual
                | Self::Equal
                | Self::NotEqual
        )
    }

    pub fn is_logical(self) -> bool {
        matches!(self, Self::And | Self::Or)
    }

    /// Whether the operator works on the bits rather than the value.
    pub fn is_bitwise(self) -> bool {
        matches!(
            self,
            Self::BitAnd | Self::BitOr | Self::BitXor | Self::ShiftLeft | Self::ShiftRight
        )
    }

    /// Whether the right operand is a shift count rather than a second value.
    ///
    /// WGSL wants a `u32` there whatever is being shifted, which is the one
    /// place an integer operator does not simply take two of the same thing.
    pub fn is_shift(self) -> bool {
        matches!(self, Self::ShiftLeft | Self::ShiftRight)
    }
}
