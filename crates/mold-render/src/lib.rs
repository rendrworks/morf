//! Backend-independent draw lists, damage tracking, and GPU instance data.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;

use mold_layout::{Geometry, Layout};
use mold_scene::{Color, Element, NodeHandle, Scene, SceneError};

mod gpu;

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
        /// Fill colour after node opacity.
        color: Color,
        /// Uniform corner radius.
        radius: f64,
        /// Border width.
        border_width: f64,
        /// Border colour after node opacity.
        border_color: Color,
    },
    /// Shaped glyph run owned by the text subsystem.
    Text {
        /// Source scene node.
        node: NodeHandle,
        /// Logical surface bounds.
        bounds: Geometry,
        /// UTF-8 text used to locate its shaped buffer.
        text: String,
        /// Font family used by the shaping cache.
        family: String,
        /// Logical font size.
        size: f64,
        /// Glyph colour after node opacity.
        color: Color,
    },
}

impl DrawCommand {
    fn node(&self) -> NodeHandle {
        match self {
            Self::Quad { node, .. } | Self::Text { node, .. } => *node,
        }
    }

    fn bounds(&self) -> Geometry {
        match self {
            Self::Quad { bounds, .. } | Self::Text { bounds, .. } => *bounds,
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
            append_node(scene, layout, root, 1.0, &mut list.commands)?;
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
}

impl SdfQuadInstance {
    /// Converts a quad command to physical GPU instance data.
    pub fn from_command(command: &DrawCommand, scale_120: u32) -> Option<Self> {
        let DrawCommand::Quad {
            bounds,
            color,
            radius,
            border_width,
            border_color,
            ..
        } = command
        else {
            return None;
        };
        let scale = scale_120.max(1) as f64 / 120.0;
        Some(Self {
            bounds: [
                (bounds.x * scale) as f32,
                (bounds.y * scale) as f32,
                (bounds.width * scale) as f32,
                (bounds.height * scale) as f32,
            ],
            color: color_array(*color),
            radii: [(*radius * scale) as f32; 4],
            border: [(*border_width * scale) as f32, 0.0, 0.0, 0.0],
            border_color: color_array(*border_color),
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
    commands: &mut Vec<DrawCommand>,
) -> Result<(), RenderError> {
    if !scene.bool_value(node, "visible")? {
        return Ok(());
    }
    let opacity = inherited_opacity * scene.number(node, "opacity")?.clamp(0.0, 1.0);
    let Some(bounds) = layout.geometry(node) else {
        return Ok(());
    };
    match scene.element(node)? {
        Element::Rect => commands.push(DrawCommand::Quad {
            node,
            bounds,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            radius: scene.number(node, "radius")?,
            border_width: scene.number(node, "border_width")?,
            border_color: with_opacity(scene.color_value(node, "border_color")?, opacity),
        }),
        Element::Text => commands.push(DrawCommand::Text {
            node,
            bounds,
            text: scene.string_value(node, "text")?.to_owned(),
            family: scene.string_value(node, "font_family")?.to_owned(),
            size: scene.number(node, "font_size")?,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
        }),
        Element::Item | Element::MouseArea | Element::Row | Element::Column => {}
    }
    for child in scene.children(node)? {
        append_node(scene, layout, child, opacity, commands)?;
    }
    Ok(())
}

fn with_opacity(mut color: Color, opacity: f64) -> Color {
    color.alpha *= opacity as f32;
    color
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
            _wrap_width: Option<f64>,
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
                color: Color::rgba8(0, 0, 0, 255),
                radius: 0.0,
                border_width: 0.0,
                border_color: Color::rgba8(0, 0, 0, 0),
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
}
