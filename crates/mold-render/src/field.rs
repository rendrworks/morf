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
    /// `[corner radius, points, inner radius, thickness]`.
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
    /// Affine translation in `xy`; `z` is how far the surface may reach outside
    /// the node — outline, softened edge, and the bulge a smooth seam adds.
    pub transform_offset: [f32; 4],
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
                (field_spread(*stroke_width, *softness, sources) * scale) as f32,
                0.0,
            ],
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
pub fn field_spread(stroke_width: f64, softness: f64, layers: &[SdfLayer]) -> f64 {
    let blend = layers
        .iter()
        .map(|layer| f64::from(layer.blend))
        .fold(0.0, f64::max);
    stroke_width.max(0.0) / 2.0 + softness.max(0.0) + blend
}
