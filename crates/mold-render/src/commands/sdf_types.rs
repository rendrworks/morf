use mold_layout::Geometry;
use mold_scene::Color;

/// One analytic shape family a distance-field layer can take.
///
/// Each is a closed-form distance function evaluated per fragment, so the edge
/// stays exact at any scale and two of them can be interpolated as *fields*
/// rather than as outlines — which is what lets a morph change topology.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SdfShapeKind {
    #[default]
    Circle,
    /// Rectangle with a uniform corner radius.
    Box,
    /// Stadium: a rectangle with fully rounded ends.
    Capsule,
    Triangle,
    Hexagon,
    /// `points`-pointed star, waisted by `inner_radius`.
    Star,
    /// Annulus of the given `thickness`.
    Ring,
    /// Circular sector spanning `angle` degrees, centred on straight up.
    Pie,
    /// Plus sign with arms `thickness` wide.
    Cross,
}

impl SdfShapeKind {
    /// Parses the name a configuration uses.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "circle" => Self::Circle,
            "box" | "rect" | "rectangle" => Self::Box,
            "capsule" | "pill" | "stadium" => Self::Capsule,
            "triangle" => Self::Triangle,
            "hexagon" => Self::Hexagon,
            "star" => Self::Star,
            "ring" | "annulus" => Self::Ring,
            "pie" | "sector" => Self::Pie,
            "cross" | "plus" => Self::Cross,
            _ => return None,
        })
    }

    /// The discriminant the shader switches on.
    pub fn code(self) -> u32 {
        self as u32
    }
}

/// How a layer combines with everything composed before it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SdfOperation {
    #[default]
    Union,
    Subtract,
    Intersect,
    /// Union whose seam is rounded over `blend` pixels.
    SmoothUnion,
    /// Subtraction whose seam is rounded over `blend` pixels.
    SmoothSubtract,
    /// Intersection whose seam is rounded over `blend` pixels.
    SmoothIntersect,
}

impl SdfOperation {
    /// Parses the name a configuration uses.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "union" => Self::Union,
            "subtract" | "difference" => Self::Subtract,
            "intersect" | "intersection" => Self::Intersect,
            "smooth_union" => Self::SmoothUnion,
            "smooth_subtract" => Self::SmoothSubtract,
            "smooth_intersect" => Self::SmoothIntersect,
            _ => return None,
        })
    }

    /// The discriminant the shader switches on.
    pub fn code(self) -> u32 {
        self as u32
    }
}

/// One analytic distance field, and how it joins the composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfLayer {
    /// Layer rectangle in logical surface coordinates.
    pub bounds: Geometry,
    /// Resolved fill for this layer, already fallen back to the field's own.
    pub color: Color,
    /// Shape at `morph` of zero.
    pub shape: SdfShapeKind,
    /// Shape at `morph` of one.
    pub morph_to: SdfShapeKind,
    /// Position between the two fields, clamped to zero through one.
    pub morph: f32,
    /// How this layer joins the ones before it.
    pub operation: SdfOperation,
    /// Seam radius for a smooth operation, in logical pixels.
    pub blend: f32,
    /// Rotation about the layer centre, in degrees.
    pub rotation: f32,
    /// Corner radii — top-left, top-right, bottom-right, bottom-left — for the
    /// shapes that have corners. A rect absorbed into a field keeps all four.
    pub radii: [f32; 4],
    /// Point count, for `Star`.
    pub points: f32,
    /// Waist as a fraction of the outer radius, for `Star`.
    pub inner_radius: f32,
    /// Arm or wall thickness, for `Ring` and `Cross`.
    pub thickness: f32,
    /// Sector sweep in degrees, for `Pie`.
    pub angle: f32,
}
