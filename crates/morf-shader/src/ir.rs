use crate::types::*;

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
    },
    Break,
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
            Stmt::Loop { body, .. } => collect_assigned(body, out),
            Stmt::Let { .. } | Stmt::Break | Stmt::Return(_) => {}
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
            Stmt::Loop { body, .. } => mark(body, assigned),
            Stmt::Assign { .. } | Stmt::Break | Stmt::Return(_) => {}
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

/// A function the shader language provides.
///
/// Everything here maps to one WGSL call, except the few noted, so emission
/// stays a print. A Lua author reaches these by name, and `math.sin` resolves
/// to the same entry as bare `sin` because both spellings will be written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Builtin {
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    /// `atan2(y, x)`: the angle of a vector, which is how a polar shader starts.
    Atan2,
    Ceil,
    Clamp,
    Cos,
    Cosh,
    /// The one thing the RbxShader collection needed that was missing.
    CountLeadingZeros,
    CountOneBits,
    CountTrailingZeros,
    Cross,
    Degrees,
    Determinant,
    Distance,
    Dot,
    Exp,
    Exp2,
    ExtractBits,
    FaceForward,
    Fma,
    FirstLeadingBit,
    FirstTrailingBit,
    Floor,
    Fract,
    InsertBits,
    Inverse,
    InverseSqrt,
    Length,
    Log,
    Log2,
    Max,
    Min,
    Mix,
    Normalize,
    Pow,
    QuantizeToF16,
    Radians,
    Reflect,
    Refract,
    ReverseBits,
    Round,
    Saturate,
    Select,
    Sign,
    Sin,
    Sinh,
    Smoothstep,
    Sqrt,
    Step,
    Tan,
    Tanh,
    Transpose,
    Trunc,
    /// `floor(a / b)`, Lua's `//`. Emitted as the division, not a call.
    FloorDiv,
    /// Samples what is rendered underneath. Effect shaders only.
    Texture,
    /// A conversion between scalar types: `f32(x)`, `i32(x)`, `u32(x)`.
    ///
    /// The result type is on the `Call`, so the emitter prints the type's own
    /// name and nothing else has to carry it.
    Convert,
    /// Reinterprets the bits rather than the value, which is where every hash
    /// starts. The result type is on the `Call`.
    Bitcast,
    /// A call to a helper the shader itself declared.
    ///
    /// The first argument carries the emitted function's name rather than a
    /// value, because a call is the one place the emitter needs a name it did
    /// not compute from a type.
    Helper,
}

impl Builtin {
    /// The WGSL function name, where there is a direct one.
    pub fn wgsl(self) -> &'static str {
        match self {
            Self::Abs => "abs",
            Self::Acos => "acos",
            Self::Acosh => "acosh",
            Self::Asin => "asin",
            Self::Asinh => "asinh",
            Self::Atan => "atan",
            Self::Atanh => "atanh",
            Self::Atan2 => "atan2",
            Self::Ceil => "ceil",
            Self::Clamp => "clamp",
            Self::Cos => "cos",
            Self::CountLeadingZeros => "countLeadingZeros",
            Self::CountOneBits => "countOneBits",
            Self::CountTrailingZeros => "countTrailingZeros",
            Self::Cross => "cross",
            Self::Cosh => "cosh",
            Self::Degrees => "degrees",
            Self::Determinant => "determinant",
            Self::Distance => "distance",
            Self::Dot => "dot",
            Self::Exp => "exp",
            Self::ExtractBits => "extractBits",
            Self::Exp2 => "exp2",
            Self::FaceForward => "faceForward",
            Self::Fma => "fma",
            Self::Floor => "floor",
            Self::FirstLeadingBit => "firstLeadingBit",
            Self::FirstTrailingBit => "firstTrailingBit",
            Self::Fract => "fract",
            Self::InsertBits => "insertBits",
            Self::Inverse => "inverse",
            Self::InverseSqrt => "inverseSqrt",
            Self::Length => "length",
            Self::Log => "log",
            Self::Log2 => "log2",
            Self::Max => "max",
            Self::Min => "min",
            Self::Mix => "mix",
            Self::Normalize => "normalize",
            Self::Pow => "pow",
            Self::QuantizeToF16 => "quantizeToF16",
            Self::Radians => "radians",
            Self::Reflect => "reflect",
            Self::Refract => "refract",
            Self::ReverseBits => "reverseBits",
            Self::Round => "round",
            Self::Saturate => "saturate",
            Self::Select => "select",
            Self::Sign => "sign",
            Self::Sin => "sin",
            Self::Sinh => "sinh",
            Self::Smoothstep => "smoothstep",
            Self::Sqrt => "sqrt",
            Self::Step => "step",
            Self::Tan => "tan",
            Self::Tanh => "tanh",
            Self::Transpose => "transpose",
            Self::Trunc => "trunc",
            Self::FloorDiv => "floor",
            Self::Texture => "textureSample",
            Self::Convert => "",
            Self::Bitcast => "bitcast",
            Self::Helper => "",
        }
    }
}
