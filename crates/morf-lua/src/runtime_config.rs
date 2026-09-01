use luna::{Closure, Executor, ExecutorMode, Fuel, Lua};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::{
    api_finish::*, reactive_execute::*, serialization::*, state::*, surface_types::*, types::*,
};

impl Runtime {
    /// Creates a sandboxed runtime with the supplied limits.
    pub fn new(limits: Limits) -> Self {
        Self::with_screen(limits, None)
    }

    /// Creates a runtime whose `morf.screens` model contains one output.
    pub fn for_screen(limits: Limits, screen: Screen) -> Self {
        Self::with_screen(limits, Some(screen))
    }

    pub(crate) fn with_screen(limits: Limits, screen: Option<Screen>) -> Self {
        let mut lua = Lua::core();
        lua.set_memory_limit(Some(limits.memory));
        let reactive = Rc::new(RefCell::new(ReactiveState::new()));
        let module_roots = Rc::new(RefCell::new(default_module_roots()));
        install_reactive_api(
            &mut lua,
            Rc::clone(&reactive),
            Rc::clone(&module_roots),
            limits,
            screen.as_ref(),
        );
        Self {
            lua,
            limits,
            reactive,
            module_roots,
        }
    }

    /// Compiles and executes a Lua chunk.
    pub fn execute(&mut self, name: &str, source: &[u8]) -> Result<(), Error> {
        if let Some(parent) = Path::new(name).parent()
            && !parent.as_os_str().is_empty()
            && !self.module_roots.borrow().contains(&parent.to_path_buf())
        {
            self.module_roots
                .borrow_mut()
                .insert(0, parent.to_path_buf());
        }
        let executor = self
            .lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, Some(name), source)?;
                Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
            })
            .map_err(|error| Error::Load(format!("{name}: {error}")))?;

        let slice_fuel = self.limits.slice_fuel.max(1);
        let mut remaining = self.limits.fuel;

        loop {
            if remaining == 0 {
                self.lua.enter(|ctx| ctx.fetch(&executor).stop(&ctx));
                return Err(Error::FuelExhausted {
                    budget: self.limits.fuel,
                });
            }

            let allowance = remaining.min(slice_fuel as u64) as i32;
            let mut fuel = Fuel::with(allowance);
            let finished = self
                .lua
                .enter(|ctx| ctx.fetch(&executor).step(ctx, &mut fuel))
                .map_err(|error| Error::Runtime(error.to_string()))?;
            let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
            remaining = remaining.saturating_sub(consumed.max(1));

            if finished {
                break;
            }
        }

        let mode = self.lua.enter(|ctx| ctx.fetch(&executor).mode());
        if mode != ExecutorMode::Result {
            return Err(Error::Runtime(format!(
                "execution stopped in {mode:?} mode"
            )));
        }

        self.lua
            .execute::<()>(&executor)
            .map_err(|error| Error::Runtime(error.to_string()))
    }

    /// Replaces the ordered filesystem roots used for user Lua modules.
    pub fn set_module_roots(&mut self, roots: Vec<PathBuf>) {
        *self.module_roots.borrow_mut() = roots;
    }

    /// Sets the root directory used by native shell path helpers.
    pub fn set_shell_root(&mut self, root: PathBuf) {
        self.reactive.borrow_mut().shell_root = root;
    }

    /// Returns the native layer-surface settings assigned by the configuration.
    pub fn layer_surface_config(&self) -> LayerSurfaceConfig {
        self.reactive.borrow().layer_surface.clone()
    }

    pub fn window_surface_configs(&self) -> Vec<WindowSurfaceConfig> {
        let mut surfaces = self
            .reactive
            .borrow()
            .window_surfaces
            .values()
            .cloned()
            .collect::<Vec<_>>();
        surfaces.sort_by_key(|surface| surface.id);
        surfaces
    }

    pub fn take_window_surface_change(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().window_surfaces_changed)
    }

    /// Takes a pending change to the shell's own layer-surface geometry.
    ///
    /// Size, anchors, margins, exclusive zone and keyboard interactivity may all
    /// be re-issued on a mapped surface, so an assignment to `morf.surface`
    /// reaches the compositor without a reconnect.
    pub fn take_layer_surface_change(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().layer_surface_changed)
    }

    pub fn take_window_surface_actions(&mut self) -> Vec<WindowSurfaceAction> {
        std::mem::take(&mut self.reactive.borrow_mut().window_surface_actions)
    }

    pub fn set_window_surface_visible(&mut self, id: u64, visible: bool) -> bool {
        let mut state = self.reactive.borrow_mut();
        let Some(surface) = state.window_surfaces.get_mut(&id) else {
            return false;
        };
        if surface.visible == visible {
            return false;
        }
        surface.visible = visible;
        state.window_surfaces_changed = true;
        true
    }

    /// Drains non-fatal binding diagnostics produced since the previous call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reactive.borrow_mut().logs)
    }

    /// Returns bindings that currently read frame-varying scene properties.
    pub fn binding_dependencies(&self) -> Vec<String> {
        let state = self.reactive.borrow();
        let Some(graph) = state.graph.as_ref() else {
            return Vec::new();
        };
        graph
            .dependencies()
            .into_iter()
            .filter_map(|entry| {
                let mut animated = entry
                    .signals
                    .into_iter()
                    .filter(|signal| {
                        state
                            .current_property_names
                            .get(signal)
                            .is_some_and(|(node, property)| {
                                state.scene.is_animating(*node, property).unwrap_or(false)
                            })
                    })
                    .collect::<Vec<_>>();
                animated.sort();
                (!animated.is_empty()).then(|| {
                    format!(
                        "depth {}: {} <- {} (current animation values do not trigger Lua; use the target accessor)",
                        entry.depth,
                        entry.effect,
                        animated.join(", ")
                    )
                })
            })
            .collect()
    }

    /// Captures values explicitly marked for transfer to a replacement runtime.
    pub fn reloadable_state(&self) -> BTreeMap<String, IpcValue> {
        let state = self.reactive.borrow();
        state
            .reloadable
            .iter()
            .filter_map(|(name, signal)| {
                state
                    .values
                    .get(signal)
                    .map(|value| (name.clone(), value.clone()))
            })
            .collect()
    }

    /// Seeds reloadable values before executing replacement configuration code.
    pub fn restore_reloadable_state(&mut self, values: BTreeMap<String, IpcValue>) {
        // One value type across the boundary now, so this only changes the map
        // kind — the values pass straight through.
        self.reactive.borrow_mut().reload_seed = values.into_iter().collect();
    }

    /// Takes a reload request raised by Lua configuration.
    pub fn take_reload_request(&mut self) -> Option<bool> {
        self.reactive.borrow_mut().reload_request.take()
    }

    pub fn take_watch_files_change(&mut self) -> Option<bool> {
        let mut state = self.reactive.borrow_mut();
        state.watch_files_changed.then(|| {
            state.watch_files_changed = false;
            state.watch_files
        })
    }

    pub fn dispatch_reload_completed(&mut self) -> bool {
        self.dispatch_reload_callbacks(true, None)
    }

    pub fn dispatch_reload_failed(&mut self, error: String) -> bool {
        self.dispatch_reload_callbacks(false, Some(error))
    }

    pub(crate) fn dispatch_reload_callbacks(
        &mut self,
        completed: bool,
        error: Option<String>,
    ) -> bool {
        let callbacks = {
            let state = self.reactive.borrow();
            if completed {
                state.reload_completed_callbacks.clone()
            } else {
                state.reload_failed_callbacks.clone()
            }
        };
        let args = error.map(IpcValue::String).into_iter().collect::<Vec<_>>();
        for callback in &callbacks {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("reload callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }
}

/// One shader a configuration registered, as the host needs it.
pub struct ShaderProgram {
    /// Hash of the generated WGSL: what a node carries and the backend keys on.
    pub program: u64,
    pub wgsl: String,
    /// Byte offset of each parameter in the uniform block.
    pub offsets: Vec<u32>,
    pub uniform_size: u32,
    /// Whether the shader reads the frame clock, and so repaints every frame.
    pub reads_time: bool,
    /// Whether the shader decides its own coverage rather than colouring what
    /// the field shaped.
    pub owns_coverage: bool,
    /// Whether the shader reads what is rendered underneath, and so belongs to
    /// the composite pass rather than the field pass.
    pub samples_behind: bool,
    /// The vertex displacement's WGSL, if it has one.
    pub vertex: Option<String>,
    /// Image paths for its declared textures, in binding order.
    pub textures: Vec<String>,
    /// Its data blocks: name and element count, in binding order.
    pub data: Vec<(String, u32)>,
}

impl Runtime {
    /// Every shader the configuration registered.
    ///
    /// The host hands these to the renderer once, at startup: compiling a
    /// pipeline costs tens of milliseconds and cannot happen during a frame.
    pub fn shaders(&self) -> Vec<ShaderProgram> {
        self.reactive
            .borrow()
            .shaders
            .values()
            .map(|shader| ShaderProgram {
                program: shader.compiled.hash,
                wgsl: shader.compiled.wgsl.clone(),
                offsets: shader
                    .compiled
                    .params
                    .iter()
                    .map(|slot| slot.offset)
                    .collect(),
                uniform_size: shader.compiled.uniform_size,
                reads_time: shader.compiled.reads_time,
                owns_coverage: shader.kind == morf_shader::ShaderKind::Surface,
                samples_behind: shader.kind == morf_shader::ShaderKind::Effect,
                vertex: shader.vertex.as_ref().map(|compiled| compiled.wgsl.clone()),
                textures: shader.texture_paths.clone(),
                data: shader.compiled.data.clone(),
            })
            .collect()
    }

    /// Whether any registered shader animates on its own.
    ///
    /// A shader that reads the clock has to repaint every frame; one that does
    /// not costs nothing after the first. Derived from the compiler rather than
    /// declared, so it cannot be forgotten.
    pub fn shaders_animate(&self) -> bool {
        self.reactive.borrow().shaders.values().any(|shader| {
            shader.compiled.reads_time
                || shader
                    .vertex
                    .as_ref()
                    .is_some_and(|vertex| vertex.reads_time)
        })
    }
}
