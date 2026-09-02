use morf_layout::Layout;
use morf_scene::{NodeHandle, Scene};
use std::collections::HashMap;
use std::error::Error as StdError;

use crate::{commands::*, effects::*, sdf::*};

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
    /// Takes the frame just diffed, handing back the buffer it replaces.
    ///
    /// The alternative is what this replaces — `previous = next.clone()`, a
    /// deep copy of every command including its owned strings and layer
    /// vectors, once a frame. That is precisely the cost the surrounding code
    /// is built to avoid: `DrawList::rebuild` keeps its buffers between frames
    /// so their capacity is not returned to the allocator sixty times a second,
    /// and then the tracker copied the whole thing anyway. Swapping keeps two
    /// warm buffers in rotation and copies nothing.
    pub fn retain(&mut self, list: &mut DrawList) {
        std::mem::swap(&mut self.previous, list);
    }

    /// Forgets what is on screen, so the next frame is damaged in full.
    ///
    /// For when the surface stops being what the last frame was painted into —
    /// a resize hands back a blank target, and every pixel the tracker believes
    /// is still there is gone with the old one.
    pub fn forget(&mut self) {
        self.previous = DrawList::default();
        self.scale_120 = 0;
    }

    /// Diffs commands and converts changed logical bounds at protocol scale in 120ths.
    pub fn diff(&mut self, next: &DrawList, scale_120: u32) -> Vec<DamageRect> {
        if self.scale_120 != 0 && self.scale_120 != scale_120 {
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
            self.scale_120 = scale_120;
            return merge_damage(damage);
        }
        let previous = keyed_commands(&self.previous.commands);
        let current = keyed_commands(&next.commands);
        let mut logical = Vec::new();
        for (key, (order, command)) in &current {
            match previous.get(key) {
                Some((old_order, old)) if old_order == order && *old == *command => {}
                Some((_, old)) => {
                    logical.push(old.bounds());
                    logical.push(command.bounds());
                }
                None => logical.push(command.bounds()),
            }
        }
        for (key, (_, command)) in &previous {
            if !current.contains_key(key) {
                logical.push(command.bounds());
            }
        }
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

    /// Resizes the target the frames are painted into.
    ///
    /// Whatever the last frame left there does not survive this, which is why
    /// it is on the trait and not only on the backend: it has to be reachable
    /// through the engine, and the engine is the only thing that knows the
    /// screen has to be repainted in full afterwards.
    fn resize(&mut self, width: u32, height: u32);
}

/// Scene painter and damage tracker driving a selected backend.
pub struct RenderEngine<B> {
    backend: B,
    damage: DamageTracker,
    /// The draw list, kept between frames for its capacity.
    list: DrawList,
}

impl<B: RenderBackend> RenderEngine<B> {
    /// Wraps a renderer backend with draw-list and damage processing.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            damage: DamageTracker::default(),
            list: DrawList::default(),
        }
    }

    /// Paints one resolved scene frame.
    pub fn render(
        &mut self,
        scene: &Scene,
        layout: &Layout,
        scale_120: u32,
        declare: impl FnOnce(&[DamageRect]),
    ) -> Result<Vec<DamageRect>, RenderError> {
        // The list is kept between frames so its buffers are not returned to
        // the allocator and reclaimed sixty times a second.
        let mut list = std::mem::take(&mut self.list);
        let result = list.rebuild(scene, layout).and_then(|()| {
            let damage = self.damage.diff(&list, scale_120);
            // The caller sees the damage before anything is presented, because
            // presenting commits the surface and a commit carries whatever
            // damage was declared before it. A host that waits until afterwards
            // has no way to tell the compositor what actually changed, and ends
            // up declaring the whole surface — which on a fullscreen overlay
            // means a full recomposite every frame.
            declare(&damage);
            if !damage.is_empty() {
                self.backend
                    .render(&list, &damage, scale_120)
                    .map_err(|error| RenderError::Backend(error.to_string()))?;
            }
            Ok(damage)
        });
        // Only on success: a failed rebuild leaves a half-built list, and
        // making that the baseline would silently under-damage the next frame.
        if result.is_ok() {
            self.damage.retain(&mut list);
        }
        self.list = list;
        result
    }

    /// Resizes the target, and forgets what was on it.
    ///
    /// The two belong together. A resize hands back a blank target, so a frame
    /// diffed against the one before it repaints only what changed and leaves
    /// the rest of the screen as the cleared colour — black, with whatever
    /// happens to animate afterwards appearing on it one piece at a time. That
    /// is what this is: the resize goes through the engine so the baseline
    /// cannot be left behind.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.backend.resize(width, height);
        self.damage.forget();
    }

    /// Returns the backend for surface-specific operations.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

/// Indexes a frame's commands so that two commands can be compared against the
/// two they correspond to in the previous frame.
///
/// The node alone is not enough to say which command is which. A `ClipRect`
/// emits two — the fill and the border it overlays — and keying on the node
/// collapses them, because collecting a `HashMap` keeps the last entry for a
/// duplicate key. The fill would then never be compared against anything and
/// changing it would repaint nothing. The occurrence index is what separates
/// them, and it is stable because paint always emits a node's commands in the
/// same order.
fn keyed_commands(commands: &[DrawCommand]) -> HashMap<(NodeHandle, u32), (usize, &DrawCommand)> {
    let mut emitted: HashMap<NodeHandle, u32> = HashMap::new();
    commands
        .iter()
        .enumerate()
        .map(|(order, command)| {
            let node = command.node();
            let occurrence = emitted.entry(node).or_default();
            let key = (node, *occurrence);
            *occurrence += 1;
            (key, (order, command))
        })
        .collect()
}
