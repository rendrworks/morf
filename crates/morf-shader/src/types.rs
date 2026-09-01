use std::fmt;

/// Every type a shader value can have.
///
/// Deliberately small. There is no `nil`, no string, no table and no function
/// value: a shader is arithmetic over numbers and vectors, and everything Lua
/// offers beyond that is rejected with a diagnostic rather than half-supported.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Type {
    F32,
    Vec2,
    Vec3,
    Vec4,
    Bool,
    /// Square matrices. Rotation is the first thing anyone reaches for in a
    /// shader, and without these it can only be written by expanding the
    /// arithmetic by hand.
    Mat2,
    Mat3,
    Mat4,
    I32,
    U32,
    /// The type of an integer literal before anything has said what it is.
    ///
    /// WGSL calls this an abstract int and so does this compiler, for the same
    /// reason: `1 / 2` has to be `0.5` as it is in Lua, while `1 << 2` has to
    /// be four. A literal cannot be both until something decides, so it is
    /// neither until then, and [`Self::defaulted`] commits it to `f32` at the
    /// points where a concrete type is finally needed.
    ///
    /// This is not a convenience. A hash constant like `2654435769` does not
    /// survive as an `f32` — twenty-four bits of mantissa cannot hold
    /// thirty-two bits of constant — so without it there is no way to write a
    /// hash at all, and no way to write noise that is not the
    /// `sin(dot(p, k)) * 43758.5453` trick.
    AbstractInt,
    /// The type of an expression that already produced a diagnostic.
    ///
    /// It unifies with everything and reports nothing, so one mistake yields
    /// one message instead of a cascade down the rest of the expression.
    Poison,
}

impl Type {
    /// Parses the name a shader specification uses.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "f32" | "float" | "number" => Self::F32,
            "vec2" => Self::Vec2,
            "vec3" => Self::Vec3,
            "vec4" | "color" | "colour" => Self::Vec4,
            "mat2" | "mat2x2" => Self::Mat2,
            "mat3" | "mat3x3" => Self::Mat3,
            "mat4" | "mat4x4" => Self::Mat4,
            "bool" => Self::Bool,
            "i32" | "int" => Self::I32,
            "u32" | "uint" => Self::U32,
            _ => return None,
        })
    }

    /// The WGSL spelling.
    pub fn wgsl(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
            Self::Mat2 => "mat2x2<f32>",
            Self::Mat3 => "mat3x3<f32>",
            Self::Mat4 => "mat4x4<f32>",
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::U32 => "u32",
            // An abstract int that reached emission was committed first.
            Self::AbstractInt => "i32",
            // Poison never reaches emission: lowering fails first, and the
            // emitter only ever runs on a program that type-checked.
            Self::Poison => "f32",
        }
    }

    /// How many components the type holds, or one for a scalar.
    pub fn components(self) -> u8 {
        match self {
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
            _ => 1,
        }
    }

    /// The vector type with this many components.
    pub fn vector(components: u8) -> Option<Self> {
        Some(match components {
            1 => Self::F32,
            2 => Self::Vec2,
            3 => Self::Vec3,
            4 => Self::Vec4,
            _ => return None,
        })
    }

    pub fn is_vector(self) -> bool {
        matches!(self, Self::Vec2 | Self::Vec3 | Self::Vec4)
    }

    pub fn is_matrix(self) -> bool {
        matches!(self, Self::Mat2 | Self::Mat3 | Self::Mat4)
    }

    /// The square matrix of this many columns.
    pub fn matrix(columns: u8) -> Option<Self> {
        Some(match columns {
            2 => Self::Mat2,
            3 => Self::Mat3,
            4 => Self::Mat4,
            _ => return None,
        })
    }

    /// The column type of a matrix, which is also what it multiplies.
    pub fn column(self) -> Option<Self> {
        match self {
            Self::Mat2 => Some(Self::Vec2),
            Self::Mat3 => Some(Self::Vec3),
            Self::Mat4 => Some(Self::Vec4),
            _ => None,
        }
    }

    /// How many columns a matrix has.
    pub fn columns(self) -> u8 {
        match self {
            Self::Mat2 => 2,
            Self::Mat3 => 3,
            Self::Mat4 => 4,
            _ => 0,
        }
    }

    /// Whether componentwise arithmetic is defined on the type.
    ///
    /// A matrix is deliberately not numeric by this test: `matrix * vector` is
    /// a product, not a componentwise operation, and letting it through the
    /// same path would make `m * v` mean the wrong thing silently.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::F32
                | Self::Vec2
                | Self::Vec3
                | Self::Vec4
                | Self::I32
                | Self::U32
                | Self::AbstractInt
        )
    }

    /// Whether the type holds whole numbers and can be shifted and masked.
    pub fn is_integer(self) -> bool {
        matches!(self, Self::I32 | Self::U32 | Self::AbstractInt)
    }

    /// Commits an abstract integer literal to a concrete type.
    ///
    /// Called wherever a type must finally be one thing: a local's declared
    /// type, a return, a constructor argument. Defaulting to `f32` rather than
    /// `i32` is what keeps `local half = 1 / 2` equal to `0.5`, which is what
    /// Lua does and what a configuration author means.
    pub fn defaulted(self) -> Self {
        match self {
            Self::AbstractInt => Self::F32,
            other => other,
        }
    }

    /// Whether a value of this type can be used where `wanted` is expected.
    ///
    /// Only an abstract integer converts, and only because it has not decided
    /// what it is yet. Nothing else coerces silently: an `f32` where a `u32`
    /// belongs is a mistake worth a message.
    pub fn fits(self, wanted: Self) -> bool {
        self == wanted
            || self.is_poison()
            || wanted.is_poison()
            || (self == Self::AbstractInt && (wanted.is_numeric() || wanted.is_integer()))
    }

    /// Whether a diagnostic about this type would be noise.
    pub fn is_poison(self) -> bool {
        matches!(self, Self::Poison)
    }

    /// Size and alignment in a uniform block, in bytes.
    ///
    /// WGSL's rules: a scalar is four bytes aligned to four, a `vec2` is eight
    /// aligned to eight, and a `vec3` occupies twelve but aligns to sixteen —
    /// which is the one that surprises people, and the reason the packer in
    /// `emit` computes offsets rather than assuming them.
    pub fn layout(self) -> (u32, u32) {
        match self {
            Self::F32 | Self::I32 | Self::U32 | Self::AbstractInt | Self::Bool | Self::Poison => {
                (4, 4)
            }
            Self::Vec2 => (8, 8),
            Self::Vec3 => (12, 16),
            Self::Vec4 => (16, 16),
            // A matrix is its columns, each aligned as a vector of that width.
            // A `mat3x3` is therefore three sixteen-byte columns — forty-eight
            // bytes, not thirty-six — which is the layout rule that surprises
            // everybody and the reason the packer computes rather than assumes.
            Self::Mat2 => (16, 8),
            Self::Mat3 => (48, 16),
            Self::Mat4 => (64, 16),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::F32 => "f32",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Vec4 => "vec4",
            Self::Mat2 => "mat2",
            Self::Mat3 => "mat3",
            Self::Mat4 => "mat4",
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::AbstractInt => "integer",
            Self::Poison => "?",
        })
    }
}

/// A compile-time constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    F32(f32),
    /// An integer literal that has not been told what it is yet.
    ///
    /// Held as `i64` so a `u32` constant near the top of the range survives the
    /// trip: `2654435769` does not fit an `i32` and is perfectly ordinary in a
    /// hash.
    Int(i64),
    I32(i32),
    Bool(bool),
}

impl Value {
    pub fn ty(self) -> Type {
        match self {
            Self::F32(_) => Type::F32,
            Self::Int(_) => Type::AbstractInt,
            Self::I32(_) => Type::I32,
            Self::Bool(_) => Type::Bool,
        }
    }
}

/// What a shader is allowed to decide.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShaderKind {
    /// The field decides the shape; the shader decides the colour inside it.
    ///
    /// Clipping, damage, hit testing and the whole geometry path are untouched,
    /// which is why this is the cheap mode and the one to reach for first.
    #[default]
    Material,
    /// The shader decides coverage too, over the node's own rectangle.
    ///
    /// Geometry and shader stop composing here: a node cannot be both a star
    /// and whatever the shader draws. That is inherent to the mode, not a gap.
    Surface,
    /// The shader reads what is already rendered underneath it.
    ///
    /// Distortion, chromatic aberration, a custom blur. Needs the node to
    /// become a layer so there is something to sample.
    Effect,
}

impl ShaderKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "material" | "fill" => Self::Material,
            "surface" => Self::Surface,
            "effect" | "post" => Self::Effect,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Surface => "surface",
            Self::Effect => "effect",
        }
    }
}
