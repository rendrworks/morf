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

