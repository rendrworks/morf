use crate::ir::Builtin;
use crate::types::Type;

/// How a builtin's argument types decide its result type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shape {
    /// One argument, result is the argument's type: `sin`, `abs`, `floor`.
    Componentwise1,
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
    /// One integer in, the same integer type out.
    Integer1,
    /// `extractBits(value, offset, count)` — three whole numbers.
    IntegerBits,
    /// `insertBits(value, newbits, offset, count)` — four, which is the one
    /// builtin in this language that takes that many.
    IntegerInsert,
    /// One matrix in, the same matrix out: `transpose`, `inverse`.
    Matrix1,
    /// One matrix in, a scalar out: `determinant`.
    MatrixFold,
    /// `texture(uv)` — vec2 in, vec4 out.
    Texture,
}

/// Everything the language provides, by the name a shader writes.
///
/// An explicit table rather than a unification scheme. It is longer to read and
/// duller to write, but every overload is checkable by eye, and the error a
/// user gets on a mismatch can list exactly what was available — which is worth
/// more than the cleverness would have been.
pub(crate) const BUILTINS: &[(&str, Builtin, Shape)] = &[
    ("abs", Builtin::Abs, Shape::Componentwise1),
    ("acosh", Builtin::Acosh, Shape::Componentwise1),
    ("asinh", Builtin::Asinh, Shape::Componentwise1),
    ("atanh", Builtin::Atanh, Shape::Componentwise1),
    ("acos", Builtin::Acos, Shape::Componentwise1),
    ("asin", Builtin::Asin, Shape::Componentwise1),
    ("atan", Builtin::Atan, Shape::Componentwise1),
    // `atan2(y, x)` is where every polar shader starts, and there is no way to
    // write it out of the other builtins.
    ("atan2", Builtin::Atan2, Shape::Componentwise2),
    ("ceil", Builtin::Ceil, Shape::Componentwise1),
    ("clamp", Builtin::Clamp, Shape::Componentwise3),
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
    ("fract", Builtin::Fract, Shape::Componentwise1),
    // Both spellings: WGSL writes it one way and GLSL, which is what a shader
    // author has read more of, writes it the other.
    ("insert_bits", Builtin::InsertBits, Shape::IntegerInsert),
    ("inverse", Builtin::Inverse, Shape::Matrix1),
    ("inversesqrt", Builtin::InverseSqrt, Shape::Componentwise1),
    ("inverse_sqrt", Builtin::InverseSqrt, Shape::Componentwise1),
    ("length", Builtin::Length, Shape::Fold1),
    ("log", Builtin::Log, Shape::Componentwise1),
    ("log2", Builtin::Log2, Shape::Componentwise1),
    ("max", Builtin::Max, Shape::Componentwise2),
    ("min", Builtin::Min, Shape::Componentwise2),
    ("mix", Builtin::Mix, Shape::MixScalar),
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
    ("sign", Builtin::Sign, Shape::Componentwise1),
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
        | Shape::Fold1
        | Shape::Texture
        | Shape::Matrix1
        | Shape::Integer1
        | Shape::MatrixFold => 1,
        Shape::Componentwise2 | Shape::Fold2 | Shape::Cross => 2,
        Shape::Componentwise3
        | Shape::MixScalar
        | Shape::EdgeScalar
        | Shape::Select
        | Shape::Refract
        | Shape::IntegerBits => 3,
        Shape::IntegerInsert => 4,
    }
}

/// Checks argument types against a shape and returns the result type.
///
/// `Err` carries the message body; the caller supplies the line and the call's
/// own name, so the wording stays in one place.
pub(crate) fn resolve(name: &str, shape: Shape, args: &[Type]) -> Result<Type, String> {
    // A poisoned argument already produced a diagnostic. Reporting a second
    // one about its type would be blaming the user for our own placeholder.
    if args.iter().any(|ty| ty.is_poison()) {
        return Ok(Type::Poison);
    }
    let numeric = |ty: Type| ty.is_numeric() && ty != Type::I32;
    match shape {
        Shape::Componentwise1 => {
            let ty = args[0];
            numeric(ty)
                .then_some(ty)
                .ok_or_else(|| format!("{name} takes a number or vector, not {ty}"))
        }
        Shape::Componentwise2 | Shape::Componentwise3 => {
            let ty = args[0];
            if !numeric(ty) {
                return Err(format!("{name} takes numbers or vectors, not {ty}"));
            }
            // A scalar rides along with a vector, as it does in WGSL, so
            // `clamp(v, 0.0, 1.0)` means what it looks like.
            for other in &args[1..] {
                if *other != ty && *other != Type::F32 {
                    return Err(format!("{name} cannot mix {ty} and {other}"));
                }
            }
            Ok(ty)
        }
        Shape::Fold1 => {
            let ty = args[0];
            numeric(ty)
                .then_some(Type::F32)
                .ok_or_else(|| format!("{name} takes a vector, not {ty}"))
        }
        Shape::Fold2 => {
            let (left, right) = (args[0], args[1]);
            if left != right {
                return Err(format!(
                    "{name} takes two of one type, not {left} and {right}"
                ));
            }
            numeric(left)
                .then_some(Type::F32)
                .ok_or_else(|| format!("{name} takes vectors, not {left}"))
        }
        Shape::MixScalar => {
            let (a, b, t) = (args[0], args[1], args[2]);
            if a != b {
                return Err(format!("mix needs both ends to match, not {a} and {b}"));
            }
            if !numeric(a) {
                return Err(format!("mix takes numbers or vectors, not {a}"));
            }
            if t != a && t != Type::F32 {
                return Err(format!("mix's amount must be f32 or {a}, not {t}"));
            }
            Ok(a)
        }
        Shape::EdgeScalar => {
            let (low, high, x) = (args[0], args[1], args[2]);
            if !numeric(x) {
                return Err(format!("{name} takes a number or vector, not {x}"));
            }
            for edge in [low, high] {
                if edge != Type::F32 && edge != x {
                    return Err(format!("{name}'s edges must be f32 or {x}, not {edge}"));
                }
            }
            Ok(x)
        }
        Shape::Select => {
            let (a, b, cond) = (args[0], args[1], args[2]);
            if a != b {
                return Err(format!("select needs both arms to match, not {a} and {b}"));
            }
            if cond != Type::Bool {
                return Err(format!(
                    "select's condition must be a bool, not {cond}; write a comparison"
                ));
            }
            Ok(a)
        }
        Shape::Cross => {
            let (left, right) = (args[0], args[1]);
            (left == Type::Vec3 && right == Type::Vec3)
                .then_some(Type::Vec3)
                .ok_or_else(|| format!("cross takes two vec3, not {left} and {right}"))
        }
        Shape::Integer1 => {
            let ty = args[0];
            if ty == Type::AbstractInt {
                return Ok(Type::I32);
            }
            ty.is_integer()
                .then_some(ty)
                .ok_or_else(|| format!("{name} takes a whole number, not {ty}"))
        }
        Shape::IntegerInsert | Shape::IntegerBits => {
            let ty = args[0];
            if !ty.is_integer() {
                return Err(format!("{name} takes a whole number, not {ty}"));
            }
            for count in &args[1..] {
                if !count.is_integer() {
                    return Err(format!(
                        "{name}'s bit counts must be whole numbers, not {count}"
                    ));
                }
            }
            Ok(if ty == Type::AbstractInt {
                Type::I32
            } else {
                ty
            })
        }
        Shape::Matrix1 => {
            let ty = args[0];
            ty.is_matrix()
                .then_some(ty)
                .ok_or_else(|| format!("{name} takes a matrix, not {ty}"))
        }
        Shape::MatrixFold => {
            let ty = args[0];
            ty.is_matrix()
                .then_some(Type::F32)
                .ok_or_else(|| format!("{name} takes a matrix, not {ty}"))
        }
        Shape::Refract => {
            let (incident, normal, eta) = (args[0], args[1], args[2]);
            if incident != normal || !incident.is_vector() {
                return Err(format!(
                    "refract takes two vectors of one type, not {incident} and {normal}"
                ));
            }
            (eta == Type::F32)
                .then_some(incident)
                .ok_or_else(|| format!("refract's ratio must be an f32, not {eta}"))
        }
        Shape::Texture => {
            let ty = args[0];
            (ty == Type::Vec2)
                .then_some(Type::Vec4)
                .ok_or_else(|| format!("texture takes a vec2 coordinate, not {ty}"))
        }
    }
}
