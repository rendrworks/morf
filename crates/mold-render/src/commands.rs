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
        /// Effective node opacity.
        opacity: f32,
        /// Inherited colour overlay.
        color_overlay: Color,
        /// Aspect-ratio policy inside the resolved bounds.
        fill_mode: ImageFillMode,
    },
    /// Filled and optionally stroked SVG path.
    Path {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// Intersected ancestor clip in logical surface coordinates.
        clip: Option<Geometry>,
        /// SVG path data in the node coordinate space.
        path: String,
        /// Fill colour after node opacity.
        fill_color: Color,
        /// Stroke colour after node opacity.
        stroke_color: Color,
        /// Logical stroke width.
        stroke_width: f64,
        /// True for even-odd fill, false for nonzero fill.
        even_odd: bool,
    },
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
    fn node(&self) -> NodeHandle {
        match self {
            Self::Quad { node, .. }
            | Self::Text { node, .. }
            | Self::Texture { node, .. }
            | Self::Path { node, .. } => *node,
        }
    }

    fn bounds(&self) -> Geometry {
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
            Self::Path {
                bounds,
                transform,
                stroke_width,
                ..
            } => {
                let half_stroke = stroke_width.max(0.0) / 2.0;
                transform.bounds(Geometry {
                    x: bounds.x - half_stroke,
                    y: bounds.y - half_stroke,
                    width: bounds.width + stroke_width.max(0.0),
                    height: bounds.height + stroke_width.max(0.0),
                })
            }
        };
        self.clip()
            .map_or(bounds, |clip| intersect_geometry(bounds, clip))
    }

    fn clip(&self) -> Option<Geometry> {
        match self {
            Self::Quad { clip, .. }
            | Self::Text { clip, .. }
            | Self::Texture { clip, .. }
            | Self::Path { clip, .. } => *clip,
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
        for root in scene.roots() {
            append_node(
                scene,
                layout,
                root,
                PaintContext {
                    opacity: 1.0,
                    transform: Transform2D::IDENTITY,
                    clip: None,
                    overlay: Color::rgba8(0, 0, 0, 0),
                    layer: None,
                },
                &mut list,
            )?;
        }
        Ok(list)
    }
}

