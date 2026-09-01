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
    ("ceil", Builtin::Ceil, Shape::Componentwise1),
    ("clamp", Builtin::Clamp, Shape::Componentwise3),
    ("cos", Builtin::Cos, Shape::Componentwise1),
    ("degrees", Builtin::Degrees, Shape::Componentwise1),
    ("distance", Builtin::Distance, Shape::Fold2),
    ("dot", Builtin::Dot, Shape::Fold2),
    ("exp", Builtin::Exp, Shape::Componentwise1),
    ("exp2", Builtin::Exp2, Shape::Componentwise1),
    ("floor", Builtin::Floor, Shape::Componentwise1),
    ("fract", Builtin::Fract, Shape::Componentwise1),
    ("length", Builtin::Length, Shape::Fold1),
    ("log", Builtin::Log, Shape::Componentwise1),
    ("log2", Builtin::Log2, Shape::Componentwise1),
    ("max", Builtin::Max, Shape::Componentwise2),
    ("min", Builtin::Min, Shape::Componentwise2),
    ("mix", Builtin::Mix, Shape::MixScalar),
    ("normalize", Builtin::Normalize, Shape::Componentwise1),
    ("pow", Builtin::Pow, Shape::Componentwise2),
    ("radians", Builtin::Radians, Shape::Componentwise1),
    ("reflect", Builtin::Reflect, Shape::Componentwise2),
    ("round", Builtin::Round, Shape::Componentwise1),
    ("select", Builtin::Select, Shape::Select),
    ("sign", Builtin::Sign, Shape::Componentwise1),
    ("sin", Builtin::Sin, Shape::Componentwise1),
    ("smoothstep", Builtin::Smoothstep, Shape::EdgeScalar),
    ("sqrt", Builtin::Sqrt, Shape::Componentwise1),
    ("step", Builtin::Step, Shape::Componentwise2),
    ("tan", Builtin::Tan, Shape::Componentwise1),
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
        Shape::Componentwise1 | Shape::Fold1 | Shape::Texture => 1,
        Shape::Componentwise2 | Shape::Fold2 => 2,
        Shape::Componentwise3 | Shape::MixScalar | Shape::EdgeScalar | Shape::Select => 3,
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
        Shape::Texture => {
            let ty = args[0];
            (ty == Type::Vec2)
                .then_some(Type::Vec4)
                .ok_or_else(|| format!("texture takes a vec2 coordinate, not {ty}"))
        }
    }
}
