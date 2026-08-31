use mold_layout::Geometry;

use crate::{commands::*, effects::*};

/// How many layers one field may compose.
///
/// A composition is resolved in a single fragment shader, so every layer costs
/// every pixel of the node — the cap is what keeps a runaway configuration from
/// turning one node into an unbounded loop per fragment.
pub const MAX_FIELD_LAYERS: usize = 16;

/// One distance-field layer as the shader storage buffer holds it.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, Default, PartialEq)]
pub struct SdfFieldLayer {
    /// `[shape, morph_to, morph, operation]`.
    pub kinds: [f32; 4],
    /// Centre then half-extents, in the field's own space.
    pub rect: [f32; 4],
    /// `[unused, points, inner radius, thickness]`.
    ///
    /// The first slot is padding, not a corner radius: corners come through
    /// `radii`, and the shader has never read this one. The doc used to say
    /// otherwise, which is the sort of disagreement that survives precisely
    /// because nothing depends on it.
    pub params: [f32; 4],
    /// `[angle, rotation, blend, unused]`.
    pub extra: [f32; 4],
    /// Linear-light fill for this layer.
    pub color: [f32; 4],
    /// Corner radii, top-left clockwise.
    pub radii: [f32; 4],
}

/// One composed field as the shader instance buffer holds it.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, Default, PartialEq)]
pub struct SdfFieldInstance {
    /// Physical bounds: origin then size.
    pub bounds: [f32; 4],
    /// Fill colour.
    pub fill: [f32; 4],
    /// Outline colour.
    pub outline: [f32; 4],
    /// `[stroke width, softness, first layer, layer count]`.
    pub style: [f32; 4],
    /// Affine matrix, column major.
    pub transform: [f32; 4],
    /// Affine translation in `xy`.
    pub transform_offset: [f32; 4],
    /// Everything the surface can reach, in the node's own space: left, top,
    /// right, bottom.
    pub area: [f32; 4],
}

impl SdfFieldInstance {
    /// Converts a field command to physical instance and layer data.
    ///
    /// The layers are written in the field's own space — origin at the node's
    /// top-left corner — because that is the space the fragment shader walks,
    /// and it keeps a layer's numbers independent of where the node sits.
    pub fn from_command(
        command: &DrawCommand,
        scale_120: u32,
        layers: &mut Vec<SdfFieldLayer>,
    ) -> Option<Self> {
        let DrawCommand::Field {
            bounds,
            transform,
            fill_color,
            stroke_color,
            stroke_width,
            softness,
            layers: sources,
            ..
        } = command
        else {
            return None;
        };
        if sources.is_empty() {
            return None;
        }
        let scale = scale_120.max(1) as f64 / 120.0;
        let first = layers.len();
        for layer in sources.iter().take(MAX_FIELD_LAYERS) {
            layers.push(SdfFieldLayer {
                kinds: [
                    layer.shape.code() as f32,
                    layer.morph_to.code() as f32,
                    layer.morph.clamp(0.0, 1.0),
                    layer.operation.code() as f32,
                ],
                rect: [
                    ((layer.bounds.x - bounds.x + layer.bounds.width / 2.0) * scale) as f32,
                    ((layer.bounds.y - bounds.y + layer.bounds.height / 2.0) * scale) as f32,
                    ((layer.bounds.width / 2.0) * scale) as f32,
                    ((layer.bounds.height / 2.0) * scale) as f32,
                ],
                params: [
                    0.0,
                    layer.points,
                    layer.inner_radius,
                    (f64::from(layer.thickness) * scale) as f32,
                ],
                extra: [
                    layer.angle,
                    layer.rotation,
                    (f64::from(layer.blend) * scale) as f32,
                    0.0,
                ],
                color: color_array(layer.color),
                radii: layer.radii.map(|radius| (f64::from(radius) * scale) as f32),
            });
        }
        Some(Self {
            bounds: [
                (bounds.x * scale) as f32,
                (bounds.y * scale) as f32,
                (bounds.width * scale) as f32,
                (bounds.height * scale) as f32,
            ],
            fill: color_array(*fill_color),
            outline: color_array(*stroke_color),
            style: [
                (stroke_width * scale) as f32,
                (softness * scale) as f32,
                first as f32,
                (layers.len() - first) as f32,
            ],
            transform: [
                transform.matrix[0] as f32,
                transform.matrix[1] as f32,
                transform.matrix[2] as f32,
                transform.matrix[3] as f32,
            ],
            transform_offset: [
                (transform.matrix[4] * scale) as f32,
                (transform.matrix[5] * scale) as f32,
                0.0,
                0.0,
            ],
            area: field_area(*bounds, *stroke_width, *softness, sources, scale),
        })
    }
}

/// How far a composed surface may reach outside the node's own rectangle.
///
/// The outline straddles the zero crossing and the softened edge fades outwards
/// from it, but the larger term is the seam: a smooth operator pushes the
/// surface *out* where two shapes meet, by up to its blend radius. A quad sized
/// without it clips the bulge flat, which is what a fused row of cards looks
/// like when the top and bottom of the join are sliced off.
/// The rectangle a composed surface can reach, in the node's own space.
///
/// A layer is free to sit outside the node that composes it — a selection that
/// overhangs its bar, a badge growing out past the edge — and the seam widens
/// the surface further still. Drawing into a quad sized to the node alone
/// slices all of that off, which is what a shape clipped flat on one side is.
/// The rectangle a field can actually paint into, in surface coordinates.
///
/// The layers alone, not the node they sit in: a composition paints nothing
/// where no layer reaches — every operator starts from "infinitely far
/// outside" — so covering the node's own rectangle is fragments spent to decide
/// that a pixel is empty. It is the difference between a fullscreen field
/// costing the screen and costing the shapes.
///
/// One function, because this used to be written twice — once to size the quad
/// and once to compute damage — and the two had already drifted: only one of
/// them accounted for a layer's rotation, so a rotated shape was drawn whole
/// and then damaged as though it were not.
pub fn field_reach(stroke_width: f64, softness: f64, layers: &[SdfLayer]) -> Option<Geometry> {
    let spread = field_spread(stroke_width, softness, layers);
    let mut left = f64::MAX;
    let mut top = f64::MAX;
    let mut right = f64::MIN;
    let mut bottom = f64::MIN;
    // Only the layers that are actually uploaded. Beyond `MAX_FIELD_LAYERS`
    // the shader never sees them, so reserving room for one would be room to
    // draw something that cannot appear.
    for layer in layers.iter().take(MAX_FIELD_LAYERS) {
        let (reach_x, reach_y) = rotated_half_extents(layer);
        let centre_x = layer.bounds.x + layer.bounds.width / 2.0;
        let centre_y = layer.bounds.y + layer.bounds.height / 2.0;
        left = left.min(centre_x - reach_x);
        top = top.min(centre_y - reach_y);
        right = right.max(centre_x + reach_x);
        bottom = bottom.max(centre_y + reach_y);
    }
    if left > right || top > bottom {
        return None;
    }
    Some(Geometry {
        x: left - spread,
        y: top - spread,
        width: (right - left) + spread * 2.0,
        height: (bottom - top) + spread * 2.0,
    })
}

fn field_area(
    bounds: Geometry,
    stroke_width: f64,
    softness: f64,
    layers: &[SdfLayer],
    scale: f64,
) -> [f32; 4] {
    let Some(reach) = field_reach(stroke_width, softness, layers) else {
        return [0.0; 4];
    };
    // The quad is expressed relative to the node it belongs to.
    [
        ((reach.x - bounds.x) * scale) as f32,
        ((reach.y - bounds.y) * scale) as f32,
        ((reach.x - bounds.x + reach.width) * scale) as f32,
        ((reach.y - bounds.y + reach.height) * scale) as f32,
    ]
}

/// How far a layer reaches from its own centre, once it has been rotated.
///
/// The shader rotates the sample point into each layer's frame, so a rotated
/// layer covers a different rectangle than the one it was given — and the quad
/// is built from those rectangles. Taking the unrotated bounds meant a rotated
/// non-square layer was sliced flat by the very quad meant to contain it.
fn rotated_half_extents(layer: &SdfLayer) -> (f64, f64) {
    let half_width = layer.bounds.width / 2.0;
    let half_height = layer.bounds.height / 2.0;
    if layer.rotation == 0.0 {
        return (half_width, half_height);
    }
    let (sin, cos) = f64::from(layer.rotation).to_radians().sin_cos();
    (
        half_width * cos.abs() + half_height * sin.abs(),
        half_width * sin.abs() + half_height * cos.abs(),
    )
}

pub fn field_spread(stroke_width: f64, softness: f64, layers: &[SdfLayer]) -> f64 {
    let blend = layers
        .iter()
        .map(|layer| f64::from(layer.blend))
        .fold(0.0, f64::max);
    stroke_width.max(0.0) / 2.0 + softness.max(0.0) + blend
}
