//! How far a field reaches past the node that owns it.
//!
//! The quad the fragment shader walks is built from these numbers, so anything
//! this understates is silently clipped: a rotated layer, a smooth seam's
//! bulge, an outline, or — since the pipelines merged — a shadow.

use mold_layout::Geometry;

use crate::commands::*;
use crate::field::MAX_FIELD_LAYERS;

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

#[allow(clippy::too_many_arguments)]
pub(crate) fn field_area(
    bounds: Geometry,
    stroke_width: f64,
    softness: f64,
    layers: &[SdfLayer],
    scale: f64,
    shadow: Option<ShadowReach>,
) -> [f32; 4] {
    let Some(reach) = field_reach(stroke_width, softness, layers) else {
        return [0.0; 4];
    };
    // A shadow is the same surface moved and dilated, so it reaches as far as
    // the surface does plus the offset, the spread and the blurred edge. Left
    // out, the quad clips a field's own shadow off — which is the bug the
    // rectangle path had already solved with `effect_bounds` and the field path
    // had never needed until it could cast one.
    let reach = match shadow {
        None => reach,
        Some(shadow) => {
            let grown = shadow.blur + shadow.spread;
            let left = (reach.x).min(reach.x + shadow.offset_x - grown);
            let top = (reach.y).min(reach.y + shadow.offset_y - grown);
            let right =
                (reach.x + reach.width).max(reach.x + reach.width + shadow.offset_x + grown);
            let bottom =
                (reach.y + reach.height).max(reach.y + reach.height + shadow.offset_y + grown);
            Geometry {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            }
        }
    };
    // The quad is expressed relative to the node it belongs to.
    [
        ((reach.x - bounds.x) * scale) as f32,
        ((reach.y - bounds.y) * scale) as f32,
        ((reach.x - bounds.x + reach.width) * scale) as f32,
        ((reach.y - bounds.y + reach.height) * scale) as f32,
    ]
}

/// How far past the surface an outer shadow falls.
#[derive(Clone, Copy, Debug)]
pub struct ShadowReach {
    pub(crate) offset_x: f64,
    pub(crate) offset_y: f64,
    pub(crate) blur: f64,
    pub(crate) spread: f64,
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
