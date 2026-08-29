//! Backend-independent draw lists, damage tracking, and GPU instance data.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;

use mold_layout::{Geometry, Layout, TextAlignment, TextElide, Transform2D};
use mold_scene::{Color, Element, NodeHandle, Scene, SceneError};

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
        /// Fill colour after node opacity.
        color: Color,
        /// Optional normalized gradient fill.
        gradient: Gradient,
        /// Corner radii in top-left clockwise order.
        radii: [f64; 4],
        /// Border width.
        border_width: f64,
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
    },
    /// Shaped glyph run owned by the text subsystem.
    Text {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// Composed node and ancestor transform.
        transform: Transform2D,
        /// UTF-8 text used to locate its shaped buffer.
        text: String,
        /// Font family used by the shaping cache.
        family: String,
        /// Logical font size.
        size: f64,
        /// Glyph colour after node opacity.
        color: Color,
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
        /// Image path or icon name.
        source: String,
        /// Theme name for an icon command.
        icon_theme: Option<String>,
        /// Effective node opacity.
        opacity: f32,
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
        match self {
            Self::Quad {
                bounds,
                transform,
                blur,
                shadow_blur,
                shadow_spread,
                shadow_offset_x,
                shadow_offset_y,
                ..
            } => transform.bounds(effect_bounds(
                *bounds,
                *blur,
                *shadow_blur,
                *shadow_spread,
                *shadow_offset_x,
                *shadow_offset_y,
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
        }
    }
}

/// Ordered commands for one surface frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    /// Back-to-front paint operations.
    pub commands: Vec<DrawCommand>,
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
                1.0,
                Transform2D::IDENTITY,
                &mut list.commands,
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
            gradient,
            radii,
            border_width,
            border_color,
            blur,
            shadow_color,
            shadow_blur,
            shadow_spread,
            shadow_offset_x,
            shadow_offset_y,
            ..
        } = command
        else {
            return None;
        };
        let scale = scale_120.max(1) as f64 / 120.0;
        let expanded = effect_bounds(
            *bounds,
            *blur,
            *shadow_blur,
            *shadow_spread,
            *shadow_offset_x,
            *shadow_offset_y,
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
            border: [(*border_width * scale) as f32, 0.0, 0.0, 0.0],
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
                0.0,
                0.0,
            ],
            shadow_color: color_array(*shadow_color),
            gradient_start_color,
            gradient_end_color,
            gradient_points,
            gradient_data,
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

fn append_node(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    inherited_opacity: f64,
    inherited_transform: Transform2D,
    commands: &mut Vec<DrawCommand>,
) -> Result<(), RenderError> {
    if !scene.bool_value(node, "visible")? {
        return Ok(());
    }
    let opacity = inherited_opacity * scene.number(node, "opacity")?.clamp(0.0, 1.0);
    let Some(bounds) = layout.geometry(node) else {
        return Ok(());
    };
    let transform = inherited_transform.then(Transform2D::around(
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ),
        scene.number(node, "scale")?,
        scene.number(node, "rotation")?,
    ));
    match scene.element(node)? {
        Element::Rect => commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            gradient: scene_gradient(scene, node, opacity)?,
            radii: rect_radii(scene, node)?,
            border_width: scene.number(node, "border_width")?,
            border_color: with_opacity(scene.color_value(node, "border_color")?, opacity),
            blur: scene.number(node, "blur")?.max(0.0),
            shadow_color: with_opacity(scene.color_value(node, "shadow_color")?, opacity),
            shadow_blur: scene.number(node, "shadow_blur")?.max(0.0),
            shadow_spread: scene.number(node, "shadow_spread")?,
            shadow_offset_x: scene.number(node, "shadow_offset_x")?,
            shadow_offset_y: scene.number(node, "shadow_offset_y")?,
        }),
        Element::Text => commands.push(DrawCommand::Text {
            node,
            bounds,
            transform,
            text: scene.string_value(node, "text")?.to_owned(),
            family: scene.string_value(node, "font_family")?.to_owned(),
            size: scene.number(node, "font_size")?,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            wrap: scene.bool_value(node, "wrap")?,
            elide: render_text_elide(scene.string_value(node, "elide")?)?,
            horizontal_alignment: render_text_alignment(
                scene.string_value(node, "horizontal_alignment")?,
            )?,
            vertical_alignment: vertical_alignment(
                scene.string_value(node, "vertical_alignment")?,
            )?,
        }),
        Element::Image => commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            source: scene.string_value(node, "source")?.to_owned(),
            icon_theme: None,
            opacity: opacity as f32,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
        }),
        Element::Icon => commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            source: scene.string_value(node, "name")?.to_owned(),
            icon_theme: Some(scene.string_value(node, "theme")?.to_owned()),
            opacity: opacity as f32,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
        }),
        Element::Shape => commands.push(DrawCommand::Path {
            node,
            bounds,
            transform,
            path: scene.string_value(node, "path")?.to_owned(),
            fill_color: with_opacity(scene.color_value(node, "fill_color")?, opacity),
            stroke_color: with_opacity(scene.color_value(node, "stroke_color")?, opacity),
            stroke_width: scene.number(node, "stroke_width")?.max(0.0),
            even_odd: scene.string_value(node, "fill_rule")? == "even_odd",
        }),
        Element::Item
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
    for child in scene.children(node)? {
        append_node(scene, layout, child, opacity, transform, commands)?;
    }
    Ok(())
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
    fn text_commands_preserve_wrap_and_alignment() {
        let mut scene = Scene::new();
        let text = scene.create(Element::Text);
        scene.assign(text, "width", 200.0).unwrap();
        scene.assign(text, "height", 80.0).unwrap();
        scene.assign(text, "wrap", true).unwrap();
        scene.assign(text, "elide", "right").unwrap();
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
            horizontal_alignment,
            vertical_alignment,
            ..
        } = list.commands[0]
        else {
            panic!("text did not emit a text command");
        };
        assert!(wrap);
        assert_eq!(elide, TextElide::Right);
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
                    alpha: 0.5,
                },
                end_color: Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 1.0,
                    alpha: 0.5,
                },
                start: [0.0, 0.0],
                end: [1.0, 1.0],
            }
        );
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
                color: Color::rgba8(0, 0, 0, 255),
                gradient: Gradient::None,
                radii: [0.0; 4],
                border_width: 0.0,
                border_color: Color::rgba8(0, 0, 0, 0),
                blur: 0.0,
                shadow_color: Color::rgba8(0, 0, 0, 0),
                shadow_blur: 0.0,
                shadow_spread: 0.0,
                shadow_offset_x: 0.0,
                shadow_offset_y: 0.0,
            }],
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
            color: Color::rgba8(255, 255, 255, 255),
            gradient: Gradient::None,
            radii: [4.0; 4],
            border_width: 0.0,
            border_color: Color::rgba8(0, 0, 0, 0),
            blur: 2.0,
            shadow_color: Color::rgba8(0, 0, 0, 128),
            shadow_blur: 6.0,
            shadow_spread: 2.0,
            shadow_offset_x: 3.0,
            shadow_offset_y: 4.0,
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
    }
}
