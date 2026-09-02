use crate::{commands::*, effects::*};
use glyph_layer::polygon_params;
use morf_region::Shape;

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
    /// `[outline start, points, inner radius, thickness]`.
    ///
    /// The first slot was padding — corners come through `radii` — and now
    /// carries where a polygon layer's outline points begin. It is the one slot
    /// in here nothing else wanted.
    pub params: [f32; 4],
    /// `[angle, rotation, blend, outline loop count]`.
    pub extra: [f32; 4],
    /// Linear-light fill for this layer.
    pub color: [f32; 4],
    /// Corner radii, top-left clockwise.
    pub radii: [f32; 4],
}

/// How an outline sits against the shape's edge.
///
/// One outline serves both the inset border a rectangle has always drawn and
/// the centred stroke a field has always drawn — they were the same band of
/// pixels described by two shaders.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BorderAlignment {
    /// Entirely inside the shape, which is what a rectangle border means.
    #[default]
    Inside,
    /// Straddling the crossing, so widening it does not move the edge.
    Centred,
    /// Entirely outside the shape.
    Outside,
}

impl BorderAlignment {
    pub(crate) fn code(self) -> f32 {
        self as u32 as f32
    }
}

/// Everything about a field's surface that is not its shape.
///
/// One per instance, read by instance index. It is a storage buffer rather
/// than more vertex attributes because the quad pipeline this pass absorbed
/// already used sixteen of them, and sixteen is the limit.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, Default, PartialEq)]
pub struct SdfFieldMaterial {
    /// `[alignment, antialiased, unused, unused]`. The width itself rides in
    /// the instance's `style.x`, where the host already needs it to size the
    /// quad the field is drawn into.
    pub border: [f32; 4],
    pub border_color: [f32; 4],
    /// `[offset x, offset y, inner, unused]`.
    pub shadow: [f32; 4],
    pub shadow_color: [f32; 4],
    /// `[unused, shadow blur, shadow spread, conic rotation]`.
    pub effects: [f32; 4],
    pub gradient_start_color: [f32; 4],
    pub gradient_end_color: [f32; 4],
    /// Normalised start and end of a linear gradient.
    pub gradient_points: [f32; 4],
    /// `[kind, centre x, centre y, radius]`.
    pub gradient_data: [f32; 4],
    pub color_overlay: [f32; 4],
    /// The rectangle a gradient is measured across, in the field's own space.
    pub shape: [f32; 4],
}

impl SdfFieldMaterial {
    /// The material a plain composed field wants: an antialiased outline, no
    /// gradient, no shadow.
    pub fn plain(alignment: BorderAlignment, shape: [f32; 4]) -> Self {
        Self {
            border: [alignment.code(), 1.0, 0.0, 0.0],
            shape,
            ..Self::default()
        }
    }
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
    /// Converts a field or quad command to physical instance, layer and
    /// material data.
    ///
    /// Both go through this pass. A rectangle is a field of one `Box` layer —
    /// it always was, once the shape it drew came from the same distance
    /// function — and giving it its own pipeline only meant that a gradient, a
    /// border and a shadow were things a rectangle could have and a composed
    /// shape could not.
    pub fn from_command(
        command: &DrawCommand,
        scale_120: u32,
        layers: &mut Vec<SdfFieldLayer>,
        materials: &mut Vec<SdfFieldMaterial>,
        outlines: &mut Vec<[f32; 2]>,
        text: &mut morf_text::TextSystem,
    ) -> Option<Self> {
        match command {
            DrawCommand::Field { .. } => {
                Self::from_field(command, scale_120, layers, materials, outlines, text)
            }
            DrawCommand::Quad { .. } => Self::from_quad(command, scale_120, layers, materials),
            _ => None,
        }
    }

    /// The layers are written in the field's own space — origin at the node's
    /// top-left corner — because that is the space the fragment shader walks,
    /// and it keeps a layer's numbers independent of where the node sits.
    fn from_field(
        command: &DrawCommand,
        scale_120: u32,
        layers: &mut Vec<SdfFieldLayer>,
        materials: &mut Vec<SdfFieldMaterial>,
        outlines: &mut Vec<[f32; 2]>,
        text: &mut morf_text::TextSystem,
    ) -> Option<Self> {
        let DrawCommand::Field {
            bounds,
            transform,
            fill_color,
            stroke_color,
            stroke_width,
            stroke_alignment,
            softness,
            gradient,
            color_overlay,
            shadow_color,
            shadow_blur,
            shadow_spread,
            shadow_offset_x,
            shadow_offset_y,
            shadow_inner,
            shader,
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
            let outline = polygon_params(layer, scale, outlines, text);
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
                params: outline.0,
                extra: [
                    layer.angle,
                    layer.rotation,
                    (f64::from(layer.blend) * scale) as f32,
                    outline.1,
                ],
                color: color_array(layer.color),
                radii: layer.radii.map(|radius| (f64::from(radius) * scale) as f32),
            });
        }
        let (gradient_start_color, gradient_end_color, gradient_points, gradient_data, angle) =
            gradient_instance(gradient);
        // A field's outline straddles the crossing by default and a rectangle's
        // sits inside it, but both are the one outline the shader now has, and
        // either can say which it wants.
        materials.push(SdfFieldMaterial {
            border: [stroke_alignment.code(), 1.0, 0.0, 0.0],
            border_color: color_array(*stroke_color),
            shadow: [
                (shadow_offset_x * scale) as f32,
                (shadow_offset_y * scale) as f32,
                if *shadow_inner { 1.0 } else { 0.0 },
                0.0,
            ],
            shadow_color: color_array(*shadow_color),
            effects: [
                0.0,
                (shadow_blur * scale) as f32,
                (shadow_spread * scale) as f32,
                angle,
            ],
            gradient_start_color,
            gradient_end_color,
            gradient_points,
            gradient_data,
            color_overlay: color_array(*color_overlay),
            shape: [
                0.0,
                0.0,
                (bounds.width * scale) as f32,
                (bounds.height * scale) as f32,
            ],
        });
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
            // A shader that owns its coverage paints across the whole node,
            // so it is handed the whole node: the layers only say where the
            // shape it replaced would have reached.
            area: if shader.as_ref().is_some_and(|shader| shader.owns_coverage) {
                [
                    0.0,
                    0.0,
                    (bounds.width * scale) as f32,
                    (bounds.height * scale) as f32,
                ]
            } else {
                field_area(
                    *bounds,
                    *stroke_width,
                    *softness,
                    sources,
                    scale,
                    // An inner shadow falls inside the surface, so it needs no room.
                    (shadow_color.alpha > 0.0 && !*shadow_inner).then_some(ShadowReach {
                        offset_x: *shadow_offset_x,
                        offset_y: *shadow_offset_y,
                        blur: *shadow_blur,
                        spread: *shadow_spread,
                    }),
                )
            },
        })
    }

    /// A rectangle, as one `Box` layer of a field.
    ///
    /// Everything a quad could say that a field could not — the gradient, the
    /// inset border, the two shadow modes, the colour overlay — now says it
    /// through the material, which every field has.
    fn from_quad(
        command: &DrawCommand,
        scale_120: u32,
        layers: &mut Vec<SdfFieldLayer>,
        materials: &mut Vec<SdfFieldMaterial>,
    ) -> Option<Self> {
        let DrawCommand::Quad {
            bounds,
            transform,
            color,
            color_overlay,
            gradient,
            radii,
            border_width,
            antialiasing,
            border_pixel_aligned,
            border_color,
            blur,
            shadow_color,
            shadow_blur,
            shadow_spread,
            shadow_offset_x,
            shadow_offset_y,
            shadow_inner,
            shader,
            ..
        } = command
        else {
            return None;
        };
        let scale = scale_120.max(1) as f64 / 120.0;
        let width = (bounds.width * scale) as f32;
        let height = (bounds.height * scale) as f32;
        let first = layers.len();
        layers.push(SdfFieldLayer {
            kinds: [Shape::Box.code() as f32, Shape::Box.code() as f32, 0.0, 0.0],
            rect: [width / 2.0, height / 2.0, width / 2.0, height / 2.0],
            params: [0.0; 4],
            extra: [0.0; 4],
            color: color_array(*color),
            radii: radii.map(|radius| (radius.max(0.0) * scale) as f32),
        });
        let (gradient_start_color, gradient_end_color, gradient_points, gradient_data, angle) =
            gradient_instance(gradient);
        materials.push(SdfFieldMaterial {
            border: [
                BorderAlignment::Inside.code(),
                if *antialiasing { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
            border_color: color_array(*border_color),
            shadow: [
                (*shadow_offset_x * scale) as f32,
                (*shadow_offset_y * scale) as f32,
                if *shadow_inner { 1.0 } else { 0.0 },
                0.0,
            ],
            shadow_color: color_array(*shadow_color),
            effects: [
                0.0,
                (*shadow_blur * scale) as f32,
                (*shadow_spread * scale) as f32,
                angle,
            ],
            gradient_start_color,
            gradient_end_color,
            gradient_points,
            gradient_data,
            color_overlay: color_array(*color_overlay),
            shape: [0.0, 0.0, width, height],
        });
        // The quad the fragment shader walks has to reach everything the
        // effects do: the blurred edge, and an outer shadow's offset, blur and
        // spread. `effect_bounds` already knows that arithmetic; this only
        // restates its answer in the node's own frame, which is the frame a
        // field's `area` is expressed in.
        let expanded = effect_bounds(
            *bounds,
            *blur,
            if *shadow_inner { 0.0 } else { *shadow_blur },
            if *shadow_inner { 0.0 } else { *shadow_spread },
            if *shadow_inner { 0.0 } else { *shadow_offset_x },
            if *shadow_inner { 0.0 } else { *shadow_offset_y },
        );
        Some(Self {
            bounds: [
                (bounds.x * scale) as f32,
                (bounds.y * scale) as f32,
                width,
                height,
            ],
            fill: color_array(*color),
            outline: color_array(*border_color),
            style: [
                if *border_pixel_aligned {
                    (*border_width * scale).round() as f32
                } else {
                    (*border_width * scale) as f32
                },
                (*blur * scale) as f32,
                first as f32,
                1.0,
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
            // A surface shader on a rectangle owns the whole node, exactly as
            // it does on a field: it is deciding coverage, so the effect
            // expansion is not what bounds it.
            area: if shader.as_ref().is_some_and(|shader| shader.owns_coverage) {
                [0.0, 0.0, width, height]
            } else {
                [
                    ((expanded.x - bounds.x) * scale) as f32,
                    ((expanded.y - bounds.y) * scale) as f32,
                    ((expanded.x + expanded.width - bounds.x) * scale) as f32,
                    ((expanded.y + expanded.height - bounds.y) * scale) as f32,
                ]
            },
        })
    }
}

pub(crate) mod glyph_layer;
mod reach;

pub use reach::*;
