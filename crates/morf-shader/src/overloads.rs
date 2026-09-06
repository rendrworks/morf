//! What each builtin's argument types mean, and what it gives back.
//!
//! Split from the table itself: `builtins` is a list of names, this is the
//! rules. Every arm here exists so that a wrong call produces a message naming
//! both types rather than WGSL a driver refuses with no line number.

use crate::builtins::Shape;
use crate::types::Type;

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
        Shape::Whole1 => {
            let ty = args[0];
            (ty.is_numeric() && !ty.is_matrix())
                .then(|| ty.defaulted())
                .ok_or_else(|| format!("{name} takes a number or vector, not {ty}"))
        }
        Shape::Whole2 | Shape::Whole3 => {
            let ty = args[0].defaulted();
            if !ty.is_numeric() {
                return Err(format!("{name} takes numbers or vectors, not {ty}"));
            }
            for other in &args[1..] {
                // A scalar rides along with a vector, and an undecided literal
                // takes whatever the first argument settled on.
                if !other.fits(ty) && *other != Type::F32 {
                    return Err(format!("{name} cannot mix {ty} and {other}"));
                }
            }
            Ok(ty)
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
        Shape::Predicate => {
            let ty = args[0];
            (ty == Type::F32)
                .then_some(Type::Bool)
                .ok_or_else(|| format!("{name} tests one number, not {ty}"))
        }
        Shape::BoolFold => {
            let ty = args[0];
            (ty == Type::Bool).then_some(Type::Bool).ok_or_else(|| {
                format!("{name} folds a bool, not {ty}; this language has no bool vectors yet")
            })
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
        Shape::Split => {
            let ty = args[0];
            (ty == Type::F32 || ty.is_vector())
                .then_some(Type::Split)
                .ok_or_else(|| format!("{name} splits a number or vector, not {ty}"))
        }
        Shape::Ldexp => {
            let (fraction, exponent) = (args[0], args[1]);
            if fraction != Type::F32 && !fraction.is_vector() {
                return Err(format!("ldexp scales a number or vector, not {fraction}"));
            }
            exponent
                .is_integer()
                .then_some(fraction)
                .ok_or_else(|| format!("ldexp's exponent is a whole number, not {exponent}"))
        }
        Shape::Outer => {
            let (left, right) = (args[0], args[1]);
            if !left.is_vector() || !right.is_vector() {
                return Err(format!("outer takes two vectors, not {left} and {right}"));
            }
            // Only the square case: this language has no `matCxR` for the rest,
            // and saying so is better than emitting a type it cannot name.
            if left != right {
                return Err(format!(
                    "outer of {left} and {right} is not square, and this language has \
                     only square matrices"
                ));
            }
            Type::matrix(left.components())
                .ok_or_else(|| format!("outer of two {left} has no matrix type here"))
        }
        Shape::Pack => {
            let ty = args[0];
            let wanted = if name.starts_with("pack4") {
                Type::Vec4
            } else {
                Type::Vec2
            };
            (ty == wanted)
                .then_some(Type::U32)
                .ok_or_else(|| format!("{name} packs a {wanted}, not {ty}"))
        }
        Shape::PackInt => {
            let ty = args[0];
            let signed = name.contains("_i8");
            let wanted = if signed { Type::Vec4I } else { Type::Vec4U };
            (ty == wanted)
                .then_some(Type::U32)
                .ok_or_else(|| format!("{name} packs a {wanted}, not {ty}"))
        }
        Shape::Unpack(width) => {
            let ty = args[0];
            (ty.is_integer())
                .then(|| Type::vector(width).expect("two or four"))
                .ok_or_else(|| format!("{name} unpacks a u32, not {ty}"))
        }
        Shape::UnpackInt(signed) => {
            let ty = args[0];
            (ty.is_integer())
                .then_some(if signed { Type::Vec4I } else { Type::Vec4U })
                .ok_or_else(|| format!("{name} unpacks a u32, not {ty}"))
        }
        Shape::DotPacked(signed) => {
            for argument in args {
                if !argument.is_integer() {
                    return Err(format!("{name} takes two u32, not {argument}"));
                }
            }
            Ok(if signed { Type::I32 } else { Type::U32 })
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
        Shape::TextureSize => {
            let ty = args[0];
            (ty == Type::Texture)
                .then_some(Type::Vec2U)
                .ok_or_else(|| format!("texture_size takes a texture, not {ty}"))
        }
        Shape::TextureLoad => {
            if args[0] != Type::Texture {
                return Err(format!("texture_load takes a texture, not {}", args[0]));
            }
            if args[1] != Type::Vec2I && args[1] != Type::Vec2U {
                return Err(format!(
                    "texture_load takes whole-number coordinates, not {}",
                    args[1]
                ));
            }
            args[2]
                .is_integer()
                .then_some(Type::Vec4)
                .ok_or_else(|| format!("texture_load's level is a whole number, not {}", args[2]))
        }
        Shape::TextureLevel => {
            if args[0] != Type::Texture {
                return Err(format!("texture_level takes a texture, not {}", args[0]));
            }
            if args[1] != Type::Vec2 {
                return Err(format!(
                    "texture_level takes a vec2 coordinate, not {}",
                    args[1]
                ));
            }
            (args[2] == Type::F32)
                .then_some(Type::Vec4)
                .ok_or_else(|| format!("texture_level's mip is an f32, not {}", args[2]))
        }
        Shape::Texture => match args {
            [coordinate] => (*coordinate == Type::Vec2)
                .then_some(Type::Vec4)
                .ok_or_else(|| format!("texture takes a vec2 coordinate, not {coordinate}")),
            [source, coordinate] => {
                if *source != Type::Texture {
                    return Err(format!(
                        "`{source}` is not a texture; declare one in the shader's `textures`"
                    ));
                }
                (*coordinate == Type::Vec2)
                    .then_some(Type::Vec4)
                    .ok_or_else(|| format!("texture takes a vec2 coordinate, not {coordinate}"))
            }
            _ => Err("texture takes a coordinate, or a texture and a coordinate".to_owned()),
        },
    }
}
