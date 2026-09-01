use crate::ir::Builtin;
pub(crate) use crate::overloads::resolve;

/// How a builtin's argument types decide its result type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shape {
    /// One argument, result is the argument's type: `sin`, `floor`.
    Componentwise1,
    /// The same, but whole numbers are allowed: `abs`, `sign`.
    ///
    /// A separate shape rather than a flag, because the difference is real:
    /// `sin` of an integer is not defined in WGSL and letting it through would
    /// mean a driver refusing it with no line number.
    Whole1,
    /// Two arguments, whole numbers allowed: `min`, `max`.
    Whole2,
    /// Three, whole numbers allowed: `clamp`.
    Whole3,
    /// Two arguments of one type, result is that type: `pow`, `min`, `max`.
    Componentwise2,
    /// Three arguments of one type: `clamp`, `mix` with a vector amount.
    Componentwise3,
    /// Vector in, scalar out: `length`.
    Fold1,
    /// Two vectors in, scalar out: `dot`, `distance`.
    Fold2,
    /// `mix(a, b, t)` where `t` may be a scalar against vector `a` and `b`.
    MixScalar,
    /// `smoothstep(edge0, edge1, x)` with scalar edges and any-typed `x`.
    EdgeScalar,
    /// `select(a, b, cond)` — a bool third argument, result is the first two.
    Select,
    /// `cross(a, b)` — two vec3 in, vec3 out. Only defined in three dimensions.
    Cross,
    /// `refract(incident, normal, eta)` — two vectors and a scalar ratio.
    Refract,
    /// One number or vector in, a bool out: `isNan`, `isInf`.
    Predicate,
    /// A boolean vector in, a bool out: `all`, `any`.
    BoolFold,
    /// One integer in, the same integer type out.
    Integer1,
    /// `extractBits(value, offset, count)` — three whole numbers.
    IntegerBits,
    /// `insertBits(value, newbits, offset, count)` — four, which is the one
    /// builtin in this language that takes that many.
    IntegerInsert,
    /// `modf(x)` and `frexp(x)`, whose results are read through `.fract`,
    /// `.whole` and `.exp` rather than being held as values.
    Split,
    /// `ldexp(fraction, exponent)` — a float and a whole number.
    Ldexp,
    /// `outer(a, b)` — two vectors to a matrix of their sizes.
    Outer,
    /// A float vector in, a `u32` out. The packing family.
    Pack,
    /// An integer vector in, a `u32` out.
    PackInt,
    /// A `u32` in, a float vector of a fixed width out.
    Unpack(u8),
    /// A `u32` in, an integer vector out.
    UnpackInt(bool),
    /// Two `u32` in, one packed dot product out.
    DotPacked(bool),
    /// One matrix in, the same matrix out: `transpose`, `inverse`.
    Matrix1,
    /// One matrix in, a scalar out: `determinant`.
    MatrixFold,
    /// `texture(uv)` samples what is underneath; `texture(name, uv)` samples a
    /// declared one. Arity decides which.
    Texture,
}

/// Everything the language provides, by the name a shader writes.
///
/// An explicit table rather than a unification scheme. It is longer to read and
/// duller to write, but every overload is checkable by eye, and the error a
/// user gets on a mismatch can list exactly what was available — which is worth
/// more than the cleverness would have been.
pub(crate) const BUILTINS: &[(&str, Builtin, Shape)] = &[
    ("abs", Builtin::Abs, Shape::Whole1),
    ("acosh", Builtin::Acosh, Shape::Componentwise1),
    ("all", Builtin::All, Shape::BoolFold),
    ("any", Builtin::Any, Shape::BoolFold),
    ("asinh", Builtin::Asinh, Shape::Componentwise1),
    ("atanh", Builtin::Atanh, Shape::Componentwise1),
    ("acos", Builtin::Acos, Shape::Componentwise1),
    ("asin", Builtin::Asin, Shape::Componentwise1),
    ("atan", Builtin::Atan, Shape::Componentwise1),
    // `atan2(y, x)` is where every polar shader starts, and there is no way to
    // write it out of the other builtins.
    ("atan2", Builtin::Atan2, Shape::Componentwise2),
    ("ceil", Builtin::Ceil, Shape::Componentwise1),
    ("clamp", Builtin::Clamp, Shape::Whole3),
    ("cos", Builtin::Cos, Shape::Componentwise1),
    ("cosh", Builtin::Cosh, Shape::Componentwise1),
    (
        "count_leading_zeros",
        Builtin::CountLeadingZeros,
        Shape::Integer1,
    ),
    ("count_one_bits", Builtin::CountOneBits, Shape::Integer1),
    (
        "count_trailing_zeros",
        Builtin::CountTrailingZeros,
        Shape::Integer1,
    ),
    ("cross", Builtin::Cross, Shape::Cross),
    ("degrees", Builtin::Degrees, Shape::Componentwise1),
    ("determinant", Builtin::Determinant, Shape::MatrixFold),
    ("distance", Builtin::Distance, Shape::Fold2),
    // Screen-space derivatives. `field.wgsl` softens its own edges with
    // `fwidth`; without these a configuration's shader could not do the same,
    // which is an odd thing for the engine to keep to itself.
    ("dpdx", Builtin::Dpdx, Shape::Componentwise1),
    ("dpdy", Builtin::Dpdy, Shape::Componentwise1),
    ("dot", Builtin::Dot, Shape::Fold2),
    ("exp", Builtin::Exp, Shape::Componentwise1),
    ("exp2", Builtin::Exp2, Shape::Componentwise1),
    ("extract_bits", Builtin::ExtractBits, Shape::IntegerBits),
    ("faceforward", Builtin::FaceForward, Shape::Componentwise3),
    ("face_forward", Builtin::FaceForward, Shape::Componentwise3),
    ("fma", Builtin::Fma, Shape::Componentwise3),
    (
        "first_leading_bit",
        Builtin::FirstLeadingBit,
        Shape::Integer1,
    ),
    (
        "first_trailing_bit",
        Builtin::FirstTrailingBit,
        Shape::Integer1,
    ),
    ("floor", Builtin::Floor, Shape::Componentwise1),
    ("frexp", Builtin::Frexp, Shape::Split),
    ("fract", Builtin::Fract, Shape::Componentwise1),
    ("fwidth", Builtin::Fwidth, Shape::Componentwise1),
    // Both spellings: WGSL writes it one way and GLSL, which is what a shader
    // author has read more of, writes it the other.
    ("insert_bits", Builtin::InsertBits, Shape::IntegerInsert),
    ("inverse", Builtin::Inverse, Shape::Matrix1),
    ("is_inf", Builtin::IsInf, Shape::Predicate),
    ("is_nan", Builtin::IsNan, Shape::Predicate),
    ("inversesqrt", Builtin::InverseSqrt, Shape::Componentwise1),
    ("inverse_sqrt", Builtin::InverseSqrt, Shape::Componentwise1),
    ("length", Builtin::Length, Shape::Fold1),
    ("log", Builtin::Log, Shape::Componentwise1),
    ("log2", Builtin::Log2, Shape::Componentwise1),
    ("max", Builtin::Max, Shape::Whole2),
    ("min", Builtin::Min, Shape::Whole2),
    ("mix", Builtin::Mix, Shape::MixScalar),
    ("modf", Builtin::Modf, Shape::Split),
    ("normalize", Builtin::Normalize, Shape::Componentwise1),
    ("pow", Builtin::Pow, Shape::Componentwise2),
    (
        "quantize_to_f16",
        Builtin::QuantizeToF16,
        Shape::Componentwise1,
    ),
    ("radians", Builtin::Radians, Shape::Componentwise1),
    ("reflect", Builtin::Reflect, Shape::Componentwise2),
    ("refract", Builtin::Refract, Shape::Refract),
    ("reverse_bits", Builtin::ReverseBits, Shape::Integer1),
    ("round", Builtin::Round, Shape::Componentwise1),
    // `clamp(x, 0, 1)`, which is written often enough to have its own name.
    ("saturate", Builtin::Saturate, Shape::Componentwise1),
    ("select", Builtin::Select, Shape::Select),
    ("sign", Builtin::Sign, Shape::Whole1),
    ("sin", Builtin::Sin, Shape::Componentwise1),
    ("sinh", Builtin::Sinh, Shape::Componentwise1),
    ("smoothstep", Builtin::Smoothstep, Shape::EdgeScalar),
    ("sqrt", Builtin::Sqrt, Shape::Componentwise1),
    ("step", Builtin::Step, Shape::Componentwise2),
    ("tan", Builtin::Tan, Shape::Componentwise1),
    // Shadertoy tonemaps with `tanh` constantly, and it cannot be written out
    // of the rest.
    ("tanh", Builtin::Tanh, Shape::Componentwise1),
    ("transpose", Builtin::Transpose, Shape::Matrix1),
    ("trunc", Builtin::Trunc, Shape::Componentwise1),
    (
        "dot4_i8_packed",
        Builtin::Dot4I8Packed,
        Shape::DotPacked(true),
    ),
    (
        "dot4_u8_packed",
        Builtin::Dot4U8Packed,
        Shape::DotPacked(false),
    ),
    ("dpdx_coarse", Builtin::DpdxCoarse, Shape::Componentwise1),
    ("dpdx_fine", Builtin::DpdxFine, Shape::Componentwise1),
    ("dpdy_coarse", Builtin::DpdyCoarse, Shape::Componentwise1),
    ("dpdy_fine", Builtin::DpdyFine, Shape::Componentwise1),
    (
        "fwidth_coarse",
        Builtin::FwidthCoarse,
        Shape::Componentwise1,
    ),
    ("fwidth_fine", Builtin::FwidthFine, Shape::Componentwise1),
    ("ldexp", Builtin::Ldexp, Shape::Ldexp),
    ("outer", Builtin::Outer, Shape::Outer),
    ("pack2x16float", Builtin::Pack2x16float, Shape::Pack),
    ("pack2x16snorm", Builtin::Pack2x16snorm, Shape::Pack),
    ("pack2x16unorm", Builtin::Pack2x16unorm, Shape::Pack),
    ("pack4x8snorm", Builtin::Pack4x8snorm, Shape::Pack),
    ("pack4x8unorm", Builtin::Pack4x8unorm, Shape::Pack),
    ("pack4x_i8", Builtin::Pack4xI8, Shape::PackInt),
    ("pack4x_i8_clamp", Builtin::Pack4xI8Clamp, Shape::PackInt),
    ("pack4x_u8", Builtin::Pack4xU8, Shape::PackInt),
    ("pack4x_u8_clamp", Builtin::Pack4xU8Clamp, Shape::PackInt),
    (
        "unpack2x16float",
        Builtin::Unpack2x16float,
        Shape::Unpack(2),
    ),
    (
        "unpack2x16snorm",
        Builtin::Unpack2x16snorm,
        Shape::Unpack(2),
    ),
    (
        "unpack2x16unorm",
        Builtin::Unpack2x16unorm,
        Shape::Unpack(2),
    ),
    ("unpack4x8snorm", Builtin::Unpack4x8snorm, Shape::Unpack(4)),
    ("unpack4x8unorm", Builtin::Unpack4x8unorm, Shape::Unpack(4)),
    ("unpack4x_i8", Builtin::Unpack4xI8, Shape::UnpackInt(true)),
    ("unpack4x_u8", Builtin::Unpack4xU8, Shape::UnpackInt(false)),
    ("texture", Builtin::Texture, Shape::Texture),
];

/// Resolves a call by name.
///
/// `math.sin` resolves to the same entry as bare `sin`, because a Lua author
/// will write both and having one of them fail would be a distinction without
/// a reason.
pub(crate) fn lookup(name: &str) -> Option<(Builtin, Shape)> {
    let bare = name.strip_prefix("math.").unwrap_or(name);
    BUILTINS
        .iter()
        .find(|(candidate, ..)| *candidate == bare)
        .map(|(_, builtin, shape)| (*builtin, *shape))
}

/// The names a diagnostic offers when a call does not resolve.
pub(crate) fn available() -> String {
    let mut names: Vec<&str> = BUILTINS.iter().map(|(name, ..)| *name).collect();
    names.sort_unstable();
    names.join(", ")
}

/// How many arguments a shape takes.
pub(crate) fn arity(shape: Shape) -> usize {
    match shape {
        Shape::Componentwise1
        | Shape::Whole1
        | Shape::Fold1
        | Shape::Texture
        | Shape::Matrix1
        | Shape::Integer1
        | Shape::Predicate
        | Shape::BoolFold
        | Shape::MatrixFold
        | Shape::Split
        | Shape::Pack
        | Shape::PackInt => 1,
        Shape::Componentwise2
        | Shape::Whole2
        | Shape::Fold2
        | Shape::Cross
        | Shape::Ldexp
        | Shape::Outer
        | Shape::DotPacked(_) => 2,
        Shape::Componentwise3
        | Shape::Whole3
        | Shape::MixScalar
        | Shape::EdgeScalar
        | Shape::Select
        | Shape::Refract
        | Shape::IntegerBits => 3,
        Shape::IntegerInsert => 4,
        Shape::Unpack(_) | Shape::UnpackInt(_) => 1,
    }
}
