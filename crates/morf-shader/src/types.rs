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
    /// Loop counters only. A configuration never writes this type by name; it
    /// exists so a numeric `for` can count without floating-point drift.
    I32,
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
            "bool" => Self::Bool,
            "i32" | "int" => Self::I32,
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
            Self::Bool => "bool",
            Self::I32 => "i32",
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

    /// Whether arithmetic is defined on the type at all.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::F32 | Self::Vec2 | Self::Vec3 | Self::Vec4 | Self::I32
        )
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
            Self::F32 | Self::I32 | Self::Bool | Self::Poison => (4, 4),
            Self::Vec2 => (8, 8),
            Self::Vec3 => (12, 16),
            Self::Vec4 => (16, 16),
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
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::Poison => "?",
        })
    }
}

/// A compile-time constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    F32(f32),
    I32(i32),
    Bool(bool),
}

impl Value {
    pub fn ty(self) -> Type {
        match self {
            Self::F32(_) => Type::F32,
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
