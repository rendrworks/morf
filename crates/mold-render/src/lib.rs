//! Backend-independent draw lists, damage tracking, and GPU instance data.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::ops::Range;

use mold_layout::{Geometry, Layout, TextAlignment, TextElide, Transform2D};
use mold_scene::{Color, Element, NodeHandle, Scene, SceneError, Value};

mod gpu;
mod path;

pub use gpu::{GpuError, GpuInfo, WgpuBackend};

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

/// Physical damage rectangle with an exclusive lower-right edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DamageRect {
    /// Left edge in physical pixels.
    pub x: u32,
    /// Top edge in physical pixels.
    pub y: u32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

/// Draw-list differ retaining the prior successful frame.
#[derive(Default)]
pub struct DamageTracker {
    previous: DrawList,
    scale_120: u32,
}

impl DamageTracker {
    /// Diffs commands and converts changed logical bounds at protocol scale in 120ths.
    pub fn diff(&mut self, next: &DrawList, scale_120: u32) -> Vec<DamageRect> {
        if self.scale_120 != 0 && self.scale_120 != scale_120 {
            self.previous = next.clone();
            self.scale_120 = scale_120;
            return merge_damage(
                next.commands
                    .iter()
                    .filter_map(|command| physical_damage(command.bounds(), scale_120))
                    .collect(),
            );
        }
        if self.previous.layers != next.layers {
            let damage = self
                .previous
                .commands
                .iter()
                .chain(&next.commands)
                .filter_map(|command| physical_damage(command.bounds(), scale_120))
                .collect();
            self.previous = next.clone();
            self.scale_120 = scale_120;
            return merge_damage(damage);
        }
        let previous: HashMap<_, _> = self
            .previous
            .commands
            .iter()
            .enumerate()
            .map(|(order, command)| (command.node(), (order, command)))
            .collect();
        let current: HashMap<_, _> = next
            .commands
            .iter()
            .enumerate()
            .map(|(order, command)| (command.node(), (order, command)))
            .collect();
        let mut logical = Vec::new();
        for (node, (order, command)) in &current {
            match previous.get(node) {
                Some((old_order, old)) if old_order == order && *old == *command => {}
                Some((_, old)) => {
                    logical.push(old.bounds());
                    logical.push(command.bounds());
                }
                None => logical.push(command.bounds()),
            }
        }
        for (node, (_, command)) in &previous {
            if !current.contains_key(node) {
                logical.push(command.bounds());
            }
        }
        self.previous = next.clone();
        self.scale_120 = scale_120;
        merge_damage(
            logical
                .into_iter()
                .filter_map(|geometry| physical_damage(geometry, scale_120))
                .collect(),
        )
    }
}

/// Renderer implementation selected by the surface runtime.
pub trait RenderBackend {
    /// Backend error.
    type Error: StdError + Send + Sync + 'static;

    /// Draws an ordered list, restricting pixel work to damage rectangles.
    fn render(
        &mut self,
        list: &DrawList,
        damage: &[DamageRect],
        scale_120: u32,
    ) -> Result<(), Self::Error>;
}

/// Scene painter and damage tracker driving a selected backend.
pub struct RenderEngine<B> {
    backend: B,
    damage: DamageTracker,
}

impl<B: RenderBackend> RenderEngine<B> {
    /// Wraps a renderer backend with draw-list and damage processing.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            damage: DamageTracker::default(),
        }
    }

    /// Paints one resolved scene frame.
    pub fn render(
        &mut self,
        scene: &Scene,
        layout: &Layout,
        scale_120: u32,
    ) -> Result<Vec<DamageRect>, RenderError> {
        let list = DrawList::from_scene(scene, layout)?;
        let damage = self.damage.diff(&list, scale_120);
        if !damage.is_empty() {
            self.backend
                .render(&list, &damage, scale_120)
                .map_err(|error| RenderError::Backend(error.to_string()))?;
        }
        Ok(damage)
    }

    /// Returns the backend for surface-specific operations.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

/// Instance payload consumed by the SDF quad shader.
#[repr(C)]
#[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy, Debug, PartialEq)]
pub struct SdfQuadInstance {
    /// Physical x, y, width, and height.
    pub bounds: [f32; 4],
    /// RGBA fill.
    pub color: [f32; 4],
    /// Per-corner radii in clockwise order.
    pub radii: [f32; 4],
    /// Border width followed by padding.
    pub border: [f32; 4],
    /// RGBA border.
    pub border_color: [f32; 4],
    /// Original rectangle within the expanded effect bounds.
    pub shape: [f32; 4],
    /// Fill blur, shadow blur, shadow spread, and padding.
    pub effects: [f32; 4],
    /// Shadow offset followed by padding.
    pub shadow: [f32; 4],
    /// RGBA shadow colour.
    pub shadow_color: [f32; 4],
    /// RGBA gradient start colour.
    pub gradient_start_color: [f32; 4],
    /// RGBA gradient end colour.
    pub gradient_end_color: [f32; 4],
    /// Normalized gradient start and end points.
    pub gradient_points: [f32; 4],
    /// Gradient kind, center, and radius.
    pub gradient_data: [f32; 4],
    /// RGBA colour overlay.
    pub color_overlay: [f32; 4],
    /// Affine linear terms in column order.
    pub transform: [f32; 4],
    /// Affine translation in logical surface coordinates.
    pub transform_offset: [f32; 2],
}

impl SdfQuadInstance {
    /// Converts a quad command to physical GPU instance data.
    pub fn from_command(command: &DrawCommand, scale_120: u32) -> Option<Self> {
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
            ..
        } = command
        else {
            return None;
        };
        let scale = scale_120.max(1) as f64 / 120.0;
        let expanded = effect_bounds(
            *bounds,
            *blur,
            if *shadow_inner { 0.0 } else { *shadow_blur },
            if *shadow_inner { 0.0 } else { *shadow_spread },
            if *shadow_inner { 0.0 } else { *shadow_offset_x },
            if *shadow_inner { 0.0 } else { *shadow_offset_y },
        );
        let (gradient_start_color, gradient_end_color, gradient_points, gradient_data, angle) =
            gradient_instance(gradient);
        Some(Self {
            bounds: [
                (expanded.x * scale) as f32,
                (expanded.y * scale) as f32,
                (expanded.width * scale) as f32,
                (expanded.height * scale) as f32,
            ],
            color: color_array(*color),
            radii: radii.map(|radius| (radius.max(0.0) * scale) as f32),
            border: [
                if *border_pixel_aligned {
                    (*border_width * scale).round() as f32
                } else {
                    (*border_width * scale) as f32
                },
                if *antialiasing { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
            border_color: color_array(*border_color),
            shape: [
                ((bounds.x - expanded.x) * scale) as f32,
                ((bounds.y - expanded.y) * scale) as f32,
                (bounds.width * scale) as f32,
                (bounds.height * scale) as f32,
            ],
            effects: [
                (*blur * scale) as f32,
                (*shadow_blur * scale) as f32,
                (*shadow_spread * scale) as f32,
                angle,
            ],
            shadow: [
                (*shadow_offset_x * scale) as f32,
                (*shadow_offset_y * scale) as f32,
                if *shadow_inner { 1.0 } else { 0.0 },
                0.0,
            ],
            shadow_color: color_array(*shadow_color),
            gradient_start_color,
            gradient_end_color,
            gradient_points,
            gradient_data,
            color_overlay: color_array(*color_overlay),
            transform: [
                transform.matrix[0] as f32,
                transform.matrix[1] as f32,
                transform.matrix[2] as f32,
                transform.matrix[3] as f32,
            ],
            transform_offset: [
                (transform.matrix[4] * scale) as f32,
                (transform.matrix[5] * scale) as f32,
            ],
        })
    }
}

/// Scene or backend failure while producing a frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Scene property or handle failure.
    Scene(String),
    /// Selected rendering backend failure.
    Backend(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(message) => write!(f, "scene paint error: {message}"),
            Self::Backend(message) => write!(f, "render backend error: {message}"),
        }
    }
}

impl StdError for RenderError {}

impl From<SceneError> for RenderError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error.to_string())
    }
}

#[derive(Clone, Copy)]
struct PaintContext {
    opacity: f64,
    transform: Transform2D,
    clip: Option<Geometry>,
    overlay: Color,
    layer: Option<usize>,
}

fn append_node(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    inherited: PaintContext,
    list: &mut DrawList,
) -> Result<(), RenderError> {
    if !scene.bool_value(node, "visible")? {
        return Ok(());
    }
    let node_opacity = scene.number(node, "opacity")?.clamp(0.0, 1.0);
    let rotation = scene.number(node, "rotation")?;
    let layer_config = layer_config(scene, node)?;
    let element = scene.element(node)?;
    let rect_blur = if matches!(element, Element::Rect | Element::ClipRect) {
        scene.number(node, "blur")?.max(0.0)
    } else {
        0.0
    };
    let layer_blur = layer_config.blur.max(rect_blur);
    let rounded_clip = scene.bool_value(node, "clip")?
        && matches!(element, Element::Rect | Element::ClipRect)
        && rect_radii(scene, node)?.iter().any(|radius| *radius > 0.0);
    let creates_layer = layer_config.enabled
        || node_opacity < 1.0
        || rotation != 0.0
        || rounded_clip
        || layer_blur > 0.0
        || layer_config.shadow_color.alpha > 0.0;
    let layer = creates_layer.then(|| {
        let index = list.layers.len();
        list.layers.push(Layer {
            node,
            commands: list.commands.len()..list.commands.len(),
            parent: inherited.layer,
            opacity: node_opacity as f32,
            blur: layer_blur as f32,
            shadow_color: layer_config.shadow_color,
            shadow_blur: layer_config.shadow_blur as f32,
            shadow_offset: [
                layer_config.shadow_offset_x as f32,
                layer_config.shadow_offset_y as f32,
            ],
            mask: None,
            bounds: Geometry::default(),
        });
        index
    });
    let opacity = if creates_layer {
        inherited.opacity
    } else {
        inherited.opacity * node_opacity
    };
    let color_overlay =
        compose_overlay(inherited.overlay, scene.color_value(node, "color_overlay")?);
    let Some(bounds) = layout.geometry(node) else {
        return Ok(());
    };
    let transform = inherited.transform.then(Transform2D::around(
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ),
        scene.number(node, "scale")?,
        rotation,
    ));
    if let Some(layer) = layer
        && rounded_clip
    {
        list.layers[layer].mask = Some(LayerMask {
            bounds,
            transform,
            radii: rect_radii(scene, node)?,
        });
    }
    let clip = if scene.bool_value(node, "clip")? {
        let bounds = transform.bounds(bounds);
        Some(
            inherited
                .clip
                .map_or(bounds, |inherited| intersect_geometry(inherited, bounds)),
        )
    } else {
        inherited.clip
    };
    match element {
        Element::Rect | Element::ClipRect => list.commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            clip,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            color_overlay,
            gradient: scene_gradient(scene, node, opacity)?,
            radii: rect_radii(scene, node)?,
            border_width: if element == Element::ClipRect {
                0.0
            } else {
                scene.number(node, "border_width")?
            },
            antialiasing: element != Element::ClipRect || scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: element == Element::ClipRect
                && scene.bool_value(node, "border_pixel_aligned")?,
            border_color: with_opacity(scene.color_value(node, "border_color")?, opacity),
            blur: if layer_blur > 0.0 { 0.0 } else { rect_blur },
            shadow_color: with_opacity(scene.color_value(node, "shadow_color")?, opacity),
            shadow_blur: scene.number(node, "shadow_blur")?.max(0.0),
            shadow_spread: scene.number(node, "shadow_spread")?,
            shadow_offset_x: scene.number(node, "shadow_offset_x")?,
            shadow_offset_y: scene.number(node, "shadow_offset_y")?,
            shadow_inner: scene.bool_value(node, "shadow_inner")?,
        }),
        Element::Text => list.commands.push(DrawCommand::Text {
            node,
            bounds,
            transform,
            clip,
            text: scene.string_value(node, "text")?.to_owned(),
            family: scene.string_value(node, "font_family")?.to_owned(),
            size: scene.number(node, "font_size")?,
            font_weight: scene.number(node, "font_weight")?,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            color_overlay,
            wrap: scene.bool_value(node, "wrap")?,
            elide: render_text_elide(scene.string_value(node, "elide")?)?,
            horizontal_alignment: render_text_alignment(
                scene.string_value(node, "horizontal_alignment")?,
            )?,
            vertical_alignment: vertical_alignment(
                scene.string_value(node, "vertical_alignment")?,
            )?,
        }),
        Element::Image => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "source")?.to_owned(),
            icon_theme: None,
            opacity: opacity as f32,
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
        }),
        Element::Icon => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "name")?.to_owned(),
            icon_theme: Some(scene.string_value(node, "theme")?.to_owned()),
            opacity: opacity as f32,
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
        }),
        Element::Shape => list.commands.push(DrawCommand::Path {
            node,
            bounds,
            transform,
            clip,
            path: scene.string_value(node, "path")?.to_owned(),
            fill_color: apply_overlay(
                with_opacity(scene.color_value(node, "fill_color")?, opacity),
                color_overlay,
            ),
            stroke_color: apply_overlay(
                with_opacity(scene.color_value(node, "stroke_color")?, opacity),
                color_overlay,
            ),
            stroke_width: scene.number(node, "stroke_width")?.max(0.0),
            even_odd: scene.string_value(node, "fill_rule")? == "even_odd",
        }),
        Element::Item
        | Element::Inset
        | Element::MouseArea
        | Element::Row
        | Element::Column
        | Element::Grid
        | Element::RowLayout
        | Element::ColumnLayout
        | Element::GridLayout
        | Element::Flickable
        | Element::Loader
        | Element::Timer => {}
    }
    let content_layer = if element == Element::ClipRect
        && scene.number(node, "border_width")? > 0.0
        && !scene.bool_value(node, "content_under_border")?
    {
        let border = scene.number(node, "border_width")?.max(0.0);
        let inner = Geometry {
            x: bounds.x + border,
            y: bounds.y + border,
            width: (bounds.width - border * 2.0).max(0.0),
            height: (bounds.height - border * 2.0).max(0.0),
        };
        let index = list.layers.len();
        list.layers.push(Layer {
            node,
            commands: list.commands.len()..list.commands.len(),
            parent: layer.or(inherited.layer),
            opacity: 1.0,
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_offset: [0.0, 0.0],
            mask: Some(LayerMask {
                bounds: inner,
                transform,
                radii: rect_radii(scene, node)?.map(|radius| (radius - border).max(0.0)),
            }),
            bounds: inner,
        });
        Some((index, inner))
    } else {
        None
    };
    for child in scene.children(node)? {
        let child_clip = content_layer.map_or(clip, |(_, inner)| {
            let inner = transform.bounds(inner);
            Some(clip.map_or(inner, |clip| intersect_geometry(clip, inner)))
        });
        append_node(
            scene,
            layout,
            child,
            PaintContext {
                opacity,
                transform,
                clip: child_clip,
                overlay: color_overlay,
                layer: content_layer
                    .map(|(layer, _)| layer)
                    .or(layer)
                    .or(inherited.layer),
            },
            list,
        )?;
    }
    if let Some((content_layer, inner)) = content_layer {
        list.layers[content_layer].commands.end = list.commands.len();
        list.layers[content_layer].bounds = inner;
    }
    if element == Element::ClipRect && scene.number(node, "border_width")? > 0.0 {
        list.commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            clip,
            color: Color::rgba8(0, 0, 0, 0),
            color_overlay: Color::rgba8(0, 0, 0, 0),
            gradient: Gradient::None,
            radii: rect_radii(scene, node)?,
            border_width: scene.number(node, "border_width")?,
            antialiasing: scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: scene.bool_value(node, "border_pixel_aligned")?,
            border_color: apply_overlay(
                with_opacity(scene.color_value(node, "border_color")?, opacity),
                color_overlay,
            ),
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_spread: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_inner: false,
        });
    }
    if let Some(layer) = layer {
        let start = list.layers[layer].commands.start;
        let end = list.commands.len();
        list.layers[layer].commands.end = end;
        let bounds =
            command_union(&list.commands[start..end]).unwrap_or_else(|| transform.bounds(bounds));
        let blurred = expand_geometry(bounds, layer_blur * 2.0);
        list.layers[layer].bounds = if layer_config.shadow_color.alpha > 0.0 {
            union_geometry(
                blurred,
                offset_geometry(
                    expand_geometry(bounds, layer_config.shadow_blur * 2.0),
                    layer_config.shadow_offset_x,
                    layer_config.shadow_offset_y,
                ),
            )
        } else {
            blurred
        };
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct LayerConfig {
    enabled: bool,
    blur: f64,
    shadow_color: Color,
    shadow_blur: f64,
    shadow_offset_x: f64,
    shadow_offset_y: f64,
}

fn layer_config(scene: &Scene, node: NodeHandle) -> Result<LayerConfig, RenderError> {
    let Value::Map(layer) = scene.current(node, "layer")? else {
        return Err(RenderError::Scene(
            "layer must be a property map".to_owned(),
        ));
    };
    let enabled = match layer.get("enabled") {
        None => false,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => {
            return Err(RenderError::Scene(
                "layer.enabled must be a boolean".to_owned(),
            ));
        }
    };
    let number = |name: &str, non_negative: bool| match layer.get(name) {
        None => Ok(0.0),
        Some(Value::Number(value)) if value.is_finite() && (!non_negative || *value >= 0.0) => {
            Ok(*value)
        }
        Some(_) => Err(RenderError::Scene(format!(
            "layer.{name} must be a {}finite number",
            if non_negative { "non-negative " } else { "" }
        ))),
    };
    let shadow_color = match layer.get("shadow_color") {
        None => Color::rgba8(0, 0, 0, 0),
        Some(Value::Color(color)) => *color,
        Some(_) => {
            return Err(RenderError::Scene(
                "layer.shadow_color must be a color".to_owned(),
            ));
        }
    };
    Ok(LayerConfig {
        enabled,
        blur: number("blur", true)?,
        shadow_color,
        shadow_blur: number("shadow_blur", true)?,
        shadow_offset_x: number("shadow_offset_x", false)?,
        shadow_offset_y: number("shadow_offset_y", false)?,
    })
}

fn command_union(commands: &[DrawCommand]) -> Option<Geometry> {
    commands
        .iter()
        .map(DrawCommand::bounds)
        .reduce(union_geometry)
}

fn union_geometry(left: Geometry, right: Geometry) -> Geometry {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.width).max(right.x + right.width);
    let bottom = (left.y + left.height).max(right.y + right.height);
    Geometry {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    }
}

fn expand_geometry(bounds: Geometry, amount: f64) -> Geometry {
    Geometry {
        x: bounds.x - amount,
        y: bounds.y - amount,
        width: bounds.width + amount * 2.0,
        height: bounds.height + amount * 2.0,
    }
}

fn offset_geometry(bounds: Geometry, x: f64, y: f64) -> Geometry {
    Geometry {
        x: bounds.x + x,
        y: bounds.y + y,
        ..bounds
    }
}

fn compose_overlay(under: Color, over: Color) -> Color {
    let alpha = over.alpha + under.alpha * (1.0 - over.alpha);
    if alpha <= f32::EPSILON {
        return Color::rgba8(0, 0, 0, 0);
    }
    Color {
        red: (over.red * over.alpha + under.red * under.alpha * (1.0 - over.alpha)) / alpha,
        green: (over.green * over.alpha + under.green * under.alpha * (1.0 - over.alpha)) / alpha,
        blue: (over.blue * over.alpha + under.blue * under.alpha * (1.0 - over.alpha)) / alpha,
        alpha,
    }
}

fn apply_overlay(color: Color, overlay: Color) -> Color {
    Color {
        red: color.red * (1.0 - overlay.alpha) + overlay.red * overlay.alpha,
        green: color.green * (1.0 - overlay.alpha) + overlay.green * overlay.alpha,
        blue: color.blue * (1.0 - overlay.alpha) + overlay.blue * overlay.alpha,
        alpha: color.alpha,
    }
}

fn intersect_geometry(left: Geometry, right: Geometry) -> Geometry {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.width).min(right.x + right.width);
    let bottom_edge = (left.y + left.height).min(right.y + right.height);
    Geometry {
        x,
        y,
        width: (right_edge - x).max(0.0),
        height: (bottom_edge - y).max(0.0),
    }
}

fn scene_gradient(scene: &Scene, node: NodeHandle, opacity: f64) -> Result<Gradient, RenderError> {
    let start_color = with_opacity(scene.color_value(node, "gradient_start_color")?, opacity);
    let end_color = with_opacity(scene.color_value(node, "gradient_end_color")?, opacity);
    Ok(match scene.string_value(node, "gradient_type")? {
        "none" => Gradient::None,
        "linear" => Gradient::Linear {
            start_color,
            end_color,
            start: [
                scene.number(node, "gradient_start_x")?,
                scene.number(node, "gradient_start_y")?,
            ],
            end: [
                scene.number(node, "gradient_end_x")?,
                scene.number(node, "gradient_end_y")?,
            ],
        },
        "radial" => {
            let radius = scene.number(node, "gradient_radius")?;
            if radius <= 0.0 {
                return Err(RenderError::Scene(
                    "Rect radial gradient radius must be positive".to_owned(),
                ));
            }
            Gradient::Radial {
                start_color,
                end_color,
                center: [
                    scene.number(node, "gradient_center_x")?,
                    scene.number(node, "gradient_center_y")?,
                ],
                radius,
            }
        }
        "conical" => Gradient::Conical {
            start_color,
            end_color,
            center: [
                scene.number(node, "gradient_center_x")?,
                scene.number(node, "gradient_center_y")?,
            ],
            angle: scene.number(node, "gradient_angle")?,
        },
        kind => {
            return Err(RenderError::Scene(format!(
                "unknown Rect gradient type `{kind}`"
            )));
        }
    })
}

fn rect_radii(scene: &Scene, node: NodeHandle) -> Result<[f64; 4], RenderError> {
    let uniform = scene.number(node, "radius")?.max(0.0);
    let corner = |property| -> Result<f64, RenderError> {
        let value = scene.number(node, property)?;
        Ok(if value < 0.0 { uniform } else { value })
    };
    Ok([
        corner("top_left_radius")?,
        corner("top_right_radius")?,
        corner("bottom_right_radius")?,
        corner("bottom_left_radius")?,
    ])
}

fn render_text_alignment(value: &str) -> Result<TextAlignment, RenderError> {
    match value {
        "left" => Ok(TextAlignment::Left),
        "right" => Ok(TextAlignment::Right),
        "center" => Ok(TextAlignment::Center),
        "justified" => Ok(TextAlignment::Justified),
        _ => Err(RenderError::Scene(format!(
            "unknown Text horizontal alignment `{value}`"
        ))),
    }
}

fn render_text_elide(value: &str) -> Result<TextElide, RenderError> {
    match value {
        "none" => Ok(TextElide::None),
        "left" => Ok(TextElide::Left),
        "middle" => Ok(TextElide::Middle),
        "right" => Ok(TextElide::Right),
        _ => Err(RenderError::Scene(format!(
            "unknown text elide mode `{value}`"
        ))),
    }
}

fn vertical_alignment(value: &str) -> Result<VerticalAlignment, RenderError> {
    match value {
        "top" => Ok(VerticalAlignment::Top),
        "center" => Ok(VerticalAlignment::Center),
        "bottom" => Ok(VerticalAlignment::Bottom),
        _ => Err(RenderError::Scene(format!(
            "unknown Text vertical alignment `{value}`"
        ))),
    }
}

fn image_fill_mode(value: &str) -> Result<ImageFillMode, RenderError> {
    match value {
        "stretch" => Ok(ImageFillMode::Stretch),
        "preserve_aspect_fit" => Ok(ImageFillMode::PreserveAspectFit),
        "preserve_aspect_crop" => Ok(ImageFillMode::PreserveAspectCrop),
        _ => Err(RenderError::Scene(format!(
            "unknown image fill mode `{value}`"
        ))),
    }
}

fn gradient_instance(gradient: &Gradient) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], f32) {
    match gradient {
        Gradient::None => ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], 0.0),
        Gradient::Linear {
            start_color,
            end_color,
            start,
            end,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [
                start[0] as f32,
                start[1] as f32,
                end[0] as f32,
                end[1] as f32,
            ],
            [1.0, 0.0, 0.0, 0.0],
            0.0,
        ),
        Gradient::Radial {
            start_color,
            end_color,
            center,
            radius,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [0.0; 4],
            [2.0, center[0] as f32, center[1] as f32, *radius as f32],
            0.0,
        ),
        Gradient::Conical {
            start_color,
            end_color,
            center,
            angle,
        } => (
            color_array(*start_color),
            color_array(*end_color),
            [0.0; 4],
            [3.0, center[0] as f32, center[1] as f32, 0.0],
            angle.to_radians() as f32,
        ),
    }
}

fn with_opacity(mut color: Color, opacity: f64) -> Color {
    color.alpha *= opacity as f32;
    color
}

fn effect_bounds(
    bounds: Geometry,
    blur: f64,
    shadow_blur: f64,
    shadow_spread: f64,
    shadow_offset_x: f64,
    shadow_offset_y: f64,
) -> Geometry {
    let blur = blur.max(0.0);
    let shadow = (shadow_blur.max(0.0) + shadow_spread.max(0.0)).max(0.0);
    let left = blur.max(shadow - shadow_offset_x);
    let right = blur.max(shadow + shadow_offset_x);
    let top = blur.max(shadow - shadow_offset_y);
    let bottom = blur.max(shadow + shadow_offset_y);
    Geometry {
        x: bounds.x - left,
        y: bounds.y - top,
        width: bounds.width + left + right,
        height: bounds.height + top + bottom,
    }
}

fn color_array(color: Color) -> [f32; 4] {
    [color.red, color.green, color.blue, color.alpha]
}

fn physical_damage(geometry: Geometry, scale_120: u32) -> Option<DamageRect> {
    if geometry.width <= 0.0 || geometry.height <= 0.0 {
        return None;
    }
    let scale = scale_120.max(1) as f64 / 120.0;
    let left = (geometry.x * scale).floor().max(0.0) as u32;
    let top = (geometry.y * scale).floor().max(0.0) as u32;
    let right = ((geometry.x + geometry.width) * scale).ceil().max(0.0) as u32;
    let bottom = ((geometry.y + geometry.height) * scale).ceil().max(0.0) as u32;
    Some(DamageRect {
        x: left,
        y: top,
        width: right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    })
}

fn merge_damage(mut damage: Vec<DamageRect>) -> Vec<DamageRect> {
    let mut index = 0;
    while index < damage.len() {
        let mut other = index + 1;
        while other < damage.len() {
            if touches(damage[index], damage[other]) {
                damage[index] = union(damage[index], damage.remove(other));
                other = index + 1;
            } else {
                other += 1;
            }
        }
        index += 1;
    }
    damage
}

fn touches(left: DamageRect, right: DamageRect) -> bool {
    left.x <= right.x.saturating_add(right.width)
        && right.x <= left.x.saturating_add(left.width)
        && left.y <= right.y.saturating_add(right.height)
        && right.y <= left.y.saturating_add(left.height)
}

fn union(left: DamageRect, right: DamageRect) -> DamageRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .max(right.x.saturating_add(right.width));
    let bottom = left
        .y
        .saturating_add(left.height)
        .max(right.y.saturating_add(right.height));
    DamageRect {
        x,
        y,
        width: right_edge - x,
        height: bottom - y,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::convert::Infallible;

    use mold_layout::{Size, TextMeasurer};
    use mold_scene::{Element, Scene};

    use super::*;

    struct NoText;

    impl TextMeasurer for NoText {
        fn measure(
            &mut self,
            _node: NodeHandle,
            _text: &str,
            _family: &str,
            _size: f64,
            _options: mold_layout::TextOptions,
        ) -> Size {
            Size::default()
        }
    }

    #[derive(Default)]
    struct RecordingBackend {
        frames: usize,
        damage: Vec<DamageRect>,
    }

    impl RenderBackend for RecordingBackend {
        type Error = Infallible;

        fn render(
            &mut self,
            _list: &DrawList,
            damage: &[DamageRect],
            _scale_120: u32,
        ) -> Result<(), Self::Error> {
            self.frames += 1;
            self.damage = damage.to_vec();
            Ok(())
        }
    }

    #[test]
    fn draw_list_preserves_tree_paint_order() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let first = scene.create(Element::Rect);
        let second = scene.create(Element::Text);
        scene.assign(first, "width", 20.0).unwrap();
        scene.assign(first, "height", 10.0).unwrap();
        scene.reparent(first, Some(root)).unwrap();
        scene.reparent(second, Some(root)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 100.0,
                height: 20.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert_eq!(list.commands[0].node(), first);
        assert_eq!(list.commands[1].node(), second);
    }

    #[test]
    fn clip_rect_overlays_its_border_after_children() {
        let mut scene = Scene::new();
        let root = scene.create(Element::ClipRect);
        let child = scene.create(Element::Rect);
        scene.assign(root, "radius", 8.0).unwrap();
        scene.assign(root, "border_width", 2.0).unwrap();
        scene.assign(root, "border_color", "#ffffffff").unwrap();
        scene.assign(child, "width", 20.0).unwrap();
        scene.assign(child, "height", 10.0).unwrap();
        scene.reparent(child, Some(root)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 40.0,
                height: 30.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert_eq!(list.commands.len(), 3);
        assert_eq!(list.commands[0].node(), root);
        assert_eq!(list.commands[1].node(), child);
        assert_eq!(list.commands[2].node(), root);
        let DrawCommand::Quad {
            border_width: background_border,
            ..
        } = list.commands[0]
        else {
            panic!("clip background did not emit a quad");
        };
        let DrawCommand::Quad {
            color,
            border_width,
            ..
        } = list.commands[2]
        else {
            panic!("clip border did not emit a quad");
        };
        assert_eq!(background_border, 0.0);
        assert_eq!(color.alpha, 0.0);
        assert_eq!(border_width, 2.0);
        assert_eq!(list.layers[0].mask.unwrap().radii, [8.0; 4]);
        assert_eq!(list.layers[1].parent, Some(0));
        assert_eq!(list.layers[1].mask.unwrap().radii, [6.0; 4]);
        assert_eq!(list.layers[1].bounds.width, 36.0);
        assert_eq!(list.layers[1].bounds.height, 26.0);
    }

    #[test]
    fn color_overlay_propagates_through_a_subtree() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        let overlay = Color::rgba8(255, 0, 0, 128);
        scene.assign(root, "color_overlay", "#ff000080").unwrap();
        scene.assign(child, "width", 20.0).unwrap();
        scene.assign(child, "height", 10.0).unwrap();
        scene.reparent(child, Some(root)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 20.0,
                height: 10.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let DrawCommand::Quad { color_overlay, .. } = &list.commands[0] else {
            panic!("child did not emit a quad");
        };
        assert_eq!(*color_overlay, overlay);
        assert_eq!(
            SdfQuadInstance::from_command(&list.commands[0], 120)
                .unwrap()
                .color_overlay,
            color_array(overlay)
        );
    }

    #[test]
    fn nested_opacity_emits_composable_subtree_layers() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let group = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        scene.assign(root, "opacity", 0.5).unwrap();
        scene.assign(group, "opacity", 0.25).unwrap();
        scene.assign(child, "width", 20.0).unwrap();
        scene.assign(child, "height", 10.0).unwrap();
        scene.reparent(group, Some(root)).unwrap();
        scene.reparent(child, Some(group)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 20.0,
                height: 10.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert_eq!(list.layers.len(), 2);
        assert_eq!(list.layers[0].commands, 0..1);
        assert_eq!(list.layers[0].parent, None);
        assert_eq!(list.layers[0].opacity, 0.5);
        assert_eq!(list.layers[0].blur, 0.0);
        assert_eq!(list.layers[0].shadow_color.alpha, 0.0);
        assert_eq!(list.layers[1].commands, 0..1);
        assert_eq!(list.layers[1].parent, Some(0));
        assert_eq!(list.layers[1].opacity, 0.25);
        assert_eq!(list.layers[1].blur, 0.0);
        assert_eq!(list.layers[1].shadow_color.alpha, 0.0);
        let DrawCommand::Quad { color, .. } = list.commands[0] else {
            panic!("child did not emit a quad");
        };
        assert_eq!(color.alpha, 1.0);
    }

    #[test]
    fn layer_enabled_map_forces_an_offscreen_subtree() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene
            .assign(
                rect,
                "layer",
                Value::Map(BTreeMap::from([
                    ("enabled".to_owned(), Value::Bool(true)),
                    ("blur".to_owned(), Value::Number(8.0)),
                    (
                        "shadow_color".to_owned(),
                        Value::Color(Color::rgba8(8, 16, 24, 128)),
                    ),
                    ("shadow_blur".to_owned(), Value::Number(10.0)),
                    ("shadow_offset_x".to_owned(), Value::Number(12.0)),
                    ("shadow_offset_y".to_owned(), Value::Number(8.0)),
                ])),
            )
            .unwrap();
        scene.assign(rect, "width", 20.0).unwrap();
        scene.assign(rect, "height", 10.0).unwrap();
        let layout = Layout::compute(
            &scene,
            rect,
            Size {
                width: 20.0,
                height: 10.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert_eq!(list.layers.len(), 1);
        assert_eq!(list.layers[0].commands, 0..1);
        assert_eq!(list.layers[0].opacity, 1.0);
        assert_eq!(list.layers[0].blur, 8.0);
        assert_eq!(list.layers[0].shadow_color, Color::rgba8(8, 16, 24, 128));
        assert_eq!(list.layers[0].shadow_blur, 10.0);
        assert_eq!(list.layers[0].shadow_offset, [12.0, 8.0]);
        assert_eq!(list.layers[0].mask, None);
        assert_eq!(
            list.layers[0].bounds,
            Geometry {
                x: -16.0,
                y: -16.0,
                width: 68.0,
                height: 54.0,
            }
        );
        let DrawCommand::Quad { blur, .. } = list.commands[0] else {
            panic!("rectangle did not emit a quad");
        };
        assert_eq!(blur, 0.0);
    }

    #[test]
    fn rounded_clip_emits_a_transformed_layer_mask() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene.assign(rect, "x", 10.0).unwrap();
        scene.assign(rect, "y", 20.0).unwrap();
        scene.assign(rect, "width", 40.0).unwrap();
        scene.assign(rect, "height", 30.0).unwrap();
        scene.assign(rect, "radius", 7.0).unwrap();
        scene.assign(rect, "clip", true).unwrap();
        scene.assign(rect, "rotation", 15.0).unwrap();
        let layout = Layout::compute(
            &scene,
            rect,
            Size {
                width: 80.0,
                height: 80.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let mask = list.layers[0]
            .mask
            .expect("rounded clip did not emit a mask");

        assert_eq!(mask.bounds, layout.geometry(rect).unwrap());
        assert_eq!(mask.radii, [7.0; 4]);
        assert_ne!(mask.transform, Transform2D::IDENTITY);
    }

    #[test]
    fn draw_list_composes_ancestor_transforms() {
        let mut scene = Scene::new();
        let parent = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        scene.assign(parent, "width", 100.0).unwrap();
        scene.assign(parent, "height", 100.0).unwrap();
        scene.assign(parent, "rotation", 90.0).unwrap();
        scene.assign(child, "x", 10.0).unwrap();
        scene.assign(child, "width", 20.0).unwrap();
        scene.assign(child, "height", 10.0).unwrap();
        scene.assign(child, "scale", 2.0).unwrap();
        scene.reparent(child, Some(parent)).unwrap();
        let layout = Layout::compute(
            &scene,
            parent,
            Size {
                width: 100.0,
                height: 100.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let DrawCommand::Quad { transform, .. } = &list.commands[0] else {
            panic!("child did not emit a quad");
        };
        let transformed = transform.bounds(layout.geometry(child).unwrap());
        assert!((transformed.x - 85.0).abs() < 0.000_001);
        assert!(transformed.y.abs() < 0.000_001);
        assert!((transformed.width - 20.0).abs() < 0.000_001);
        assert!((transformed.height - 40.0).abs() < 0.000_001);
    }

    #[test]
    fn draw_list_intersects_nested_ancestor_clips() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let viewport = scene.create(Element::Item);
        let child = scene.create(Element::Rect);
        scene.assign(root, "width", 100.0).unwrap();
        scene.assign(root, "height", 100.0).unwrap();
        scene.assign(root, "clip", true).unwrap();
        scene.assign(viewport, "x", 25.0).unwrap();
        scene.assign(viewport, "width", 50.0).unwrap();
        scene.assign(viewport, "height", 100.0).unwrap();
        scene.assign(viewport, "clip", true).unwrap();
        scene.assign(child, "x", -25.0).unwrap();
        scene.assign(child, "width", 100.0).unwrap();
        scene.assign(child, "height", 100.0).unwrap();
        scene.reparent(viewport, Some(root)).unwrap();
        scene.reparent(child, Some(viewport)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 100.0,
                height: 100.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        assert_eq!(
            list.commands[0].clip(),
            Some(Geometry {
                x: 25.0,
                y: 0.0,
                width: 50.0,
                height: 100.0,
            })
        );
        assert_eq!(list.commands[0].bounds(), list.commands[0].clip().unwrap());
    }

    #[test]
    fn text_commands_preserve_wrap_and_alignment() {
        let mut scene = Scene::new();
        let text = scene.create(Element::Text);
        scene.assign(text, "width", 200.0).unwrap();
        scene.assign(text, "height", 80.0).unwrap();
        scene.assign(text, "wrap", true).unwrap();
        scene.assign(text, "elide", "right").unwrap();
        scene.assign(text, "font_weight", 700.0).unwrap();
        scene
            .assign(text, "horizontal_alignment", "center")
            .unwrap();
        scene.assign(text, "vertical_alignment", "bottom").unwrap();
        let layout = Layout::compute(
            &scene,
            text,
            Size {
                width: 200.0,
                height: 80.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let DrawCommand::Text {
            wrap,
            elide,
            font_weight,
            horizontal_alignment,
            vertical_alignment,
            ..
        } = list.commands[0]
        else {
            panic!("text did not emit a text command");
        };
        assert!(wrap);
        assert_eq!(elide, TextElide::Right);
        assert_eq!(font_weight, 700.0);
        assert_eq!(horizontal_alignment, TextAlignment::Center);
        assert_eq!(vertical_alignment, VerticalAlignment::Bottom);
    }

    #[test]
    fn rectangles_emit_normalized_gradient_instances() {
        let mut scene = Scene::new();
        let rect = scene.create(Element::Rect);
        scene.assign(rect, "width", 100.0).unwrap();
        scene.assign(rect, "height", 50.0).unwrap();
        scene.assign(rect, "opacity", 0.5).unwrap();
        scene.assign(rect, "gradient_type", "linear").unwrap();
        scene
            .assign(rect, "gradient_start_color", "#ff0000")
            .unwrap();
        scene.assign(rect, "gradient_end_color", "#0000ff").unwrap();
        scene.assign(rect, "gradient_end_y", 1.0).unwrap();
        scene.assign(rect, "radius", 4.0).unwrap();
        scene.assign(rect, "top_right_radius", 12.0).unwrap();
        let layout = Layout::compute(
            &scene,
            rect,
            Size {
                width: 100.0,
                height: 50.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let DrawCommand::Quad {
            gradient, radii, ..
        } = &list.commands[0]
        else {
            panic!("rectangle did not emit a quad");
        };
        assert_eq!(*radii, [4.0, 12.0, 4.0, 4.0]);
        assert_eq!(
            gradient,
            &Gradient::Linear {
                start_color: Color {
                    red: 1.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                },
                end_color: Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 1.0,
                    alpha: 1.0,
                },
                start: [0.0, 0.0],
                end: [1.0, 1.0],
            }
        );
        assert_eq!(list.layers.len(), 1);
        assert_eq!(list.layers[0].opacity, 0.5);
        assert_eq!(list.layers[0].blur, 0.0);
        assert_eq!(list.layers[0].shadow_color.alpha, 0.0);
        let instance = SdfQuadInstance::from_command(&list.commands[0], 120).unwrap();
        assert_eq!(instance.gradient_data[0], 1.0);
        assert_eq!(instance.gradient_points, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(instance.radii, [4.0, 12.0, 4.0, 4.0]);
    }

    #[test]
    fn images_and_icons_emit_texture_commands() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Item);
        let image = scene.create(Element::Image);
        let icon = scene.create(Element::Icon);
        scene
            .assign(image, "source", "/tmp/wallpaper.webp")
            .unwrap();
        scene.assign(image, "width", 40.0).unwrap();
        scene.assign(image, "height", 20.0).unwrap();
        scene
            .assign(image, "fill_mode", "preserve_aspect_fit")
            .unwrap();
        scene.assign(icon, "name", "network-wireless").unwrap();
        scene.assign(icon, "theme", "Adwaita").unwrap();
        scene.assign(icon, "width", 16.0).unwrap();
        scene.assign(icon, "height", 16.0).unwrap();
        scene.reparent(image, Some(root)).unwrap();
        scene.reparent(icon, Some(root)).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 100.0,
                height: 100.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert!(matches!(
            &list.commands[0],
            DrawCommand::Texture {
                source,
                icon_theme: None,
                fill_mode: ImageFillMode::PreserveAspectFit,
                ..
            } if source == "/tmp/wallpaper.webp"
        ));
        assert!(matches!(
            &list.commands[1],
            DrawCommand::Texture { source, icon_theme: Some(theme), .. }
                if source == "network-wireless" && theme == "Adwaita"
        ));
    }

    #[test]
    fn shapes_emit_path_commands() {
        let mut scene = Scene::new();
        let shape = scene.create(Element::Shape);
        scene.assign(shape, "path", "M0 0 L16 0 L8 16 Z").unwrap();
        scene.assign(shape, "width", 16.0).unwrap();
        scene.assign(shape, "height", 16.0).unwrap();
        scene.assign(shape, "stroke_width", 2.0).unwrap();
        let layout = Layout::compute(
            &scene,
            shape,
            Size {
                width: 16.0,
                height: 16.0,
            },
            &mut NoText,
        )
        .unwrap();

        let list = DrawList::from_scene(&scene, &layout).unwrap();

        assert!(matches!(
            &list.commands[0],
            DrawCommand::Path { path, stroke_width, .. }
                if path == "M0 0 L16 0 L8 16 Z" && *stroke_width == 2.0
        ));
    }

    #[test]
    fn unchanged_frames_submit_no_gpu_work() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Rect);
        scene.assign(root, "width", 20.0).unwrap();
        scene.assign(root, "height", 10.0).unwrap();
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 20.0,
                height: 10.0,
            },
            &mut NoText,
        )
        .unwrap();
        let mut engine = RenderEngine::new(RecordingBackend::default());

        assert!(!engine.render(&scene, &layout, 120).unwrap().is_empty());
        assert!(engine.render(&scene, &layout, 120).unwrap().is_empty());
        assert_eq!(engine.backend_mut().frames, 1);
    }

    #[test]
    fn fractional_scale_rounds_damage_outward() {
        let geometry = Geometry {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };

        assert_eq!(
            physical_damage(geometry, 180),
            Some(DamageRect {
                x: 1,
                y: 3,
                width: 5,
                height: 6,
            })
        );
    }

    #[test]
    fn scale_change_redamages_an_unchanged_frame() {
        let mut scene = Scene::new();
        let root = scene.create(Element::Rect);
        let layout = Layout::compute(
            &scene,
            root,
            Size {
                width: 20.0,
                height: 10.0,
            },
            &mut NoText,
        )
        .unwrap();
        let list = DrawList::from_scene(&scene, &layout).unwrap();
        let mut tracker = DamageTracker::default();
        tracker.diff(&list, 120);

        assert_eq!(
            tracker.diff(&list, 150),
            vec![DamageRect {
                x: 0,
                y: 0,
                width: 25,
                height: 13,
            }]
        );
    }

    #[test]
    fn changed_command_damages_old_and_new_bounds() {
        let node = {
            let mut scene = Scene::new();
            scene.create(Element::Rect)
        };
        let mut tracker = DamageTracker::default();
        let first = DrawList {
            commands: vec![DrawCommand::Quad {
                node,
                bounds: Geometry {
                    width: 10.0,
                    height: 10.0,
                    ..Geometry::default()
                },
                transform: Transform2D::IDENTITY,
                clip: None,
                color: Color::rgba8(0, 0, 0, 255),
                color_overlay: Color::rgba8(0, 0, 0, 0),
                gradient: Gradient::None,
                radii: [0.0; 4],
                border_width: 0.0,
                antialiasing: true,
                border_pixel_aligned: false,
                border_color: Color::rgba8(0, 0, 0, 0),
                blur: 0.0,
                shadow_color: Color::rgba8(0, 0, 0, 0),
                shadow_blur: 0.0,
                shadow_spread: 0.0,
                shadow_offset_x: 0.0,
                shadow_offset_y: 0.0,
                shadow_inner: false,
            }],
            layers: Vec::new(),
        };
        tracker.diff(&first, 120);
        let mut second = first.clone();
        if let DrawCommand::Quad { bounds, .. } = &mut second.commands[0] {
            bounds.x = 20.0;
        }

        let damage = tracker.diff(&second, 120);

        assert_eq!(damage.len(), 2);
    }

    #[test]
    fn blur_and_shadow_expand_damage_and_gpu_bounds() {
        let node = {
            let mut scene = Scene::new();
            scene.create(Element::Rect)
        };
        let command = DrawCommand::Quad {
            node,
            bounds: Geometry {
                x: 20.0,
                y: 20.0,
                width: 40.0,
                height: 20.0,
            },
            transform: Transform2D::IDENTITY,
            clip: None,
            color: Color::rgba8(255, 255, 255, 255),
            color_overlay: Color::rgba8(0, 0, 0, 0),
            gradient: Gradient::None,
            radii: [4.0; 4],
            border_width: 0.6,
            antialiasing: false,
            border_pixel_aligned: true,
            border_color: Color::rgba8(0, 0, 0, 0),
            blur: 2.0,
            shadow_color: Color::rgba8(0, 0, 0, 128),
            shadow_blur: 6.0,
            shadow_spread: 2.0,
            shadow_offset_x: 3.0,
            shadow_offset_y: 4.0,
            shadow_inner: false,
        };

        assert_eq!(
            command.bounds(),
            Geometry {
                x: 15.0,
                y: 16.0,
                width: 56.0,
                height: 36.0,
            }
        );
        let instance = SdfQuadInstance::from_command(&command, 120).unwrap();
        assert_eq!(instance.bounds, [15.0, 16.0, 56.0, 36.0]);
        assert_eq!(instance.shape, [5.0, 4.0, 40.0, 20.0]);
        assert_eq!(instance.effects[..3], [2.0, 6.0, 2.0]);
        assert_eq!(instance.border[..2], [1.0, 0.0]);

        let mut inner = command;
        if let DrawCommand::Quad {
            blur, shadow_inner, ..
        } = &mut inner
        {
            *blur = 0.0;
            *shadow_inner = true;
        }
        assert_eq!(
            inner.bounds(),
            Geometry {
                x: 20.0,
                y: 20.0,
                width: 40.0,
                height: 20.0,
            }
        );
        let instance = SdfQuadInstance::from_command(&inner, 120).unwrap();
        assert_eq!(instance.shadow[2], 1.0);
    }
}
