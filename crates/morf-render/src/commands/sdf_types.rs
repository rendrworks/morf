use morf_layout::Geometry;
// The one shape vocabulary, shared with the input-region rasteriser so a
// star-shaped node is clickable as a star. Re-exported, so naming a shape does
// not oblige a caller to depend on `morf-region` directly.
pub use morf_region::{Operation, Shape, ShapeParams};
use morf_scene::Color;

/// One analytic distance field, and how it joins the composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfLayer {
    /// Layer rectangle in logical surface coordinates.
    pub bounds: Geometry,
    /// Resolved fill for this layer, already fallen back to the field's own.
    pub color: Color,
    /// Shape at `morph` of zero.
    pub shape: Shape,
    /// Shape at `morph` of one.
    pub morph_to: Shape,
    /// Position between the two fields, clamped to zero through one.
    pub morph: f32,
    /// How this layer joins the ones before it.
    pub operation: Operation,
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
    /// The letter this layer is, for `Polygon`.
    ///
    /// A glyph is not a family of shape with parameters — it is one particular
    /// outline — so it reaches the composition as a character and is resolved
    /// to points when the frame is gathered, where the fonts are. One character
    /// rather than a string: a layer is one shape, and a word is a row of them.
    pub glyph: Option<char>,
    /// The letter it is turning into, interpolated at `morph`.
    ///
    /// The points are walked to their opposite numbers on the CPU and the
    /// result is one outline, so a morphing letter costs the composition
    /// exactly what a still one does.
    pub glyph_morph_to: Option<char>,
}
