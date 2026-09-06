//! A gradient as the material carries it: stops packed for the shader.

use morf_scene::{ColorSpace, Gradient, GradientKind, MAX_GRADIENT_STOPS};

use crate::effects::color_array;

/// The gradient half of a material, laid out as the shader reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GradientMaterial {
    /// `[kind, centre x, centre y, radius]`; kind is 0 none, 1 linear, 2
    /// radial, 3 conic, and the centre and radius are fractions of the shape.
    pub gradient: [f32; 4],
    /// `[angle in radians, stop count, space, unused]`; space is 0 sRGB, 1
    /// OkLab, 2 OkLCh.
    pub gradient_extra: [f32; 4],
    /// Stop positions, four to a vector.
    pub gradient_positions: [[f32; 4]; 4],
    /// Linear-light stop colours, straight alpha.
    pub gradient_colors: [[f32; 4]; MAX_GRADIENT_STOPS],
}

/// Packs a gradient for the shader; none packs as all zeros, which the shader
/// reads as the flat fill.
pub(crate) fn gradient_material(gradient: Option<&Gradient>) -> GradientMaterial {
    let Some(gradient) = gradient else {
        return GradientMaterial::default();
    };
    let kind = match gradient.kind {
        GradientKind::Linear => 1.0,
        GradientKind::Radial => 2.0,
        GradientKind::Conic => 3.0,
    };
    let space = match gradient.space {
        ColorSpace::Srgb => 0.0,
        ColorSpace::Oklab => 1.0,
        ColorSpace::Oklch => 2.0,
    };
    let [x, y] = gradient.at;
    // A radial gradient with no radius reaches the farthest corner, as a
    // stylesheet's does.
    let radius = gradient.radius.unwrap_or_else(|| {
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
            .iter()
            .map(|[cx, cy]| ((cx - x).powi(2) + (cy - y).powi(2)).sqrt())
            .fold(0.0, f64::max)
    });
    let mut material = GradientMaterial {
        gradient: [kind, x as f32, y as f32, radius as f32],
        gradient_extra: [
            gradient.angle.to_radians() as f32,
            gradient.stops.len().min(MAX_GRADIENT_STOPS) as f32,
            space,
            0.0,
        ],
        ..GradientMaterial::default()
    };
    for (index, stop) in gradient.stops.iter().take(MAX_GRADIENT_STOPS).enumerate() {
        material.gradient_positions[index / 4][index % 4] = stop.position as f32;
        material.gradient_colors[index] = color_array(stop.color);
    }
    material
}
