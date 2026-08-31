use morf_layout::{Geometry, Layout, TextAlignment, TextElide, Transform2D};
use morf_scene::{Color, NodeHandle, Scene};
use std::ops::Range;

use crate::{effects::*, field::*, paint::*, sdf::*};

mod sdf_types;

pub use sdf_types::*;

/// One ordered paint operation emitted from the scene graph.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    /// SDF rounded rectangle and border.
    Quad {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// Intersected ancestor clip in logical surface coordinates.
        clip: Option<Geometry>,
        /// Fill colour after node opacity.
        color: Color,
        /// Inherited colour overlay.
        color_overlay: Color,
        /// Optional normalized gradient fill.
        gradient: Gradient,
        /// Corner radii in top-left clockwise order.
        radii: [f64; 4],
        /// Border width.
        border_width: f64,
        /// If rectangle edges use smooth coverage.
        antialiasing: bool,
        /// If the border width is rounded in physical pixels.
        border_pixel_aligned: bool,
        /// Border colour after node opacity.
        border_color: Color,
        /// Fill-edge blur radius.
        blur: f64,
        /// Outer shadow colour.
        shadow_color: Color,
        /// Shadow blur radius.
        shadow_blur: f64,
        /// Shadow expansion around the rectangle.
        shadow_spread: f64,
        /// Shadow horizontal displacement.
        shadow_offset_x: f64,
        /// Shadow vertical displacement.
        shadow_offset_y: f64,
        /// Draw the shadow inside the rectangle edge.
        shadow_inner: bool,
    },
    /// Shaped glyph run owned by the text subsystem.
    Text {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// Intersected ancestor clip in logical surface coordinates.
        clip: Option<Geometry>,
        /// UTF-8 text used to locate its shaped buffer.
        text: String,
        /// Font family used by the shaping cache.
        family: String,
        /// Optional local font file or directory.
        font_source: String,
        /// Logical font size.
        size: f64,
        /// Numeric OpenType font weight.
        font_weight: f64,
        /// Glyph colour after node opacity.
        color: Color,
        /// Inherited colour overlay.
        color_overlay: Color,
        /// Whether lines wrap at the resolved width.
        wrap: bool,
        /// Ellipsis placement for an overflowing unwrapped line.
        elide: TextElide,
        /// Horizontal line alignment.
        horizontal_alignment: TextAlignment,
        /// Vertical placement inside the resolved height.
        vertical_alignment: VerticalAlignment,
        /// How the glyph field is thresholded: edge, softness and outline.
        field_style: DistanceFieldStyle,
    },
    /// Rasterized image or theme icon.
    Texture {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// Intersected ancestor clip in logical surface coordinates.
        clip: Option<Geometry>,
        /// Image path or icon name.
        source: String,
        /// Theme name for an icon command.
        icon_theme: Option<String>,
        /// Inherited colour overlay.
        color_overlay: Color,
        /// Aspect-ratio policy inside the resolved bounds.
        fill_mode: ImageFillMode,
        /// Interpret source alpha as a cached signed distance field mask.
        distance_field: bool,
        /// Pixel distance represented on either side of the mask edge.
        distance_field_spread: f32,
        /// Edge shaping applied to the sampled field.
        distance_field_style: DistanceFieldStyle,
    },
    /// Composed signed-distance field resolved in one fragment shader.
    Field {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// Intersected ancestor clip in logical surface coordinates.
        clip: Option<Geometry>,
        /// Fill colour after node opacity.
        fill_color: Color,
        /// Outline colour after node opacity.
        stroke_color: Color,
        /// Logical outline width.
        stroke_width: f64,
        /// Where that outline sits against the edge.
        stroke_alignment: BorderAlignment,
        /// Extra edge softness in logical pixels.
        softness: f64,
        /// Gradient across the node's own rectangle, if any.
        gradient: Gradient,
        /// Multiplied over the finished surface.
        color_overlay: Color,
        /// Drop shadow colour; fully transparent means no shadow.
        shadow_color: Color,
        /// Shadow edge softness in logical pixels.
        shadow_blur: f64,
        /// How far the shadow is dilated past the shape.
        shadow_spread: f64,
        shadow_offset_x: f64,
        shadow_offset_y: f64,
        /// Whether the shadow falls inside the shape rather than behind it.
        shadow_inner: bool,
        /// Layers in composition order; the first establishes the field.
        layers: Vec<SdfLayer>,
    },
}

/// Edge shaping applied when sampling a cached distance field.
///
/// The field is a continuous distance, not a coverage mask, so where the edge
/// sits and how sharply it falls off are decisions taken at sampling time. That
/// makes every one of these an ordinary animatable scene property rather than a
/// property of the cached texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceFieldStyle {
    /// How far to move the edge, in logical pixels, from where the shape says.
    ///
    /// Positive thickens and negative thins, the way a variable font gains or
    /// loses weight, and zero is the shape as drawn. Logical pixels rather than
    /// normalised field units because that is what a configuration can reason
    /// about: half a pixel more weight means the same thing at every size.
    ///
    /// This used to be an absolute threshold neutral at `0.5` for images and a
    /// signed offset neutral at `0.0` for text — one field, two unit systems,
    /// told apart only by which function had filled it in, with a `Default`
    /// that was right for one of them and wrong for the other.
    pub thickness: f32,
    /// Extra edge feathering in source pixels, on top of pixel-derived coverage.
    pub softness: f32,
    /// Outline band drawn outside the fill edge, in source pixels.
    pub outline_width: f32,
    /// Outline colour, composited beneath the fill.
    pub outline_color: Color,
}

impl Default for DistanceFieldStyle {
    fn default() -> Self {
        Self {
            thickness: 0.0,
            softness: 0.0,
            outline_width: 0.0,
            outline_color: Color::rgba8(0, 0, 0, 0),
        }
    }
}

/// Image placement policy inside resolved node bounds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ImageFillMode {
    #[default]
    Stretch,
    PreserveAspectFit,
    PreserveAspectCrop,
}

/// Vertical positioning for shaped text inside its node bounds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VerticalAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Gradient fill encoded in normalized rectangle coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    /// Use the rectangle's solid colour.
    None,
    /// Interpolate along a line.
    Linear {
        start_color: Color,
        end_color: Color,
        start: [f64; 2],
        end: [f64; 2],
    },
    /// Interpolate outwards from a center point.
    Radial {
        start_color: Color,
        end_color: Color,
        center: [f64; 2],
        radius: f64,
    },
    /// Interpolate around a center point from an angle in degrees.
    Conical {
        start_color: Color,
        end_color: Color,
        center: [f64; 2],
        angle: f64,
    },
}

impl DrawCommand {
    pub(crate) fn node(&self) -> NodeHandle {
        match self {
            Self::Quad { node, .. }
            | Self::Text { node, .. }
            | Self::Texture { node, .. }
            | Self::Field { node, .. } => *node,
        }
    }

    pub(crate) fn bounds(&self) -> Geometry {
        let bounds = match self {
            Self::Quad {
                bounds,
                transform,
                blur,
                shadow_blur,
                shadow_spread,
                shadow_offset_x,
                shadow_offset_y,
                shadow_inner,
                ..
            } => transform.bounds(effect_bounds(
                *bounds,
                *blur,
                if *shadow_inner { 0.0 } else { *shadow_blur },
                if *shadow_inner { 0.0 } else { *shadow_spread },
                if *shadow_inner { 0.0 } else { *shadow_offset_x },
                if *shadow_inner { 0.0 } else { *shadow_offset_y },
            )),
            Self::Text {
                bounds, transform, ..
            }
            | Self::Texture {
                bounds, transform, ..
            } => transform.bounds(*bounds),
            Self::Field {
                transform,
                stroke_width,
                softness,
                layers: sources,
                ..
            } => {
                // One computation, shared with the quad the shader is given.
                // Written out separately here, the two drifted: this copy took
                // the layer rectangles unrotated, so a rotated shape was drawn
                // whole and damaged as though it were not.
                let Some(reach) = field_reach(*stroke_width, *softness, sources) else {
                    return Geometry::default();
                };
                transform.bounds(reach)
            }
        };
        self.clip()
            .map_or(bounds, |clip| intersect_geometry(bounds, clip))
    }

    pub(crate) fn clip(&self) -> Option<Geometry> {
        match self {
            Self::Quad { clip, .. }
            | Self::Text { clip, .. }
            | Self::Texture { clip, .. }
            | Self::Field { clip, .. } => *clip,
        }
    }
}

/// Ordered commands for one surface frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    /// Back-to-front paint operations.
    pub commands: Vec<DrawCommand>,
    /// Nested offscreen subtree layers.
    pub layers: Vec<Layer>,
}

/// One subtree rendered into an offscreen target before composition.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Scene node that owns the layer.
    pub node: NodeHandle,
    /// Contiguous command range contained by the subtree.
    pub commands: Range<usize>,
    /// Containing layer, if nested.
    pub parent: Option<usize>,
    /// Opacity applied once while compositing the complete subtree.
    pub opacity: f32,
    /// Logical dual-kawase blur radius.
    pub blur: f32,
    /// Colour applied to the blurred subtree alpha behind the layer.
    pub shadow_color: Color,
    /// Logical dual-kawase shadow radius.
    pub shadow_blur: f32,
    /// Logical shadow displacement.
    pub shadow_offset: [f32; 2],
    /// Rounded owner geometry used to mask the composited subtree.
    pub mask: Option<LayerMask>,
    /// Logical bounds affected by this layer.
    pub bounds: Geometry,
}

/// Rounded geometry applied while compositing an offscreen layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerMask {
    /// Owner geometry before its transform.
    pub bounds: Geometry,
    /// Composed owner transform.
    pub transform: Transform2D,
    /// Corner radii in top-left clockwise order.
    pub radii: [f64; 4],
}

impl DrawList {
    /// Builds a draw list from resolved scene geometry.
    pub fn from_scene(scene: &Scene, layout: &Layout) -> Result<Self, RenderError> {
        let mut list = Self::default();
        list.rebuild(scene, layout)?;
        Ok(list)
    }

    /// Refills this list from the scene, keeping the memory it already holds.
    ///
    /// A command is 350-odd bytes and a busy surface has thousands of them, so
    /// a list built afresh every frame is hundreds of kilobytes allocated, filled
    /// and returned to the allocator sixty times a second. Reusing the buffer
    /// keeps the capacity and the pages, which is most of the cost once a scene
    /// is large enough to leave the cache.
    pub fn rebuild(&mut self, scene: &Scene, layout: &Layout) -> Result<(), RenderError> {
        self.commands.clear();
        self.layers.clear();
        let list = self;
        for root in scene.roots() {
            append_node(
                scene,
                layout,
                root,
                PaintContext {
                    transform: Transform2D::IDENTITY,
                    clip: None,
                    overlay: Color::rgba8(0, 0, 0, 0),
                    layer: None,
                    in_field: false,
                },
                list,
            )?;
        }
        Ok(())
    }
}
