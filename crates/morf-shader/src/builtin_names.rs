//! The builtin enum and its WGSL spellings.
//!
//! Split from `ir` when the two crossed the line gate: what is here is a list
//! of names, and what is left there is the shape of a program.

/// A function the shader language provides.
///
/// Everything here maps to one WGSL call, except the few noted, so emission
/// stays a print. A Lua author reaches these by name, and `math.sin` resolves
/// to the same entry as bare `sin` because both spellings will be written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Builtin {
    Abs,
    Acos,
    /// `all` and `any` fold a boolean vector; `isNan` and `isInf` test one
    /// number. WGSL calls these relational rather than math functions.
    All,
    Any,
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
    /// Screen-space derivatives. The engine's own shader antialiases with
    /// `fwidth`; without these a configuration's shader cannot.
    Dpdx,
    Dpdy,
    Fwidth,
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
    IsInf,
    IsNan,
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
            Self::All => "all",
            Self::Any => "any",
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
            Self::Dpdx => "dpdx",
            Self::Dpdy => "dpdy",
            Self::Fwidth => "fwidth",
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
            Self::IsInf => "isInf",
            Self::IsNan => "isNan",
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
