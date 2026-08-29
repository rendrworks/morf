//! Sandboxed execution of mold configuration code.

use std::cell::{Ref, RefCell, RefMut};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use luna::{
    Callback, CallbackReturn, Closure, Context, Executor, ExecutorMode, Fuel, Function, Lua,
    StashedClosure, Table, UserData, UserRef, Value as LuaValue, Variadic,
};
use mold_io::{
    Bus, DbusProxy, DbusSignal, DbusValue, FileEvent, FileView, FileWatcher, LineParser, Process,
    ProcessEvent, Socket, SocketServer, SplitParser, Timer as IoTimer,
};
use mold_reactive::{EffectContext, Graph, SignalId};
use mold_scene::{
    AnimationFrame, Behavior, Easing, Element, FlickState, ListChange, ListModel, ModelId,
    NodeHandle, Physics, Scene, Value as SceneValue, ViewTransition, VirtualList,
};
use mold_services::{
    AuthMessageType, GreetdClient, GreetdResponse, PamAuthenticator, PamTask, PipeWire,
    StatusNotifierAddress, StatusNotifierHost, UdevEvent, UdevMonitor, XkbKeymap,
};

/// Execution limits applied independently to each loaded chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum VM fuel a chunk may consume.
    pub fuel: u64,
    /// Maximum bytes owned by the Lua state.
    pub memory: usize,
    /// VM fuel granted before the host regains control.
    pub slice_fuel: i32,
    /// Maximum VM fuel granted to one reactive Lua effect.
    pub effect_fuel: u64,
    /// Maximum VM fuel granted to all effects in one recompute pass.
    pub frame_fuel: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 10_000_000,
            memory: 64 * 1024 * 1024,
            slice_fuel: 4_096,
            effect_fuel: 100_000,
            frame_fuel: 1_000_000,
        }
    }
}

/// A configuration execution failure.
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    /// The source could not be compiled.
    Load(String),
    /// Execution stopped with a Lua error.
    Runtime(String),
    /// Execution exceeded its instruction budget.
    FuelExhausted { budget: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(message) => write!(f, "could not load Lua: {message}"),
            Self::Runtime(message) => write!(f, "Lua error: {message}"),
            Self::FuelExhausted { budget } => {
                write!(f, "Lua fuel exhausted after {budget} instructions")
            }
        }
    }
}

impl StdError for Error {}

/// The Luna VM owned behind mold's stable runtime boundary.
pub struct Runtime {
    lua: Lua,
    limits: Limits,
    reactive: Rc<RefCell<ReactiveState>>,
    module_roots: Rc<RefCell<Vec<PathBuf>>>,
}

/// Output metadata exposed to one per-screen Lua configuration instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Screen {
    /// Compositor output name.
    pub name: String,
    /// Logical output width when advertised.
    pub width: Option<i32>,
    /// Logical output height when advertised.
    pub height: Option<i32>,
    /// Integer fallback scale advertised by wl_output.
    pub scale: i32,
}

/// Deferred parent and anchor transition requested by Lua.
#[derive(Clone, Debug)]
pub struct ParentTransitionRequest {
    pub node: NodeHandle,
    pub parent: NodeHandle,
    pub anchors: Option<std::collections::BTreeMap<String, SceneValue>>,
    pub behavior: Behavior,
}

/// Primitive value accepted by the bounded IPC surface.
#[derive(Clone, Debug, PartialEq)]
pub enum IpcValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

/// Deferred virtual keyboard request produced by Lua.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualKeyboardRequest {
    /// One evdev keycode state change.
    Key { keycode: u32, pressed: bool },
    /// XKB modifier masks and layout group.
    Modifiers {
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    },
}

/// Deferred input-method-v2 request produced by Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputMethodRequest {
    /// Inserts committed UTF-8 text.
    Commit(String),
    /// Replaces the preedit string and cursor range.
    Preedit { text: String, begin: i32, end: i32 },
    /// Deletes byte ranges around the cursor.
    Delete { before: u32, after: u32 },
}

/// Deferred text-input-v3 state request produced by Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextInputRequest {
    Disable,
    Surrounding {
        text: String,
        cursor: i32,
        anchor: i32,
    },
    ContentType {
        hints: u32,
        purpose: u32,
    },
    CursorRect {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

/// One compositor output capture delivered to Lua.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Screencopy {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Bytes between adjacent rows.
    pub stride: u32,
    /// Shared-memory pixel format name.
    pub format: String,
    /// Whether rows are ordered bottom-to-top.
    pub y_invert: bool,
    /// Captured bytes including stride padding.
    pub pixels: Vec<u8>,
}

/// Correlated output-capture request queued by Lua.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreencopyRequest {
    /// Runtime-local request identifier.
    pub id: u64,
    /// Whether the compositor should include the cursor image.
    pub include_cursor: bool,
}

impl IpcValue {
    fn to_lua<'gc>(&self, ctx: Context<'gc>) -> LuaValue<'gc> {
        match self {
            Self::Nil => LuaValue::Nil,
            Self::Boolean(value) => LuaValue::Boolean(*value),
            Self::Integer(value) => LuaValue::Integer(*value),
            Self::Number(value) => LuaValue::Number(*value),
            Self::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        }
    }

    fn from_lua(value: LuaValue<'_>) -> Result<Self, String> {
        match value {
            LuaValue::Nil => Ok(Self::Nil),
            LuaValue::Boolean(value) => Ok(Self::Boolean(value)),
            LuaValue::Integer(value) => Ok(Self::Integer(value)),
            LuaValue::Number(value) if value.is_finite() => Ok(Self::Number(value)),
            LuaValue::String(value) => Ok(Self::String(value.display_lossy().to_string())),
            value => Err(format!(
                "IPC values must be nil, boolean, number, or string, found {}",
                value.type_name()
            )),
        }
    }
}

fn script_ipc_value(value: &ScriptValue) -> IpcValue {
    match value {
        ScriptValue::Nil => IpcValue::Nil,
        ScriptValue::Boolean(value) => IpcValue::Boolean(*value),
        ScriptValue::Integer(value) => IpcValue::Integer(*value),
        ScriptValue::Number(value) => IpcValue::Number(*value),
        ScriptValue::String(value) => IpcValue::String(value.clone()),
    }
}

fn ipc_script_value(value: IpcValue) -> ScriptValue {
    match value {
        IpcValue::Nil => ScriptValue::Nil,
        IpcValue::Boolean(value) => ScriptValue::Boolean(value),
        IpcValue::Integer(value) => ScriptValue::Integer(value),
        IpcValue::Number(value) => ScriptValue::Number(value),
        IpcValue::String(value) => ScriptValue::String(value),
    }
}

/// Event name accepted by Lua element handlers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UiEvent {
    /// Pointer entered the target.
    PointerEntered,
    /// Pointer left the target.
    PointerExited,
    /// Pointer moved over or while grabbing the target.
    PointerMoved,
    /// Pointer button was pressed on the target.
    Pressed,
    /// Pointer button was released after pressing the target.
    Released,
    /// Pointer press and release completed on the same target.
    Clicked,
    /// A pointer drag crossed the movement threshold.
    DragStarted,
    /// A pointer drag moved after crossing the threshold.
    Dragged,
    /// A pointer drag ended.
    DragFinished,
    /// A key was pressed while the target held focus.
    KeyPressed,
    /// A touch contact began on the target.
    TouchPressed,
    /// A grabbed touch contact moved.
    TouchMoved,
    /// A grabbed touch contact ended.
    TouchReleased,
    /// A grabbed touch contact was cancelled.
    TouchCanceled,
}

impl UiEvent {
    fn property(self) -> &'static str {
        match self {
            Self::PointerEntered => "on_entered",
            Self::PointerExited => "on_exited",
            Self::PointerMoved => "on_position_changed",
            Self::Pressed => "on_pressed",
            Self::Released => "on_released",
            Self::Clicked => "on_clicked",
            Self::DragStarted => "on_drag_started",
            Self::Dragged => "on_dragged",
            Self::DragFinished => "on_drag_finished",
            Self::KeyPressed => "on_key_pressed",
            Self::TouchPressed => "on_touch_pressed",
            Self::TouchMoved => "on_touch_moved",
            Self::TouchReleased => "on_touch_released",
            Self::TouchCanceled => "on_touch_canceled",
        }
    }
}

impl Runtime {
    /// Creates a sandboxed runtime with the supplied limits.
    pub fn new(limits: Limits) -> Self {
        Self::with_screen(limits, None)
    }

    /// Creates a runtime whose `mold.screens` model contains one output.
    pub fn for_screen(limits: Limits, screen: Screen) -> Self {
        Self::with_screen(limits, Some(screen))
    }

    fn with_screen(limits: Limits, screen: Option<Screen>) -> Self {
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
                        "depth {}: {} <- {} (1 evaluation/frame)",
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
                    .map(|value| (name.clone(), script_ipc_value(value)))
            })
            .collect()
    }

    /// Seeds reloadable values before executing replacement configuration code.
    pub fn restore_reloadable_state(&mut self, values: BTreeMap<String, IpcValue>) {
        self.reactive.borrow_mut().reload_seed = values
            .into_iter()
            .map(|(name, value)| (name, ipc_script_value(value)))
            .collect();
    }

    /// Borrows the scene produced by executed configuration code.
    pub fn scene(&self) -> Ref<'_, Scene> {
        Ref::map(self.reactive.borrow(), |state| &state.scene)
    }

    /// Mutably borrows the scene for frame-pipeline structural operations.
    pub fn scene_mut(&mut self) -> RefMut<'_, Scene> {
        RefMut::map(self.reactive.borrow_mut(), |state| &mut state.scene)
    }

    /// Drains parent transitions queued by Lua handlers.
    pub fn take_parent_transitions(&mut self) -> Vec<ParentTransitionRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().parent_transitions)
    }

    /// Advances animations and recomputes bindings that read current values.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, Error> {
        let frame = self
            .reactive
            .borrow_mut()
            .scene
            .tick_animations(delta)
            .map_err(|error| Error::Runtime(error.to_string()))?;
        {
            let mut state = self.reactive.borrow_mut();
            for change in &frame.changes {
                bump_property_signal(&mut state, change.node, change.property, false)
                    .map_err(Error::Runtime)?;
            }
        }
        self.lua
            .enter(|ctx| flush_reactive(&self.reactive, ctx, self.limits))
            .map_err(Error::Runtime)?;
        Ok(frame)
    }

    /// Updates the clock service signal and recomputes dependent Lua bindings.
    pub fn update_clock(&mut self, value: impl Into<String>) -> Result<(), Error> {
        let value = ScriptValue::String(value.into());
        {
            let mut state = self.reactive.borrow_mut();
            let clock = state.clock;
            state
                .graph
                .as_mut()
                .ok_or_else(|| Error::Runtime("reactive graph is already running".to_owned()))?
                .write(clock, value.clone())
                .map_err(|error| Error::Runtime(error.to_string()))?;
            state.values.insert(clock, value);
        }
        self.lua
            .enter(|ctx| flush_reactive(&self.reactive, ctx, self.limits))
            .map_err(Error::Runtime)
    }

    /// Returns the number of Lua effect evaluations performed by this runtime.
    pub fn effect_runs(&self) -> u64 {
        self.reactive.borrow().effect_runs
    }

    /// Runs one bounded Lua UI handler and retains failures as runtime logs.
    pub fn dispatch_ui_event(&mut self, node: NodeHandle, event: UiEvent) -> bool {
        self.dispatch_ui_event_with_args(node, event, &[])
    }

    /// Returns compositor idle thresholds requested by Lua callbacks.
    pub fn idle_timeouts(&self) -> Vec<u32> {
        let mut timeouts = self
            .reactive
            .borrow()
            .idle_callbacks
            .keys()
            .copied()
            .collect::<Vec<_>>();
        timeouts.sort_unstable();
        timeouts
    }

    /// Dispatches one compositor idle state change to registered Lua callbacks.
    pub fn dispatch_idle(&mut self, timeout_ms: u32, idle: bool) -> bool {
        let callbacks = self
            .reactive
            .borrow()
            .idle_callbacks
            .get(&timeout_ms)
            .cloned()
            .unwrap_or_default();
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, &[IpcValue::Boolean(idle)], self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("idle callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending compositor output power requests.
    pub fn take_output_power_requests(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.reactive.borrow_mut().output_power_requests)
    }

    /// Takes pending compositor clipboard publications.
    pub fn take_clipboard_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.reactive.borrow_mut().clipboard_requests)
    }

    /// Dispatches a compositor clipboard selection to registered Lua callbacks.
    pub fn dispatch_clipboard(&mut self, text: Option<String>) -> bool {
        let callbacks = self.reactive.borrow().clipboard_callbacks.clone();
        let value = text.map_or(IpcValue::Nil, IpcValue::String);
        for callback in &callbacks {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_handler_args(ctx, callback, std::slice::from_ref(&value), self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("clipboard callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes pending output-capture requests.
    pub fn take_screencopy_requests(&mut self) -> Vec<ScreencopyRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().screencopy_requests)
    }

    /// Dispatches one output capture to its requesting Lua callback.
    pub fn dispatch_screencopy(
        &mut self,
        request_id: u64,
        result: Result<Screencopy, String>,
    ) -> bool {
        let Some(callback) = self
            .reactive
            .borrow_mut()
            .screencopy_callbacks
            .remove(&request_id)
        else {
            return false;
        };
        if let Err(message) = self
            .lua
            .enter(|ctx| execute_screencopy_handler(ctx, &callback, result, self.limits))
        {
            self.reactive
                .borrow_mut()
                .logs
                .push(format!("screencopy callback: {message}"));
        }
        true
    }

    /// Takes pending virtual keyboard protocol requests.
    pub fn take_virtual_keyboard_requests(&mut self) -> Vec<VirtualKeyboardRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().virtual_keyboard_requests)
    }

    /// Takes whether Lua requested the compositor input-method role.
    pub fn take_input_method_enable_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().input_method_enable_requested)
    }

    /// Takes pending input-method protocol requests.
    pub fn take_input_method_requests(&mut self) -> Vec<InputMethodRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().input_method_requests)
    }

    /// Dispatches an atomically committed input-method context to Lua.
    pub fn dispatch_input_method(
        &mut self,
        active: bool,
        surrounding_text: Option<String>,
        cursor: u32,
        anchor: u32,
        serial: u32,
    ) -> bool {
        let callbacks = self.reactive.borrow().input_method_callbacks.clone();
        let args = [
            IpcValue::Boolean(active),
            surrounding_text.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(cursor)),
            IpcValue::Integer(i64::from(anchor)),
            IpcValue::Integer(i64::from(serial)),
        ];
        for callback in &callbacks {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("input method callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Takes whether Lua requested text-input-v3 creation.
    pub fn take_text_input_enable_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().text_input_enable_requested)
    }

    /// Takes pending text-input-v3 state requests.
    pub fn take_text_input_requests(&mut self) -> Vec<TextInputRequest> {
        std::mem::take(&mut self.reactive.borrow_mut().text_input_requests)
    }

    /// Dispatches one atomically committed text-input edit batch to Lua.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_text_input(
        &mut self,
        focused: bool,
        preedit: Option<String>,
        preedit_begin: i32,
        preedit_end: i32,
        commit: Option<String>,
        delete_before: u32,
        delete_after: u32,
        serial: u32,
    ) -> bool {
        let callbacks = self.reactive.borrow().text_input_callbacks.clone();
        let args = [
            IpcValue::Boolean(focused),
            preedit.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(preedit_begin)),
            IpcValue::Integer(i64::from(preedit_end)),
            commit.map_or(IpcValue::Nil, IpcValue::String),
            IpcValue::Integer(i64::from(delete_before)),
            IpcValue::Integer(i64::from(delete_after)),
            IpcValue::Integer(i64::from(serial)),
        ];
        for callback in &callbacks {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("text input callback: {message}"));
            }
        }
        !callbacks.is_empty()
    }

    /// Runs one bounded key handler with keysym and UTF-8 text arguments.
    pub fn dispatch_key_event(
        &mut self,
        node: NodeHandle,
        keysym: u32,
        text: Option<&str>,
    ) -> bool {
        self.dispatch_ui_event_with_args(
            node,
            UiEvent::KeyPressed,
            &[
                IpcValue::Integer(keysym as i64),
                text.map_or(IpcValue::Nil, |value| IpcValue::String(value.to_owned())),
            ],
        )
    }

    /// Dispatches one touch event with contact identity and surface coordinates.
    pub fn dispatch_touch_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        id: i32,
        x: f64,
        y: f64,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::TouchPressed
                | UiEvent::TouchMoved
                | UiEvent::TouchReleased
                | UiEvent::TouchCanceled
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Integer(i64::from(id)),
                IpcValue::Number(x),
                IpcValue::Number(y),
            ],
        )
    }

    /// Dispatches pointer coordinates and displacement to a movement handler.
    pub fn dispatch_pointer_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::PointerMoved | UiEvent::DragStarted | UiEvent::Dragged | UiEvent::DragFinished
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Number(x),
                IpcValue::Number(y),
                IpcValue::Number(delta_x),
                IpcValue::Number(delta_y),
            ],
        )
    }

    /// Returns whether a MouseArea accepts one Linux input button code.
    pub fn accepts_pointer_button(&self, node: NodeHandle, button: u32) -> bool {
        let state = self.reactive.borrow();
        let Ok(value) = state.scene.current(node, "accepted_buttons") else {
            return false;
        };
        let accepted = |value: &SceneValue| match value {
            SceneValue::String(name) => match name.as_str() {
                "all" => true,
                "left" => button == 0x110,
                "right" => button == 0x111,
                "middle" => button == 0x112,
                _ => false,
            },
            SceneValue::Number(code) => *code == f64::from(button),
            _ => false,
        };
        match value {
            SceneValue::List(values) => values.iter().any(accepted),
            value => accepted(value),
        }
    }

    /// Returns the first scene node with a key handler in tree order.
    pub fn first_key_target(&self) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets(&state);
        targets
            .iter()
            .copied()
            .find(|node| state.scene.bool_value(*node, "focus").unwrap_or(false))
            .or_else(|| targets.first().copied())
    }

    /// Returns the nearest key-handling ancestor of a hit-tested node.
    pub fn key_target_for_node(&self, node: NodeHandle) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let mut current = Some(node);
        while let Some(node) = current {
            if state.handlers.contains_key(&(node, UiEvent::KeyPressed))
                && state.scene.bool_value(node, "enabled").unwrap_or(false)
                && state.scene.bool_value(node, "visible").unwrap_or(false)
            {
                return Some(node);
            }
            current = state.scene.parent(node).ok().flatten();
        }
        None
    }

    /// Advances keyboard focus through enabled visible key handlers.
    pub fn next_key_target(&self, current: Option<NodeHandle>) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let targets = key_targets(&state);
        if targets.is_empty() {
            return None;
        }
        let next = current
            .and_then(|current| targets.iter().position(|node| *node == current))
            .map_or(0, |index| (index + 1) % targets.len());
        Some(targets[next])
    }

    fn dispatch_ui_event_with_args(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        args: &[IpcValue],
    ) -> bool {
        let handler = self.reactive.borrow().handlers.get(&(node, event)).cloned();
        let Some(handler) = handler else {
            return false;
        };
        let result = self
            .lua
            .enter(|ctx| execute_handler_args(ctx, &handler, args, self.limits));
        if let Err(message) = result {
            self.reactive.borrow_mut().logs.push(format!(
                "{:?}.{}: {message}",
                node,
                event.property()
            ));
        }
        true
    }

    /// Polls native service jobs and runs completed callbacks with bounded fuel.
    pub fn poll_services(&mut self) -> bool {
        let mut ready = Vec::new();
        let mut timers = Vec::new();
        let mut dbus_signals = Vec::new();
        let mut udev_events = Vec::new();
        let mut status_updates = Vec::new();
        let mut loaders = Vec::new();
        let mut service_changed = false;
        {
            let mut state = self.reactive.borrow_mut();
            let mut index = 0;
            while index < state.pam_tasks.len() {
                let result = state.pam_tasks[index].task.wait(Duration::ZERO);
                if let Some(result) = result {
                    let task = state.pam_tasks.swap_remove(index);
                    ready.push((task.callback, task.unlock_on_success, result));
                } else {
                    index += 1;
                }
            }
            let timer_definitions = state
                .timer_callbacks
                .iter()
                .map(|(node, callback)| (*node, callback.clone()))
                .collect::<Vec<_>>();
            let mut stale_timers = Vec::new();
            for (node, callback) in timer_definitions {
                let Ok(running) = state.scene.bool_value(node, "running") else {
                    stale_timers.push(node);
                    continue;
                };
                let interval = state.scene.number(node, "interval").unwrap_or(0.0);
                let repeat = state.scene.bool_value(node, "repeat").unwrap_or(false);
                let duration = (interval.is_finite() && interval > 0.0)
                    .then(|| Duration::from_secs_f64(interval / 1_000.0));
                let current = state
                    .timers
                    .iter()
                    .position(|timer| timer.node == Some(node));
                if !running || duration.is_none() {
                    if let Some(index) = current {
                        state.timers.swap_remove(index);
                        service_changed = true;
                    }
                    continue;
                }
                let duration = duration.expect("validated duration");
                let matches = current.is_some_and(|index| {
                    state.timers[index].interval == duration && state.timers[index].repeat == repeat
                });
                if matches {
                    continue;
                }
                if let Some(index) = current {
                    state.timers.swap_remove(index);
                }
                match IoTimer::every(duration) {
                    Ok(timer) => state.timers.push(PendingTimer {
                        timer,
                        callback,
                        repeat,
                        interval: duration,
                        node: Some(node),
                    }),
                    Err(error) => state.logs.push(format!("Timer: {error}")),
                }
                service_changed = true;
            }
            for node in stale_timers {
                state.timer_callbacks.remove(&node);
                state.timers.retain(|timer| timer.node != Some(node));
            }
            let loader_definitions = state
                .loader_factories
                .iter()
                .map(|(node, factory)| (*node, factory.clone()))
                .collect::<Vec<_>>();
            let mut stale_loaders = Vec::new();
            for (node, factory) in loader_definitions {
                let Ok(active) = state.scene.bool_value(node, "active") else {
                    stale_loaders.push(node);
                    continue;
                };
                if active && state.loaded_loaders.insert(node) {
                    loaders.push((node, factory));
                } else if !active && state.loaded_loaders.remove(&node) {
                    let children = state.scene.children(node).unwrap_or_default();
                    for child in children {
                        let _ = state.scene.remove(child);
                    }
                    service_changed = true;
                }
            }
            for node in stale_loaders {
                state.loader_factories.remove(&node);
                state.loaded_loaders.remove(&node);
            }
            let mut index = 0;
            while index < state.timers.len() {
                if state.timers[index].timer.tick(Duration::ZERO) {
                    timers.push(state.timers[index].callback.clone());
                    if !state.timers[index].repeat {
                        if let Some(node) = state.timers[index].node {
                            let _ = assign_scene_property(
                                &mut state,
                                node,
                                "running",
                                SceneValue::Bool(false),
                            );
                        }
                        state.timers.swap_remove(index);
                        continue;
                    }
                }
                index += 1;
            }
            for subscription in &state.dbus_signals {
                while let Some(value) = subscription.signal.next_value(Duration::ZERO) {
                    dbus_signals.push((subscription.callback.clone(), value));
                }
            }
            let mut udev_errors = Vec::new();
            for subscription in &state.udev_monitors {
                for _ in 0..32 {
                    match subscription.monitor.next_event(Duration::ZERO) {
                        Ok(Some(event)) => {
                            udev_events.push((subscription.callback.clone(), event));
                        }
                        Ok(None) => break,
                        Err(error) => {
                            udev_errors.push(error.to_string());
                            break;
                        }
                    }
                }
            }
            state.logs.extend(
                udev_errors
                    .into_iter()
                    .map(|error| format!("udev: {error}")),
            );
            let mut status_errors = Vec::new();
            for subscription in &mut state.status_notifiers {
                match subscription.host.poll_changed() {
                    Ok(Some(items)) => status_updates.push((subscription.callback.clone(), items)),
                    Ok(None) => {}
                    Err(error) => status_errors.push(error.to_string()),
                }
            }
            state.logs.extend(
                status_errors
                    .into_iter()
                    .map(|error| format!("status notifier: {error}")),
            );
        }
        for (node, factory) in loaders {
            let result = self
                .lua
                .enter(|ctx| execute_node_factory(ctx, &factory, self.limits));
            match result {
                Ok(child) => {
                    let mut state = self.reactive.borrow_mut();
                    if state.scene.reparent(child, Some(node)).is_ok() {
                        service_changed = true;
                    } else {
                        let _ = state.scene.remove(child);
                    }
                }
                Err(error) => self
                    .reactive
                    .borrow_mut()
                    .logs
                    .push(format!("Loader: {error}")),
            }
        }
        let changed = service_changed
            || !ready.is_empty()
            || !timers.is_empty()
            || !dbus_signals.is_empty()
            || !udev_events.is_empty()
            || !status_updates.is_empty();
        for (callback, unlock_on_success, result) in ready {
            if unlock_on_success && result.is_ok() {
                self.reactive.borrow_mut().session_unlock_requested = true;
            }
            let args = match result {
                Ok(()) => vec![IpcValue::Boolean(true), IpcValue::Nil],
                Err(error) => vec![
                    IpcValue::Boolean(false),
                    IpcValue::String(error.to_string()),
                ],
            };
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, &callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("PAM callback: {message}"));
            }
        }
        for callback in timers {
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, &callback, &[], self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("timer callback: {message}"));
            }
        }
        for (callback, value) in dbus_signals {
            let value = match value {
                Ok(value) => value,
                Err(message) => {
                    self.reactive
                        .borrow_mut()
                        .logs
                        .push(format!("D-Bus signal: {message}"));
                    continue;
                }
            };
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_dbus_handler(ctx, &callback, value, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("D-Bus callback: {message}"));
            }
        }
        for (callback, event) in udev_events {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_dbus_handler(ctx, &callback, udev_event_value(event), self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("udev callback: {message}"));
            }
        }
        for (callback, items) in status_updates {
            if let Err(message) = self.lua.enter(|ctx| {
                execute_dbus_handler(ctx, &callback, status_notifier_value(items), self.limits)
            }) {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("status notifier callback: {message}"));
            }
        }
        changed
    }

    /// Takes a successful native authentication request to release a session lock.
    pub fn take_session_unlock_request(&mut self) -> bool {
        std::mem::take(&mut self.reactive.borrow_mut().session_unlock_requested)
    }

    /// Returns registered IPC verb names in lexical order.
    pub fn ipc_verbs(&self) -> Vec<String> {
        let mut verbs = self
            .reactive
            .borrow()
            .ipc_handlers
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        verbs.sort();
        verbs
    }

    /// Calls one registered IPC handler with bounded primitive arguments.
    pub fn call_ipc(&mut self, verb: &str, args: &[IpcValue]) -> Result<Vec<IpcValue>, Error> {
        let handler = self
            .reactive
            .borrow()
            .ipc_handlers
            .get(verb)
            .cloned()
            .ok_or_else(|| Error::Runtime(format!("unknown IPC verb `{verb}`")))?;
        self.lua
            .enter(|ctx| execute_ipc_handler(ctx, &handler, args, self.limits))
            .map_err(Error::Runtime)
    }
}

fn key_targets(state: &ReactiveState) -> Vec<NodeHandle> {
    let mut targets = Vec::new();
    let mut pending = state.scene.roots();
    pending.reverse();
    while let Some(node) = pending.pop() {
        if state.handlers.contains_key(&(node, UiEvent::KeyPressed))
            && state.scene.bool_value(node, "enabled").unwrap_or(false)
            && state.scene.bool_value(node, "visible").unwrap_or(false)
        {
            targets.push(node);
        }
        if let Ok(mut children) = state.scene.children(node) {
            children.reverse();
            pending.extend(children);
        }
    }
    targets
}

#[derive(Clone, Debug, PartialEq)]
enum ScriptValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

impl ScriptValue {
    fn from_lua(value: LuaValue<'_>) -> Result<Self, String> {
        match value {
            LuaValue::Nil => Ok(Self::Nil),
            LuaValue::Boolean(value) => Ok(Self::Boolean(value)),
            LuaValue::Integer(value) => Ok(Self::Integer(value)),
            LuaValue::Number(value) if value.is_finite() => Ok(Self::Number(value)),
            LuaValue::String(value) => Ok(Self::String(value.display_lossy().to_string())),
            value => Err(format!(
                "reactive signals do not support {} values yet",
                value.type_name()
            )),
        }
    }

    fn to_lua<'gc>(&self, ctx: Context<'gc>) -> LuaValue<'gc> {
        match self {
            Self::Nil => LuaValue::Nil,
            Self::Boolean(value) => LuaValue::Boolean(*value),
            Self::Integer(value) => LuaValue::Integer(*value),
            Self::Number(value) => LuaValue::Number(*value),
            Self::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        }
    }

    fn to_scene(&self) -> SceneValue {
        match self {
            Self::Nil => SceneValue::Nil,
            Self::Boolean(value) => SceneValue::Bool(*value),
            Self::Integer(value) => SceneValue::Number(*value as f64),
            Self::Number(value) => SceneValue::Number(*value),
            Self::String(value) => SceneValue::String(value.clone()),
        }
    }
}

#[derive(Debug)]
struct SignalToken {
    id: SignalId,
}

#[derive(Debug)]
struct NodeToken {
    handle: NodeHandle,
}

#[derive(Debug)]
struct DbusToken {
    proxy: DbusProxy,
}

struct PipeWireToken {
    service: PipeWire,
}

struct GreetdToken {
    client: RefCell<GreetdClient>,
}

struct ProcessToken {
    process: RefCell<Process>,
}

struct FileToken {
    file: FileView,
}

struct FileWatcherToken {
    watcher: FileWatcher,
}

struct SocketToken {
    socket: RefCell<Socket>,
}

struct SocketServerToken {
    server: SocketServer,
}

struct LineParserToken {
    parser: RefCell<LineParser>,
}

struct SplitParserToken {
    parser: RefCell<SplitParser>,
}

struct ListModelToken {
    model: Rc<RefCell<ListModel>>,
}

struct VirtualListToken {
    model: Rc<RefCell<ListModel>>,
    view: RefCell<VirtualList>,
}

struct LuaVirtualView {
    model: Rc<RefCell<ListModel>>,
    view: VirtualList,
    delegate: StashedClosure,
    active: HashMap<ModelId, NodeHandle>,
    column_extent: f64,
}

#[derive(Clone, Copy)]
enum ViewKind {
    Repeater,
    List,
    Grid,
}

struct FlickToken {
    state: RefCell<FlickState>,
}

struct PendingPam {
    task: PamTask,
    callback: StashedClosure,
    unlock_on_success: bool,
}

struct PendingTimer {
    timer: IoTimer,
    callback: StashedClosure,
    repeat: bool,
    interval: Duration,
    node: Option<NodeHandle>,
}

struct PendingDbusSignal {
    signal: DbusSignal,
    callback: StashedClosure,
}

struct PendingUdev {
    monitor: UdevMonitor,
    callback: StashedClosure,
}

struct PendingStatusNotifier {
    host: StatusNotifierHost,
    callback: StashedClosure,
}

#[derive(Clone)]
struct LuaEffect {
    closure: StashedClosure,
    sink: Option<EffectSink>,
}

#[derive(Clone)]
struct PropertySink {
    node: NodeHandle,
    property: String,
}

#[derive(Clone)]
enum EffectSink {
    Property(PropertySink),
    State(NodeHandle),
}

#[derive(Clone)]
struct StateDefinition {
    properties: Vec<(String, StateValue)>,
    anchors: Option<std::collections::BTreeMap<String, SceneValue>>,
    parent: Option<NodeHandle>,
}

#[derive(Clone)]
enum StateValue {
    Value(SceneValue),
    Binding(StashedClosure),
}

#[derive(Clone)]
struct StateTransition {
    from: String,
    to: String,
    reversible: bool,
    behavior: Behavior,
}

#[derive(Default)]
struct StateSet {
    definitions: HashMap<String, StateDefinition>,
    transitions: Vec<StateTransition>,
    current: Option<String>,
}

#[derive(Default)]
struct Capture {
    reads: HashSet<SignalId>,
    property_reads: HashSet<(NodeHandle, String, bool)>,
    writes: Vec<(SignalId, ScriptValue)>,
}

struct ReactiveState {
    graph: Option<Graph<ScriptValue>>,
    values: HashMap<SignalId, ScriptValue>,
    signals: Vec<SignalId>,
    property_signals: HashMap<(NodeHandle, String, bool), SignalId>,
    current_property_names: HashMap<String, (NodeHandle, String)>,
    property_revision: i64,
    reload_seed: HashMap<String, ScriptValue>,
    reloadable: HashMap<String, SignalId>,
    effects: HashMap<u64, LuaEffect>,
    next_effect: u64,
    active: Option<Capture>,
    logs: Vec<String>,
    scene: Scene,
    effect_runs: u64,
    clock: SignalId,
    handlers: HashMap<(NodeHandle, UiEvent), StashedClosure>,
    parent_transitions: Vec<ParentTransitionRequest>,
    states: HashMap<NodeHandle, StateSet>,
    ipc_handlers: HashMap<String, StashedClosure>,
    idle_callbacks: HashMap<u32, Vec<StashedClosure>>,
    output_power_requests: Vec<bool>,
    clipboard_requests: Vec<String>,
    clipboard_callbacks: Vec<StashedClosure>,
    screencopy_requests: Vec<ScreencopyRequest>,
    screencopy_callbacks: HashMap<u64, StashedClosure>,
    next_screencopy: u64,
    virtual_keyboard_requests: Vec<VirtualKeyboardRequest>,
    input_method_enable_requested: bool,
    input_method_requests: Vec<InputMethodRequest>,
    input_method_callbacks: Vec<StashedClosure>,
    text_input_enable_requested: bool,
    text_input_requests: Vec<TextInputRequest>,
    text_input_callbacks: Vec<StashedClosure>,
    views: HashMap<NodeHandle, LuaVirtualView>,
    pam_tasks: Vec<PendingPam>,
    timers: Vec<PendingTimer>,
    timer_callbacks: HashMap<NodeHandle, StashedClosure>,
    loader_factories: HashMap<NodeHandle, StashedClosure>,
    loaded_loaders: HashSet<NodeHandle>,
    dbus_signals: Vec<PendingDbusSignal>,
    udev_monitors: Vec<PendingUdev>,
    status_notifiers: Vec<PendingStatusNotifier>,
    session_unlock_requested: bool,
}

impl ReactiveState {
    fn new() -> Self {
        let mut graph = Graph::default();
        let initial_clock = ScriptValue::String(String::new());
        let clock = graph.signal("mold.clock", initial_clock.clone());
        let mut values = HashMap::new();
        values.insert(clock, initial_clock);
        Self {
            graph: Some(graph),
            values,
            signals: vec![clock],
            property_signals: HashMap::new(),
            current_property_names: HashMap::new(),
            property_revision: 0,
            reload_seed: HashMap::new(),
            reloadable: HashMap::new(),
            effects: HashMap::new(),
            next_effect: 0,
            active: None,
            logs: Vec::new(),
            scene: Scene::new(),
            effect_runs: 0,
            clock,
            handlers: HashMap::new(),
            parent_transitions: Vec::new(),
            states: HashMap::new(),
            ipc_handlers: HashMap::new(),
            idle_callbacks: HashMap::new(),
            output_power_requests: Vec::new(),
            clipboard_requests: Vec::new(),
            clipboard_callbacks: Vec::new(),
            screencopy_requests: Vec::new(),
            screencopy_callbacks: HashMap::new(),
            next_screencopy: 0,
            virtual_keyboard_requests: Vec::new(),
            input_method_enable_requested: false,
            input_method_requests: Vec::new(),
            input_method_callbacks: Vec::new(),
            text_input_enable_requested: false,
            text_input_requests: Vec::new(),
            text_input_callbacks: Vec::new(),
            views: HashMap::new(),
            pam_tasks: Vec::new(),
            timers: Vec::new(),
            timer_callbacks: HashMap::new(),
            loader_factories: HashMap::new(),
            loaded_loaders: HashSet::new(),
            dbus_signals: Vec::new(),
            udev_monitors: Vec::new(),
            status_notifiers: Vec::new(),
            session_unlock_requested: false,
        }
    }
}

fn create_node(state: &Rc<RefCell<ReactiveState>>, element: Element) -> NodeHandle {
    state.borrow_mut().scene.create(element)
}

fn bump_property_signal(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    target: bool,
) -> Result<(), String> {
    let Some(signal) = state
        .property_signals
        .get(&(node, property.to_owned(), target))
        .copied()
    else {
        return Ok(());
    };
    state.property_revision = state.property_revision.wrapping_add(1);
    let value = ScriptValue::Integer(state.property_revision);
    if let Some(active) = &mut state.active {
        active.writes.push((signal, value.clone()));
    } else {
        state
            .graph
            .as_mut()
            .ok_or_else(|| "reactive graph is already running".to_owned())?
            .write(signal, value.clone())
            .map_err(|error| error.to_string())?;
    }
    state.values.insert(signal, value);
    Ok(())
}

fn assign_scene_property(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    value: SceneValue,
) -> Result<(), String> {
    let old_current = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    let old_target = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    state
        .scene
        .assign(node, property, value)
        .map_err(|error| error.to_string())?;
    let current_changed = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        != &old_current;
    let target_changed = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        != &old_target;
    if current_changed {
        bump_property_signal(state, node, property, false)?;
    }
    if target_changed {
        bump_property_signal(state, node, property, true)?;
    }
    Ok(())
}

fn animate_scene_property(
    state: &mut ReactiveState,
    node: NodeHandle,
    property: &str,
    from: SceneValue,
    to: SceneValue,
    behavior: Behavior,
) -> Result<(), String> {
    let old_current = state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    let old_target = state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        .clone();
    state
        .scene
        .animate_from(node, property, from, to, behavior)
        .map_err(|error| error.to_string())?;
    if state
        .scene
        .current(node, property)
        .map_err(|error| error.to_string())?
        != &old_current
    {
        bump_property_signal(state, node, property, false)?;
    }
    if state
        .scene
        .target(node, property)
        .map_err(|error| error.to_string())?
        != &old_target
    {
        bump_property_signal(state, node, property, true)?;
    }
    Ok(())
}

fn node_userdata<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    handle: NodeHandle,
) -> UserData<'gc> {
    let index = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, key): (UserRef<NodeToken>, String) = stack.consume(ctx)?;
        let (property, target) = key
            .strip_suffix("_target")
            .map_or((key.as_str(), false), |property| (property, true));
        let value = {
            let mut state = state.borrow_mut();
            if !state
                .scene
                .has_property(node.handle, property)
                .map_err(|error| HostError(error.to_string()))?
            {
                return Err(HostError(format!("unknown node property `{key}`")).into());
            }
            let property_key = (node.handle, property.to_owned(), target);
            let signal = state.property_signals.get(&property_key).copied();
            if let Some(active) = &mut state.active {
                if let Some(signal) = signal {
                    active.reads.insert(signal);
                } else {
                    active.property_reads.insert(property_key);
                }
            }
            if target {
                state.scene.target(node.handle, property)
            } else {
                state.scene.current(node.handle, property)
            }
            .map_err(|error| HostError(error.to_string()))?
            .clone()
        };
        stack.replace(ctx, scene_to_lua(ctx, &value).map_err(HostError)?);
        Ok(CallbackReturn::Return)
    });
    let metatable = Table::new(&ctx);
    metatable.set_field(ctx, "__index", index);
    let userdata = UserData::new_static(&ctx, NodeToken { handle });
    userdata.set_metatable(ctx, Some(metatable));
    userdata
}

#[derive(Debug)]
struct HostError(String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for HostError {}

fn install_reactive_api(
    lua: &mut Lua,
    state: Rc<RefCell<ReactiveState>>,
    module_roots: Rc<RefCell<Vec<PathBuf>>>,
    limits: Limits,
    screen: Option<&Screen>,
) {
    lua.enter(|ctx| {
        let get = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            move |ctx, _, mut stack| {
                let signal: UserRef<SignalToken> = stack.consume(ctx)?;
                let mut state = state.borrow_mut();
                let value = if let Some(active) = &mut state.active {
                    active.reads.insert(signal.id);
                    active
                        .writes
                        .iter()
                        .rev()
                        .find(|(id, _)| *id == signal.id)
                        .map(|(_, value)| value.clone())
                        .or_else(|| state.values.get(&signal.id).cloned())
                } else {
                    state.values.get(&signal.id).cloned()
                }
                .ok_or_else(|| HostError("stale reactive signal".to_owned()))?;
                stack.replace(ctx, value.to_lua(ctx));
                Ok(CallbackReturn::Return)
            }
        });

        let set = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            move |ctx, _, mut stack| {
                let (signal, value): (UserRef<SignalToken>, LuaValue) = stack.consume(ctx)?;
                let value = ScriptValue::from_lua(value).map_err(HostError)?;
                {
                    let mut state = state.borrow_mut();
                    if let Some(active) = &mut state.active {
                        active.writes.push((signal.id, value));
                        stack.replace(ctx, true);
                        return Ok(CallbackReturn::Return);
                    }
                    let graph = state
                        .graph
                        .as_mut()
                        .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?;
                    graph
                        .write(signal.id, value.clone())
                        .map_err(|error| HostError(error.to_string()))?;
                    state.values.insert(signal.id, value);
                }
                replace_status(ctx, &mut stack, flush_reactive(&state, ctx, limits));
                Ok(CallbackReturn::Return)
            }
        });

        let methods = Table::new(&ctx);
        methods.set_field(ctx, "get", get);
        methods.set_field(ctx, "set", set);
        let signal_metatable = Table::new(&ctx);
        signal_metatable.set_field(ctx, "__index", methods);
        let signal_metatable = ctx.stash(signal_metatable);

        let signal = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            let signal_metatable = signal_metatable.clone();
            move |ctx, _, mut stack| {
                let (name, value): (String, LuaValue) = stack.consume(ctx)?;
                let value = ScriptValue::from_lua(value).map_err(HostError)?;
                let id = {
                    let mut state = state.borrow_mut();
                    let id = state
                        .graph
                        .as_mut()
                        .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?
                        .signal(name, value.clone());
                    state.values.insert(id, value);
                    state.signals.push(id);
                    id
                };
                let userdata = UserData::new_static(&ctx, SignalToken { id });
                userdata.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
                stack.replace(ctx, userdata);
                Ok(CallbackReturn::Return)
            }
        });

        let reloadable = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            let signal_metatable = signal_metatable.clone();
            move |ctx, _, mut stack| {
                let (name, initial): (String, LuaValue) = stack.consume(ctx)?;
                if name.is_empty() {
                    return Err(HostError("reloadable id cannot be empty".into()).into());
                }
                let initial = ScriptValue::from_lua(initial).map_err(HostError)?;
                let id = {
                    let mut state = state.borrow_mut();
                    if state.reloadable.contains_key(&name) {
                        return Err(HostError(format!(
                            "reloadable id `{name}` is already registered"
                        ))
                        .into());
                    }
                    let value = match state.reload_seed.remove(&name) {
                        Some(value)
                            if std::mem::discriminant(&value)
                                == std::mem::discriminant(&initial) =>
                        {
                            value
                        }
                        Some(_) => {
                            state.logs.push(format!(
                                "reloadable `{name}` changed value type; using its new default"
                            ));
                            initial
                        }
                        None => initial,
                    };
                    let id = state
                        .graph
                        .as_mut()
                        .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?
                        .signal(format!("reloadable.{name}"), value.clone());
                    state.values.insert(id, value);
                    state.signals.push(id);
                    state.reloadable.insert(name, id);
                    id
                };
                let userdata = UserData::new_static(&ctx, SignalToken { id });
                userdata.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
                stack.replace(ctx, userdata);
                Ok(CallbackReturn::Return)
            }
        });

        let effect = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            move |ctx, _, mut stack| {
                let (name, closure): (String, Closure) = stack.consume(ctx)?;
                {
                    let mut state = state.borrow_mut();
                    let token = state.next_effect;
                    state.next_effect = state.next_effect.wrapping_add(1);
                    state.effects.insert(
                        token,
                        LuaEffect {
                            closure: ctx.stash(closure),
                            sink: None,
                        },
                    );
                    state
                        .graph
                        .as_mut()
                        .ok_or_else(|| HostError("reactive graph is already running".to_owned()))?
                        .external_effect(name, token);
                }
                replace_status(ctx, &mut stack, flush_reactive(&state, ctx, limits));
                Ok(CallbackReturn::Return)
            }
        });

        let mold = Table::new(&ctx);
        mold.set_field(ctx, "signal", signal);
        mold.set_field(ctx, "reloadable", reloadable);
        mold.set_field(ctx, "effect", effect);
        let clock = UserData::new_static(
            &ctx,
            SignalToken {
                id: state.borrow().clock,
            },
        );
        clock.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
        mold.set_field(ctx, "clock", clock);
        let idle_state = Rc::clone(&state);
        let idle_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (milliseconds, callback): (i64, Closure) = stack.consume(ctx)?;
            let milliseconds = u32::try_from(milliseconds)
                .map_err(|_| HostError("idle timeout must fit an unsigned 32-bit value".into()))?;
            let mut state = idle_state.borrow_mut();
            let callback_count = state.idle_callbacks.values().map(Vec::len).sum::<usize>();
            if callback_count >= 256 {
                return Err(HostError("idle callback limit reached".into()).into());
            }
            if !state.idle_callbacks.contains_key(&milliseconds) && state.idle_callbacks.len() >= 64
            {
                return Err(HostError("idle timeout limit reached".into()).into());
            }
            state
                .idle_callbacks
                .entry(milliseconds)
                .or_default()
                .push(ctx.stash(callback));
            Ok(CallbackReturn::Return)
        });
        let idle = Table::new(&ctx);
        idle.set_field(ctx, "subscribe", idle_subscribe);
        mold.set_field(ctx, "idle", idle);
        let output_power_state = Rc::clone(&state);
        let output_power_set = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let mode: String = stack.consume(ctx)?;
            let on = match mode.as_str() {
                "off" => false,
                "on" => true,
                _ => return Err(HostError("output power mode must be `on` or `off`".into()).into()),
            };
            let mut state = output_power_state.borrow_mut();
            if state.output_power_requests.len() >= 64 {
                return Err(HostError("output power request limit reached".into()).into());
            }
            state.output_power_requests.push(on);
            Ok(CallbackReturn::Return)
        });
        let output_power = Table::new(&ctx);
        output_power.set_field(ctx, "set", output_power_set);
        mold.set_field(ctx, "output_power", output_power);
        let clipboard_set_state = Rc::clone(&state);
        let clipboard_set = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let text: String = stack.consume(ctx)?;
            if text.len() > 1_048_576 {
                return Err(HostError("clipboard text limit reached".into()).into());
            }
            let mut state = clipboard_set_state.borrow_mut();
            if state.clipboard_requests.len() >= 64 {
                return Err(HostError("clipboard request limit reached".into()).into());
            }
            state.clipboard_requests.push(text);
            Ok(CallbackReturn::Return)
        });
        let clipboard_subscribe_state = Rc::clone(&state);
        let clipboard_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let callback: Closure = stack.consume(ctx)?;
            let mut state = clipboard_subscribe_state.borrow_mut();
            if state.clipboard_callbacks.len() >= 64 {
                return Err(HostError("clipboard callback limit reached".into()).into());
            }
            state.clipboard_callbacks.push(ctx.stash(callback));
            Ok(CallbackReturn::Return)
        });
        let clipboard = Table::new(&ctx);
        clipboard.set_field(ctx, "set", clipboard_set);
        clipboard.set_field(ctx, "subscribe", clipboard_subscribe);
        mold.set_field(ctx, "clipboard", clipboard);
        let screencopy_state = Rc::clone(&state);
        let screencopy_capture = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (include_cursor, callback): (bool, Closure) = stack.consume(ctx)?;
            let mut state = screencopy_state.borrow_mut();
            if state.screencopy_callbacks.len() >= 4 {
                return Err(HostError("screencopy request limit reached".into()).into());
            }
            let id = state.next_screencopy;
            state.next_screencopy = state.next_screencopy.wrapping_add(1);
            state
                .screencopy_requests
                .push(ScreencopyRequest { id, include_cursor });
            state.screencopy_callbacks.insert(id, ctx.stash(callback));
            Ok(CallbackReturn::Return)
        });
        let screencopy = Table::new(&ctx);
        screencopy.set_field(ctx, "capture", screencopy_capture);
        mold.set_field(ctx, "screencopy", screencopy);
        let virtual_key_state = Rc::clone(&state);
        let virtual_key = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (keycode, pressed): (i64, bool) = stack.consume(ctx)?;
            let keycode = u32::try_from(keycode).map_err(|_| {
                HostError("virtual keycode must fit an unsigned 32-bit value".into())
            })?;
            let mut state = virtual_key_state.borrow_mut();
            if state.virtual_keyboard_requests.len() >= 256 {
                return Err(HostError("virtual keyboard request limit reached".into()).into());
            }
            state
                .virtual_keyboard_requests
                .push(VirtualKeyboardRequest::Key { keycode, pressed });
            Ok(CallbackReturn::Return)
        });
        let virtual_modifiers_state = Rc::clone(&state);
        let virtual_modifiers = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let values: (i64, i64, i64, i64) = stack.consume(ctx)?;
            let request = VirtualKeyboardRequest::Modifiers {
                depressed: u32::try_from(values.0)
                    .map_err(|_| HostError("depressed modifiers must fit u32".into()))?,
                latched: u32::try_from(values.1)
                    .map_err(|_| HostError("latched modifiers must fit u32".into()))?,
                locked: u32::try_from(values.2)
                    .map_err(|_| HostError("locked modifiers must fit u32".into()))?,
                group: u32::try_from(values.3)
                    .map_err(|_| HostError("keyboard group must fit u32".into()))?,
            };
            let mut state = virtual_modifiers_state.borrow_mut();
            if state.virtual_keyboard_requests.len() >= 256 {
                return Err(HostError("virtual keyboard request limit reached".into()).into());
            }
            state.virtual_keyboard_requests.push(request);
            Ok(CallbackReturn::Return)
        });
        let virtual_keyboard = Table::new(&ctx);
        virtual_keyboard.set_field(ctx, "key", virtual_key);
        virtual_keyboard.set_field(ctx, "modifiers", virtual_modifiers);
        mold.set_field(ctx, "virtual_keyboard", virtual_keyboard);
        let input_method_subscribe_state = Rc::clone(&state);
        let input_method_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let callback: Closure = stack.consume(ctx)?;
            let mut state = input_method_subscribe_state.borrow_mut();
            if state.input_method_callbacks.len() >= 64 {
                return Err(HostError("input method callback limit reached".into()).into());
            }
            state.input_method_callbacks.push(ctx.stash(callback));
            state.input_method_enable_requested = true;
            Ok(CallbackReturn::Return)
        });
        let input_method_commit_state = Rc::clone(&state);
        let input_method_commit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let text: String = stack.consume(ctx)?;
            if text.len() > 4_000 {
                return Err(HostError("input method text limit reached".into()).into());
            }
            let mut state = input_method_commit_state.borrow_mut();
            if state.input_method_requests.len() >= 256 {
                return Err(HostError("input method request limit reached".into()).into());
            }
            state
                .input_method_requests
                .push(InputMethodRequest::Commit(text));
            Ok(CallbackReturn::Return)
        });
        let input_method_preedit_state = Rc::clone(&state);
        let input_method_preedit = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (text, begin, end): (String, i64, i64) = stack.consume(ctx)?;
            if text.len() > 4_000 {
                return Err(HostError("input method text limit reached".into()).into());
            }
            let begin = i32::try_from(begin)
                .map_err(|_| HostError("preedit cursor start must fit i32".into()))?;
            let end = i32::try_from(end)
                .map_err(|_| HostError("preedit cursor end must fit i32".into()))?;
            let mut state = input_method_preedit_state.borrow_mut();
            if state.input_method_requests.len() >= 256 {
                return Err(HostError("input method request limit reached".into()).into());
            }
            state
                .input_method_requests
                .push(InputMethodRequest::Preedit { text, begin, end });
            Ok(CallbackReturn::Return)
        });
        let input_method_delete_state = Rc::clone(&state);
        let input_method_delete = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (before, after): (i64, i64) = stack.consume(ctx)?;
            let before = u32::try_from(before)
                .map_err(|_| HostError("delete before length must fit u32".into()))?;
            let after = u32::try_from(after)
                .map_err(|_| HostError("delete after length must fit u32".into()))?;
            let mut state = input_method_delete_state.borrow_mut();
            if state.input_method_requests.len() >= 256 {
                return Err(HostError("input method request limit reached".into()).into());
            }
            state
                .input_method_requests
                .push(InputMethodRequest::Delete { before, after });
            Ok(CallbackReturn::Return)
        });
        let input_method = Table::new(&ctx);
        input_method.set_field(ctx, "subscribe", input_method_subscribe);
        input_method.set_field(ctx, "commit", input_method_commit);
        input_method.set_field(ctx, "preedit", input_method_preedit);
        input_method.set_field(ctx, "delete", input_method_delete);
        mold.set_field(ctx, "input_method", input_method);
        let text_input_subscribe_state = Rc::clone(&state);
        let text_input_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let callback: Closure = stack.consume(ctx)?;
            let mut state = text_input_subscribe_state.borrow_mut();
            if state.text_input_callbacks.len() >= 64 {
                return Err(HostError("text input callback limit reached".into()).into());
            }
            state.text_input_callbacks.push(ctx.stash(callback));
            state.text_input_enable_requested = true;
            Ok(CallbackReturn::Return)
        });
        let text_input_disable_state = Rc::clone(&state);
        let text_input_disable = Callback::from_fn(&ctx, move |_, _, _| {
            let mut state = text_input_disable_state.borrow_mut();
            if state.text_input_requests.len() >= 256 {
                return Err(HostError("text input request limit reached".into()).into());
            }
            state.text_input_requests.push(TextInputRequest::Disable);
            Ok(CallbackReturn::Return)
        });
        let text_input_surrounding_state = Rc::clone(&state);
        let text_input_surrounding = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (text, cursor, anchor): (String, i64, i64) = stack.consume(ctx)?;
            if text.len() > 4_000 {
                return Err(HostError("text input text limit reached".into()).into());
            }
            let cursor = i32::try_from(cursor)
                .map_err(|_| HostError("text input cursor must fit i32".into()))?;
            let anchor = i32::try_from(anchor)
                .map_err(|_| HostError("text input anchor must fit i32".into()))?;
            let mut state = text_input_surrounding_state.borrow_mut();
            if state.text_input_requests.len() >= 256 {
                return Err(HostError("text input request limit reached".into()).into());
            }
            state
                .text_input_requests
                .push(TextInputRequest::Surrounding {
                    text,
                    cursor,
                    anchor,
                });
            Ok(CallbackReturn::Return)
        });
        let text_input_content_state = Rc::clone(&state);
        let text_input_content = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (hints, purpose): (i64, i64) = stack.consume(ctx)?;
            let hints = u32::try_from(hints)
                .map_err(|_| HostError("text input hints must fit u32".into()))?;
            let purpose = u32::try_from(purpose)
                .map_err(|_| HostError("text input purpose must fit u32".into()))?;
            let mut state = text_input_content_state.borrow_mut();
            if state.text_input_requests.len() >= 256 {
                return Err(HostError("text input request limit reached".into()).into());
            }
            state
                .text_input_requests
                .push(TextInputRequest::ContentType { hints, purpose });
            Ok(CallbackReturn::Return)
        });
        let text_input_rect_state = Rc::clone(&state);
        let text_input_rect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let values: (i64, i64, i64, i64) = stack.consume(ctx)?;
            let request = TextInputRequest::CursorRect {
                x: i32::try_from(values.0)
                    .map_err(|_| HostError("cursor x must fit i32".into()))?,
                y: i32::try_from(values.1)
                    .map_err(|_| HostError("cursor y must fit i32".into()))?,
                width: i32::try_from(values.2)
                    .map_err(|_| HostError("cursor width must fit i32".into()))?,
                height: i32::try_from(values.3)
                    .map_err(|_| HostError("cursor height must fit i32".into()))?,
            };
            let mut state = text_input_rect_state.borrow_mut();
            if state.text_input_requests.len() >= 256 {
                return Err(HostError("text input request limit reached".into()).into());
            }
            state.text_input_requests.push(request);
            Ok(CallbackReturn::Return)
        });
        let text_input = Table::new(&ctx);
        text_input.set_field(ctx, "subscribe", text_input_subscribe);
        text_input.set_field(ctx, "disable", text_input_disable);
        text_input.set_field(ctx, "surrounding", text_input_surrounding);
        text_input.set_field(ctx, "content_type", text_input_content);
        text_input.set_field(ctx, "cursor_rect", text_input_rect);
        mold.set_field(ctx, "text_input", text_input);
        let timer_state = Rc::clone(&state);
        let timer = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (milliseconds, callback, repeat): (f64, Closure, LuaValue) = stack.consume(ctx)?;
            if !milliseconds.is_finite() || milliseconds <= 0.0 {
                return Err(HostError("timer interval must be finite and positive".into()).into());
            }
            let repeat = match repeat {
                LuaValue::Nil => true,
                LuaValue::Boolean(value) => value,
                _ => return Err(HostError("timer repeat must be boolean".into()).into()),
            };
            let timer = IoTimer::every(Duration::from_secs_f64(milliseconds / 1_000.0))
                .map_err(|error| HostError(error.to_string()))?;
            let interval = Duration::from_secs_f64(milliseconds / 1_000.0);
            timer_state.borrow_mut().timers.push(PendingTimer {
                timer,
                callback: ctx.stash(callback),
                repeat,
                interval,
                node: None,
            });
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "timer", timer);
        let ipc_register = Callback::from_fn(&ctx, {
            let state = Rc::clone(&state);
            move |ctx, _, mut stack| {
                let (_table, name, value): (Table, String, LuaValue) = stack.consume(ctx)?;
                match value {
                    LuaValue::Function(Function::Closure(closure)) => {
                        state
                            .borrow_mut()
                            .ipc_handlers
                            .insert(name, ctx.stash(closure));
                    }
                    LuaValue::Nil => {
                        state.borrow_mut().ipc_handlers.remove(&name);
                    }
                    _ => {
                        return Err(HostError(
                            "mold.ipc values must be functions or nil".to_owned(),
                        )
                        .into());
                    }
                }
                Ok(CallbackReturn::Return)
            }
        });
        let ipc_metatable = Table::new(&ctx);
        ipc_metatable.set_field(ctx, "__newindex", ipc_register);
        let ipc = Table::new(&ctx);
        ipc.set_metatable(ctx, Some(ipc_metatable));
        mold.set_field(ctx, "ipc", ipc);
        let screens = Table::new(&ctx);
        if let Some(screen) = screen {
            let value = Table::new(&ctx);
            value.set_field(ctx, "name", screen.name.as_str());
            value.set_field(
                ctx,
                "width",
                screen
                    .width
                    .map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
            );
            value.set_field(
                ctx,
                "height",
                screen
                    .height
                    .map_or(LuaValue::Nil, |value| LuaValue::Integer(value as i64)),
            );
            value.set_field(ctx, "scale", screen.scale as i64);
            screens
                .set(ctx, 1, value)
                .expect("screen table accepts integer keys");
        }
        mold.set_field(ctx, "screens", screens);
        let variants = execute_module(
            ctx,
            "mold.variants",
            b"return function(items, factory) return factory(items[1]) end",
            limits,
        )
        .expect("embedded variants module is valid");
        mold.set_field(ctx, "variants", variants);

        let model_len = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let model: UserRef<ListModelToken> = stack.consume(ctx)?;
            stack.replace(ctx, model.model.borrow().len() as i64);
            Ok(CallbackReturn::Return)
        });
        let model_get = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (model, index): (UserRef<ListModelToken>, i64) = stack.consume(ctx)?;
            let index = lua_index(index)?;
            let model = model.model.borrow();
            let value = model
                .get(index)
                .map(|(_, value)| scene_to_lua(ctx, value))
                .transpose()
                .map_err(HostError)?
                .unwrap_or(LuaValue::Nil);
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        });
        let model_insert = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (model, index, value): (UserRef<ListModelToken>, i64, LuaValue) =
                stack.consume(ctx)?;
            let index = lua_insert_index(index, model.model.borrow().len())?;
            let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
            if model.model.borrow_mut().insert(index, value).is_none() {
                return Err(HostError("list-model insert index is out of range".into()).into());
            }
            Ok(CallbackReturn::Return)
        });
        let model_remove = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (model, index): (UserRef<ListModelToken>, i64) = stack.consume(ctx)?;
            let index = lua_index(index)?;
            let value = model
                .model
                .borrow_mut()
                .remove(index)
                .map(|value| scene_to_lua(ctx, &value))
                .transpose()
                .map_err(HostError)?
                .unwrap_or(LuaValue::Nil);
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        });
        let model_move = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (model, from, to): (UserRef<ListModelToken>, i64, i64) = stack.consume(ctx)?;
            let from = lua_index(from)?;
            let to = lua_index(to)?;
            if !model.model.borrow_mut().move_item(from, to) {
                return Err(HostError("list-model move index is out of range".into()).into());
            }
            Ok(CallbackReturn::Return)
        });
        let model_set = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (model, index, value): (UserRef<ListModelToken>, i64, LuaValue) =
                stack.consume(ctx)?;
            let index = lua_index(index)?;
            let value = lua_to_scene(ctx, value, 0).map_err(HostError)?;
            if !model.model.borrow_mut().set(index, value) {
                return Err(HostError("list-model update index is out of range".into()).into());
            }
            Ok(CallbackReturn::Return)
        });
        let model_methods = Table::new(&ctx);
        model_methods.set_field(ctx, "len", model_len);
        model_methods.set_field(ctx, "get", model_get);
        model_methods.set_field(ctx, "insert", model_insert);
        model_methods.set_field(ctx, "remove", model_remove);
        model_methods.set_field(ctx, "move", model_move);
        model_methods.set_field(ctx, "set", model_set);
        let model_metatable = Table::new(&ctx);
        model_metatable.set_field(ctx, "__index", model_methods);
        let model_metatable = ctx.stash(model_metatable);
        let list_model = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let items: Table = stack.consume(ctx)?;
            let value = lua_to_scene(ctx, LuaValue::Table(items), 0).map_err(HostError)?;
            let SceneValue::List(values) = value else {
                return Err(HostError("list-model needs an array table".into()).into());
            };
            let userdata = UserData::new_static(
                &ctx,
                ListModelToken {
                    model: Rc::new(RefCell::new(ListModel::new(values))),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&model_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "list_model", list_model);

        let virtual_visible = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let view: UserRef<VirtualListToken> = stack.consume(ctx)?;
            let model = view.model.borrow();
            let range = view.view.borrow().visible_range(model.len());
            let items = Table::new(&ctx);
            for (position, index) in range.enumerate() {
                let value = Table::new(&ctx);
                value.set_field(ctx, "index", index as i64 + 1);
                if let Some((_, item)) = model.get(index) {
                    value.set_field(ctx, "item", scene_to_lua(ctx, item).map_err(HostError)?);
                }
                items
                    .set(ctx, position as i64 + 1, value)
                    .expect("virtual-list table accepts integer keys");
            }
            stack.replace(ctx, items);
            Ok(CallbackReturn::Return)
        });
        let virtual_offset = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (view, offset): (UserRef<VirtualListToken>, f64) = stack.consume(ctx)?;
            view.view.borrow_mut().set_offset(offset);
            Ok(CallbackReturn::Return)
        });
        let virtual_sync = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let view: UserRef<VirtualListToken> = stack.consume(ctx)?;
            let changes = view.model.borrow_mut().take_changes();
            let transitions = view.view.borrow_mut().sync(&view.model.borrow(), &changes);
            let result = Table::new(&ctx);
            for (index, transition) in transitions.into_iter().enumerate() {
                result
                    .set(
                        ctx,
                        index as i64 + 1,
                        view_transition_to_lua(ctx, transition),
                    )
                    .expect("view-transition table accepts integer keys");
            }
            stack.replace(ctx, result);
            Ok(CallbackReturn::Return)
        });
        let virtual_methods = Table::new(&ctx);
        virtual_methods.set_field(ctx, "visible", virtual_visible);
        virtual_methods.set_field(ctx, "set_offset", virtual_offset);
        virtual_methods.set_field(ctx, "sync", virtual_sync);
        let virtual_metatable = Table::new(&ctx);
        virtual_metatable.set_field(ctx, "__index", virtual_methods);
        let virtual_metatable = ctx.stash(virtual_metatable);
        let virtual_list = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (model, item_extent, viewport_extent, overscan): (
                UserRef<ListModelToken>,
                f64,
                f64,
                i64,
            ) = stack.consume(ctx)?;
            let overscan = usize::try_from(overscan)
                .map_err(|_| HostError("virtual-list overscan cannot be negative".into()))?;
            let view = VirtualList::new(item_extent, viewport_extent, overscan)
                .ok_or_else(|| HostError("invalid virtual-list dimensions".into()))?;
            let userdata = UserData::new_static(
                &ctx,
                VirtualListToken {
                    model: Rc::clone(&model.model),
                    view: RefCell::new(view),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&virtual_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "virtual_list", virtual_list);

        let sync_state = Rc::clone(&state);
        let sync_view = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (node, offset): (UserRef<NodeToken>, f64) = stack.consume(ctx)?;
            let mut view = sync_state
                .borrow_mut()
                .views
                .remove(&node.handle)
                .ok_or_else(|| HostError("node is not a ListView".to_owned()))?;
            let result =
                reconcile_lua_view(&sync_state, ctx, limits, node.handle, offset, &mut view);
            sync_state.borrow_mut().views.insert(node.handle, view);
            let transitions = result.map_err(HostError)?;
            let values = Table::new(&ctx);
            for (index, transition) in transitions.into_iter().enumerate() {
                values
                    .set(
                        ctx,
                        index as i64 + 1,
                        view_transition_to_lua(ctx, transition),
                    )
                    .expect("view-transition table accepts integer keys");
            }
            stack.replace(ctx, values);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "sync_view", sync_view);

        let flick_drag = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (flick, delta): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
            flick.state.borrow_mut().drag_by(delta);
            stack.replace(ctx, flick.state.borrow().offset);
            Ok(CallbackReturn::Return)
        });
        let flick_release = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (flick, velocity): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
            flick.state.borrow_mut().release(velocity);
            Ok(CallbackReturn::Return)
        });
        let flick_tick = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (flick, milliseconds): (UserRef<FlickToken>, f64) = stack.consume(ctx)?;
            if !milliseconds.is_finite() || milliseconds < 0.0 {
                return Err(HostError("flick delta must be finite and non-negative".into()).into());
            }
            let active = flick
                .state
                .borrow_mut()
                .tick(Duration::from_secs_f64(milliseconds / 1_000.0));
            stack.replace(ctx, (flick.state.borrow().offset, active));
            Ok(CallbackReturn::Return)
        });
        let flick_position = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let flick: UserRef<FlickToken> = stack.consume(ctx)?;
            stack.replace(ctx, flick.state.borrow().offset);
            Ok(CallbackReturn::Return)
        });
        let flick_methods = Table::new(&ctx);
        flick_methods.set_field(ctx, "drag_by", flick_drag);
        flick_methods.set_field(ctx, "release", flick_release);
        flick_methods.set_field(ctx, "tick", flick_tick);
        flick_methods.set_field(ctx, "position", flick_position);
        let flick_metatable = Table::new(&ctx);
        flick_metatable.set_field(ctx, "__index", flick_methods);
        let flick_metatable = ctx.stash(flick_metatable);
        let flickable = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let offset = table_number(ctx, options, "offset", 0.0).map_err(HostError)?;
            let minimum = table_number(ctx, options, "minimum", 0.0).map_err(HostError)?;
            let maximum = table_number(ctx, options, "maximum", 0.0).map_err(HostError)?;
            let deceleration =
                table_number(ctx, options, "deceleration", 2_500.0).map_err(HostError)?;
            if !offset.is_finite()
                || !minimum.is_finite()
                || !maximum.is_finite()
                || !deceleration.is_finite()
                || minimum > maximum
                || deceleration < 0.0
            {
                return Err(HostError("invalid flickable state".into()).into());
            }
            let userdata = UserData::new_static(
                &ctx,
                FlickToken {
                    state: RefCell::new(FlickState {
                        offset: offset.clamp(minimum, maximum),
                        velocity: 0.0,
                        minimum,
                        maximum,
                        deceleration,
                    }),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&flick_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "flickable", flickable);

        let transition_state = Rc::clone(&state);
        let transition_parent = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (node, parent, options): (UserRef<NodeToken>, UserRef<NodeToken>, Table) =
                stack.consume(ctx)?;
            let duration = table_number(ctx, options, "duration", 250.0).map_err(HostError)?;
            if duration < 0.0 {
                return Err(
                    HostError("parent-transition duration cannot be negative".into()).into(),
                );
            }
            let easing = parse_easing(options.get_value(ctx, "easing")).map_err(HostError)?;
            let anchors = match options.get_value(ctx, "anchors") {
                LuaValue::Nil => None,
                value => match lua_to_scene(ctx, value, 0).map_err(HostError)? {
                    SceneValue::Map(anchors) => Some(anchors),
                    _ => return Err(HostError("transition anchors must be a table".into()).into()),
                },
            };
            transition_state
                .borrow_mut()
                .parent_transitions
                .push(ParentTransitionRequest {
                    node: node.handle,
                    parent: parent.handle,
                    anchors,
                    behavior: Behavior {
                        duration: Duration::from_secs_f64(duration / 1_000.0),
                        easing,
                    },
                });
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "transition_parent", transition_parent);

        let process_write = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (process, bytes): (UserRef<ProcessToken>, String) = stack.consume(ctx)?;
            process
                .process
                .borrow_mut()
                .write(bytes.as_bytes())
                .map_err(|error| HostError(error.to_string()))?;
            Ok(CallbackReturn::Return)
        });
        let process_close = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let process: UserRef<ProcessToken> = stack.consume(ctx)?;
            process.process.borrow_mut().close_stdin();
            Ok(CallbackReturn::Return)
        });
        let process_kill = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let process: UserRef<ProcessToken> = stack.consume(ctx)?;
            process
                .process
                .borrow_mut()
                .kill()
                .map_err(|error| HostError(error.to_string()))?;
            Ok(CallbackReturn::Return)
        });
        let process_next = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let process: UserRef<ProcessToken> = stack.consume(ctx)?;
            let event = process
                .process
                .borrow_mut()
                .next_event(Duration::ZERO)
                .map_err(|error| HostError(error.to_string()))?;
            let Some(event) = event else {
                stack.replace(ctx, LuaValue::Nil);
                return Ok(CallbackReturn::Return);
            };
            let value = Table::new(&ctx);
            match event {
                ProcessEvent::Stdout(bytes) => {
                    value.set_field(ctx, "kind", "stdout");
                    value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
                }
                ProcessEvent::Stderr(bytes) => {
                    value.set_field(ctx, "kind", "stderr");
                    value.set_field(ctx, "data", String::from_utf8_lossy(&bytes).as_ref());
                }
                ProcessEvent::Exit(status) => {
                    value.set_field(ctx, "kind", "exit");
                    value.set_field(ctx, "success", status.success());
                    value.set_field(
                        ctx,
                        "code",
                        status
                            .code()
                            .map_or(LuaValue::Nil, |code| LuaValue::Integer(code as i64)),
                    );
                }
            }
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        });
        let process_methods = Table::new(&ctx);
        process_methods.set_field(ctx, "write", process_write);
        process_methods.set_field(ctx, "close_stdin", process_close);
        process_methods.set_field(ctx, "kill", process_kill);
        process_methods.set_field(ctx, "next", process_next);
        let process_metatable = Table::new(&ctx);
        process_metatable.set_field(ctx, "__index", process_methods);
        let process_metatable = ctx.stash(process_metatable);
        let process = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (program, args): (String, Table) = stack.consume(ctx)?;
            let args = table_string_array(ctx, args, 64).map_err(HostError)?;
            let process =
                Process::spawn(program, args).map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(
                &ctx,
                ProcessToken {
                    process: RefCell::new(process),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&process_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "process", process);

        let file_read = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let file: UserRef<FileToken> = stack.consume(ctx)?;
            let bytes = file
                .file
                .read_bounded(1024 * 1024)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, String::from_utf8_lossy(&bytes).as_ref());
            Ok(CallbackReturn::Return)
        });
        let file_write = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (file, bytes): (UserRef<FileToken>, String) = stack.consume(ctx)?;
            if bytes.len() > 1024 * 1024 {
                return Err(HostError("file write exceeds 1 MiB".to_owned()).into());
            }
            file.file
                .write(bytes.as_bytes())
                .map_err(|error| HostError(error.to_string()))?;
            Ok(CallbackReturn::Return)
        });
        let watcher_next = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (watcher, timeout_ms): (UserRef<FileWatcherToken>, i64) = stack.consume(ctx)?;
            let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
            let event = watcher.watcher.next_event(timeout);
            match event {
                Some(FileEvent::Changed) => stack.replace(ctx, "changed"),
                Some(FileEvent::Moved) => stack.replace(ctx, "moved"),
                Some(FileEvent::Deleted) => stack.replace(ctx, "deleted"),
                None => stack.replace(ctx, LuaValue::Nil),
            }
            Ok(CallbackReturn::Return)
        });
        let watcher_methods = Table::new(&ctx);
        watcher_methods.set_field(ctx, "next", watcher_next);
        let watcher_metatable = Table::new(&ctx);
        watcher_metatable.set_field(ctx, "__index", watcher_methods);
        let watcher_metatable = ctx.stash(watcher_metatable);
        let file_watch = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let file: UserRef<FileToken> = stack.consume(ctx)?;
            let watcher = file
                .file
                .watch()
                .map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(&ctx, FileWatcherToken { watcher });
            userdata.set_metatable(ctx, Some(ctx.fetch(&watcher_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        let file_methods = Table::new(&ctx);
        file_methods.set_field(ctx, "read", file_read);
        file_methods.set_field(ctx, "write", file_write);
        file_methods.set_field(ctx, "watch", file_watch);
        let file_metatable = Table::new(&ctx);
        file_metatable.set_field(ctx, "__index", file_methods);
        let file_metatable = ctx.stash(file_metatable);
        let file_view = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let path: String = stack.consume(ctx)?;
            let userdata = UserData::new_static(
                &ctx,
                FileToken {
                    file: FileView::new(path),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&file_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "file", file_view);

        let socket_send = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (socket, bytes): (UserRef<SocketToken>, String) = stack.consume(ctx)?;
            if bytes.len() > 64 * 1024 {
                return Err(HostError("socket send exceeds 64 KiB".to_owned()).into());
            }
            socket
                .socket
                .borrow_mut()
                .send(bytes.as_bytes())
                .map_err(|error| HostError(error.to_string()))?;
            Ok(CallbackReturn::Return)
        });
        let socket_receive = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (socket, maximum, timeout_ms): (UserRef<SocketToken>, i64, i64) =
                stack.consume(ctx)?;
            let maximum = usize::try_from(maximum)
                .ok()
                .filter(|maximum| (1..=64 * 1024).contains(maximum))
                .ok_or_else(|| HostError("socket receive limit must be 1..65536".to_owned()))?;
            let timeout = bounded_timeout(timeout_ms).map_err(HostError)?;
            let mut bytes = vec![0; maximum];
            match socket
                .socket
                .borrow_mut()
                .receive_timeout(&mut bytes, timeout)
            {
                Ok(read) => {
                    bytes.truncate(read);
                    stack.replace(ctx, String::from_utf8_lossy(&bytes).as_ref());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    stack.replace(ctx, LuaValue::Nil);
                }
                Err(error) => return Err(HostError(error.to_string()).into()),
            }
            Ok(CallbackReturn::Return)
        });
        let socket_methods = Table::new(&ctx);
        socket_methods.set_field(ctx, "send", socket_send);
        socket_methods.set_field(ctx, "receive", socket_receive);
        let socket_metatable = Table::new(&ctx);
        socket_metatable.set_field(ctx, "__index", socket_methods);
        let socket_metatable = ctx.stash(socket_metatable);
        let accepted_socket_metatable = socket_metatable.clone();
        let server_accept = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let server: UserRef<SocketServerToken> = stack.consume(ctx)?;
            let Some(socket) = server
                .server
                .try_accept()
                .map_err(|error| HostError(error.to_string()))?
            else {
                stack.replace(ctx, LuaValue::Nil);
                return Ok(CallbackReturn::Return);
            };
            let userdata = UserData::new_static(
                &ctx,
                SocketToken {
                    socket: RefCell::new(socket),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&accepted_socket_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        let server_methods = Table::new(&ctx);
        server_methods.set_field(ctx, "accept", server_accept);
        let server_metatable = Table::new(&ctx);
        server_metatable.set_field(ctx, "__index", server_methods);
        let server_metatable = ctx.stash(server_metatable);
        let socket_server = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let path: String = stack.consume(ctx)?;
            let server = SocketServer::bind(path).map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(&ctx, SocketServerToken { server });
            userdata.set_metatable(ctx, Some(ctx.fetch(&server_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "socket_server", socket_server);
        let socket = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let path: String = stack.consume(ctx)?;
            let socket = Socket::connect(path).map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(
                &ctx,
                SocketToken {
                    socket: RefCell::new(socket),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&socket_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "socket", socket);

        let line_push = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (parser, chunk): (UserRef<LineParserToken>, String) = stack.consume(ctx)?;
            let values = parser.parser.borrow_mut().push(chunk.as_bytes());
            stack.replace(ctx, string_table(ctx, values));
            Ok(CallbackReturn::Return)
        });
        let line_finish = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let parser: UserRef<LineParserToken> = stack.consume(ctx)?;
            match parser.parser.borrow_mut().finish() {
                Some(value) => stack.replace(ctx, value),
                None => stack.replace(ctx, LuaValue::Nil),
            }
            Ok(CallbackReturn::Return)
        });
        let line_methods = Table::new(&ctx);
        line_methods.set_field(ctx, "push", line_push);
        line_methods.set_field(ctx, "finish", line_finish);
        let line_metatable = Table::new(&ctx);
        line_metatable.set_field(ctx, "__index", line_methods);
        let line_metatable = ctx.stash(line_metatable);
        let line_parser = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let userdata = UserData::new_static(
                &ctx,
                LineParserToken {
                    parser: RefCell::new(LineParser::default()),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&line_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "line_parser", line_parser);

        let split_push = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (parser, chunk): (UserRef<SplitParserToken>, String) = stack.consume(ctx)?;
            let values = parser
                .parser
                .borrow_mut()
                .push(chunk.as_bytes())
                .into_iter()
                .map(|value| String::from_utf8_lossy(&value).into_owned());
            stack.replace(ctx, string_table(ctx, values));
            Ok(CallbackReturn::Return)
        });
        let split_finish = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let parser: UserRef<SplitParserToken> = stack.consume(ctx)?;
            match parser.parser.borrow_mut().finish() {
                Some(value) => stack.replace(ctx, String::from_utf8_lossy(&value).as_ref()),
                None => stack.replace(ctx, LuaValue::Nil),
            }
            Ok(CallbackReturn::Return)
        });
        let split_methods = Table::new(&ctx);
        split_methods.set_field(ctx, "push", split_push);
        split_methods.set_field(ctx, "finish", split_finish);
        let split_metatable = Table::new(&ctx);
        split_metatable.set_field(ctx, "__index", split_methods);
        let split_metatable = ctx.stash(split_metatable);
        let split_parser = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let delimiter: String = stack.consume(ctx)?;
            let parser = SplitParser::new(delimiter.into_bytes())
                .map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(
                &ctx,
                SplitParserToken {
                    parser: RefCell::new(parser),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&split_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        mold.set_field(ctx, "split_parser", split_parser);

        let dbus_get = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, property): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
            let value = proxy.proxy.get_value(&property).map_err(HostError)?;
            stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
            Ok(CallbackReturn::Return)
        });
        let dbus_call = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, method): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
            let value = proxy.proxy.call_value(&method).map_err(HostError)?;
            stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
            Ok(CallbackReturn::Return)
        });
        let dbus_call_with = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, method, argument): (UserRef<DbusToken>, String, LuaValue) =
                stack.consume(ctx)?;
            let argument = lua_to_dbus(ctx, argument, 0).map_err(HostError)?;
            let value = proxy
                .proxy
                .call_value_with(&method, &argument)
                .map_err(HostError)?;
            stack.replace(ctx, dbus_value_to_lua(ctx, value).map_err(HostError)?);
            Ok(CallbackReturn::Return)
        });
        let dbus_set = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, property, value): (UserRef<DbusToken>, String, LuaValue) =
                stack.consume(ctx)?;
            let value = lua_to_dbus(ctx, value, 0).map_err(HostError)?;
            proxy
                .proxy
                .set_value(&property, &value)
                .map_err(HostError)?;
            Ok(CallbackReturn::Return)
        });
        let dbus_signal_state = Rc::clone(&state);
        let dbus_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (proxy, signal, callback): (UserRef<DbusToken>, String, Closure) =
                stack.consume(ctx)?;
            let signal = proxy
                .proxy
                .subscribe(signal)
                .map_err(|error| HostError(error.to_string()))?;
            dbus_signal_state
                .borrow_mut()
                .dbus_signals
                .push(PendingDbusSignal {
                    signal,
                    callback: ctx.stash(callback),
                });
            Ok(CallbackReturn::Return)
        });
        let dbus_introspect = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let proxy: UserRef<DbusToken> = stack.consume(ctx)?;
            let xml = proxy
                .proxy
                .introspect()
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, xml);
            Ok(CallbackReturn::Return)
        });
        let dbus_methods = Table::new(&ctx);
        dbus_methods.set_field(ctx, "get", dbus_get);
        dbus_methods.set_field(ctx, "call", dbus_call);
        dbus_methods.set_field(ctx, "call_with", dbus_call_with);
        dbus_methods.set_field(ctx, "set", dbus_set);
        dbus_methods.set_field(ctx, "subscribe", dbus_subscribe);
        dbus_methods.set_field(ctx, "introspect", dbus_introspect);
        let dbus_metatable = Table::new(&ctx);
        dbus_metatable.set_field(ctx, "__index", dbus_methods);
        let dbus_metatable = ctx.stash(dbus_metatable);
        let dbus_proxy = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (bus, destination, path, interface): (String, String, String, String) =
                stack.consume(ctx)?;
            let bus = match bus.as_str() {
                "session" => Bus::Session,
                "system" => Bus::System,
                _ => return Err(HostError(format!("unknown D-Bus bus `{bus}`")).into()),
            };
            let proxy = DbusProxy::connect(bus, destination, path, interface)
                .map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(&ctx, DbusToken { proxy });
            userdata.set_metatable(ctx, Some(ctx.fetch(&dbus_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        let dbus = Table::new(&ctx);
        dbus.set_field(ctx, "proxy", dbus_proxy);
        mold.set_field(ctx, "dbus", dbus);

        let pipewire_nodes = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let pipewire: UserRef<PipeWireToken> = stack.consume(ctx)?;
            let nodes = Table::new(&ctx);
            for (index, node) in pipewire.service.nodes().into_iter().enumerate() {
                let value = Table::new(&ctx);
                value.set_field(ctx, "id", node.id as i64);
                value.set_field(
                    ctx,
                    "serial",
                    node.serial
                        .and_then(|value| i64::try_from(value).ok())
                        .map_or(LuaValue::Nil, LuaValue::Integer),
                );
                value.set_field(ctx, "name", node.name.as_str());
                value.set_field(ctx, "description", node.description.as_str());
                value.set_field(ctx, "media_class", node.media_class.as_str());
                nodes
                    .set(ctx, index as i64 + 1, value)
                    .expect("PipeWire node table accepts integer keys");
            }
            stack.replace(ctx, nodes);
            Ok(CallbackReturn::Return)
        });
        let pipewire_volume = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (pipewire, id): (UserRef<PipeWireToken>, i64) = stack.consume(ctx)?;
            let id = u32::try_from(id).map_err(|_| HostError("invalid PipeWire node id".into()))?;
            let volume = pipewire
                .service
                .volume(id)
                .map_err(|error| HostError(error.to_string()))?;
            let value = Table::new(&ctx);
            let channels = Table::new(&ctx);
            for (index, channel) in volume.channels.iter().enumerate() {
                channels
                    .set(ctx, index as i64 + 1, *channel as f64)
                    .expect("PipeWire channel table accepts integer keys");
            }
            value.set_field(ctx, "channels", channels);
            value.set_field(ctx, "level", volume.average() as f64);
            value.set_field(ctx, "muted", volume.muted);
            stack.replace(ctx, value);
            Ok(CallbackReturn::Return)
        });
        let pipewire_set_volume = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (pipewire, id, level, muted): (UserRef<PipeWireToken>, i64, f64, bool) =
                stack.consume(ctx)?;
            let id = u32::try_from(id).map_err(|_| HostError("invalid PipeWire node id".into()))?;
            if !level.is_finite() || level < 0.0 || level > f32::MAX as f64 {
                return Err(
                    HostError("PipeWire volume must be finite and non-negative".into()).into(),
                );
            }
            let current = pipewire
                .service
                .volume(id)
                .map_err(|error| HostError(error.to_string()))?;
            let channels = vec![level as f32; current.channels.len().max(1)];
            pipewire
                .service
                .set_volume(id, &channels, muted)
                .map_err(|error| HostError(error.to_string()))?;
            Ok(CallbackReturn::Return)
        });
        let pipewire_methods = Table::new(&ctx);
        pipewire_methods.set_field(ctx, "nodes", pipewire_nodes);
        pipewire_methods.set_field(ctx, "volume", pipewire_volume);
        pipewire_methods.set_field(ctx, "set_volume", pipewire_set_volume);
        let pipewire_metatable = Table::new(&ctx);
        pipewire_metatable.set_field(ctx, "__index", pipewire_methods);
        let pipewire_metatable = ctx.stash(pipewire_metatable);
        let pipewire_connect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let service = PipeWire::connect().map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(&ctx, PipeWireToken { service });
            userdata.set_metatable(ctx, Some(ctx.fetch(&pipewire_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        let pipewire = Table::new(&ctx);
        pipewire.set_field(ctx, "connect", pipewire_connect);
        mold.set_field(ctx, "pipewire", pipewire);

        let udev_state = Rc::clone(&state);
        let udev_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (subsystem, callback): (Option<String>, Closure) = stack.consume(ctx)?;
            let monitor =
                UdevMonitor::new(subsystem).map_err(|error| HostError(error.to_string()))?;
            udev_state.borrow_mut().udev_monitors.push(PendingUdev {
                monitor,
                callback: ctx.stash(callback),
            });
            Ok(CallbackReturn::Return)
        });
        let udev = Table::new(&ctx);
        udev.set_field(ctx, "subscribe", udev_subscribe);
        mold.set_field(ctx, "udev", udev);

        let status_notifier_state = Rc::clone(&state);
        let status_notifier_subscribe = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let callback: Closure = stack.consume(ctx)?;
            let host =
                StatusNotifierHost::connect().map_err(|error| HostError(error.to_string()))?;
            let mut state = status_notifier_state.borrow_mut();
            if state.status_notifiers.len() >= 4 {
                return Err(HostError("status notifier subscription limit reached".into()).into());
            }
            state.status_notifiers.push(PendingStatusNotifier {
                host,
                callback: ctx.stash(callback),
            });
            Ok(CallbackReturn::Return)
        });
        let status_notifier = Table::new(&ctx);
        status_notifier.set_field(ctx, "subscribe", status_notifier_subscribe);
        mold.set_field(ctx, "status_notifier", status_notifier);

        let greetd_create = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (greetd, username): (UserRef<GreetdToken>, String) = stack.consume(ctx)?;
            let response = greetd
                .client
                .borrow_mut()
                .create_session(&username)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, greetd_response(ctx, response));
            Ok(CallbackReturn::Return)
        });
        let greetd_respond = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (greetd, response): (UserRef<GreetdToken>, Option<String>) = stack.consume(ctx)?;
            let response = greetd
                .client
                .borrow_mut()
                .respond(response.as_deref())
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, greetd_response(ctx, response));
            Ok(CallbackReturn::Return)
        });
        let greetd_start = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (greetd, command, environment): (UserRef<GreetdToken>, Table, Table) =
                stack.consume(ctx)?;
            let command = table_string_array(ctx, command, 64).map_err(HostError)?;
            let environment = table_string_array(ctx, environment, 256).map_err(HostError)?;
            let response = greetd
                .client
                .borrow_mut()
                .start_session(&command, &environment)
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, greetd_response(ctx, response));
            Ok(CallbackReturn::Return)
        });
        let greetd_cancel = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let greetd: UserRef<GreetdToken> = stack.consume(ctx)?;
            let response = greetd
                .client
                .borrow_mut()
                .cancel_session()
                .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, greetd_response(ctx, response));
            Ok(CallbackReturn::Return)
        });
        let greetd_methods = Table::new(&ctx);
        greetd_methods.set_field(ctx, "create_session", greetd_create);
        greetd_methods.set_field(ctx, "respond", greetd_respond);
        greetd_methods.set_field(ctx, "start_session", greetd_start);
        greetd_methods.set_field(ctx, "cancel_session", greetd_cancel);
        let greetd_metatable = Table::new(&ctx);
        greetd_metatable.set_field(ctx, "__index", greetd_methods);
        let greetd_metatable = ctx.stash(greetd_metatable);
        let greetd_connect = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let path: Option<String> = stack.consume(ctx)?;
            let timeout = Duration::from_secs(2);
            let client = match path {
                Some(path) => GreetdClient::connect(path, timeout),
                None => GreetdClient::connect_environment(timeout),
            }
            .map_err(|error| HostError(error.to_string()))?;
            let userdata = UserData::new_static(
                &ctx,
                GreetdToken {
                    client: RefCell::new(client),
                },
            );
            userdata.set_metatable(ctx, Some(ctx.fetch(&greetd_metatable)));
            stack.replace(ctx, userdata);
            Ok(CallbackReturn::Return)
        });
        let greetd = Table::new(&ctx);
        greetd.set_field(ctx, "connect", greetd_connect);
        mold.set_field(ctx, "greetd", greetd);

        let pam_state = Rc::clone(&state);
        let pam_authenticate = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (service, username, password, callback): (String, String, String, Closure) =
                stack.consume(ctx)?;
            pam_state.borrow_mut().pam_tasks.push(PendingPam {
                task: PamAuthenticator::authenticate_async(service, username, password),
                callback: ctx.stash(callback),
                unlock_on_success: false,
            });
            Ok(CallbackReturn::Return)
        });
        let pam = Table::new(&ctx);
        pam.set_field(ctx, "authenticate", pam_authenticate);
        let pam_unlock_state = Rc::clone(&state);
        let pam_authenticate_unlock = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let (service, username, password, callback): (String, String, String, Closure) =
                stack.consume(ctx)?;
            pam_unlock_state.borrow_mut().pam_tasks.push(PendingPam {
                task: PamAuthenticator::authenticate_async(service, username, password),
                callback: ctx.stash(callback),
                unlock_on_success: true,
            });
            Ok(CallbackReturn::Return)
        });
        pam.set_field(ctx, "authenticate_unlock", pam_authenticate_unlock);
        mold.set_field(ctx, "pam", pam);

        let xkb_compile = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
            let options: Table = stack.consume(ctx)?;
            let rules = table_string(ctx, options, "rules", "").map_err(HostError)?;
            let model = table_string(ctx, options, "model", "pc105").map_err(HostError)?;
            let layout = table_string(ctx, options, "layout", "us").map_err(HostError)?;
            let variant = table_string(ctx, options, "variant", "").map_err(HostError)?;
            let xkb_options = match options.get_value(ctx, "options") {
                LuaValue::Nil => None,
                LuaValue::String(value) => Some(value.display_lossy().to_string()),
                _ => return Err(HostError("XKB options must be a string".into()).into()),
            };
            let keymap =
                XkbKeymap::compile(&rules, &model, &layout, &variant, xkb_options.as_deref())
                    .map_err(|error| HostError(error.to_string()))?;
            stack.replace(ctx, xkb_keymap_to_lua(ctx, &keymap));
            Ok(CallbackReturn::Return)
        });
        let xkb = Table::new(&ctx);
        xkb.set_field(ctx, "compile", xkb_compile);
        mold.set_field(ctx, "xkb", xkb);

        let ui = Table::new(&ctx);
        for (name, element) in [
            ("Item", Element::Item),
            ("Rect", Element::Rect),
            ("Text", Element::Text),
            ("Image", Element::Image),
            ("Icon", Element::Icon),
            ("Shape", Element::Shape),
            ("MouseArea", Element::MouseArea),
            ("Row", Element::Row),
            ("Column", Element::Column),
            ("Grid", Element::Grid),
            ("RowLayout", Element::RowLayout),
            ("ColumnLayout", Element::ColumnLayout),
            ("GridLayout", Element::GridLayout),
        ] {
            ui.set_field(
                ctx,
                name,
                element_constructor(ctx, Rc::clone(&state), limits, element),
            );
        }
        ui.set_field(
            ctx,
            "Repeater",
            view_constructor(ctx, Rc::clone(&state), limits, ViewKind::Repeater),
        );
        ui.set_field(
            ctx,
            "ListView",
            view_constructor(ctx, Rc::clone(&state), limits, ViewKind::List),
        );
        ui.set_field(
            ctx,
            "GridView",
            view_constructor(ctx, Rc::clone(&state), limits, ViewKind::Grid),
        );
        ui.set_field(
            ctx,
            "Flickable",
            element_constructor(ctx, Rc::clone(&state), limits, Element::Flickable),
        );
        ui.set_field(
            ctx,
            "Loader",
            loader_constructor(ctx, Rc::clone(&state), limits),
        );
        ui.set_field(
            ctx,
            "Timer",
            timer_constructor(ctx, Rc::clone(&state), limits),
        );
        let component = execute_module(
            ctx,
            "mold.component",
            include_bytes!("../../../runtime/lua/mold/component.lua"),
            limits,
        )
        .expect("engine component module must load");
        ui.set_field(ctx, "component", component);
        for kind in ["spring", "smoothed"] {
            ui.set_field(
                ctx,
                kind,
                Callback::from_fn(&ctx, move |ctx, _, mut stack| {
                    let options: Table = stack.consume(ctx)?;
                    options.set_field(ctx, "kind", kind);
                    stack.replace(ctx, options);
                    Ok(CallbackReturn::Return)
                }),
            );
        }
        mold.set_field(ctx, "ui", ui);
        ctx.set_global("mold", mold);

        let mold = ctx.stash(mold);
        let ui = ctx.stash(ui);
        ctx.set_global(
            "require",
            Callback::from_fn(&ctx, move |ctx, _, mut stack| {
                let name: String = stack.consume(ctx)?;
                match name.as_str() {
                    "mold" => stack.replace(ctx, ctx.fetch(&mold)),
                    "mold.ui" => stack.replace(ctx, ctx.fetch(&ui)),
                    "patin.widgets.button" => {
                        let module = execute_module(
                            ctx,
                            "patin.widgets.button",
                            include_bytes!("../../../runtime/lua/patin/widgets/button.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.services.upower" => {
                        let module = execute_module(
                            ctx,
                            "patin.services.upower",
                            include_bytes!("../../../runtime/lua/patin/services/upower.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.services.network" => {
                        let module = execute_module(
                            ctx,
                            "patin.services.network",
                            include_bytes!("../../../runtime/lua/patin/services/network.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.services.volume" => {
                        let module = execute_module(
                            ctx,
                            "patin.services.volume",
                            include_bytes!("../../../runtime/lua/patin/services/volume.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.indicators.battery" => {
                        let module = execute_module(
                            ctx,
                            "patin.indicators.battery",
                            include_bytes!("../../../runtime/lua/patin/indicators/battery.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.indicators.network" => {
                        let module = execute_module(
                            ctx,
                            "patin.indicators.network",
                            include_bytes!("../../../runtime/lua/patin/indicators/network.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    "patin.indicators.volume" => {
                        let module = execute_module(
                            ctx,
                            "patin.indicators.volume",
                            include_bytes!("../../../runtime/lua/patin/indicators/volume.lua"),
                            limits,
                        )
                        .map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                    _ => {
                        let source = load_runtime_module(&module_roots.borrow(), &name)
                            .map_err(HostError)?;
                        let module =
                            execute_module(ctx, &name, &source, limits).map_err(HostError)?;
                        stack.replace(ctx, module);
                    }
                }
                Ok(CallbackReturn::Return)
            }),
        );
    });
}

fn default_module_roots() -> Vec<PathBuf> {
    let mut roots = std::env::var_os("MOLD_RUNTIME_PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/lua"));
    if let Ok(executable) = std::env::current_exe()
        && let Some(prefix) = executable.parent().and_then(Path::parent)
    {
        roots.push(prefix.join("share/mold/runtime/lua"));
    }
    roots
}

fn load_runtime_module(roots: &[PathBuf], name: &str) -> Result<Vec<u8>, String> {
    if name.is_empty()
        || name.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
    {
        return Err(format!("invalid module name `{name}`"));
    }
    let relative = name.replace('.', "/");
    for root in roots {
        for path in [
            root.join(format!("{relative}.lua")),
            root.join(&relative).join("init.lua"),
            root.join("lua").join(format!("{relative}.lua")),
            root.join("lua").join(&relative).join("init.lua"),
        ] {
            match fs::read(&path) {
                Ok(source) => return Ok(source),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("could not read {}: {error}", path.display())),
            }
        }
    }
    Err(format!("module `{name}` is not available"))
}

fn dbus_value_to_lua(ctx: Context<'_>, value: DbusValue) -> Result<LuaValue<'_>, String> {
    Ok(match value {
        DbusValue::Nil => LuaValue::Nil,
        DbusValue::Bool(value) => LuaValue::Boolean(value),
        DbusValue::Integer(value) => LuaValue::Integer(value),
        DbusValue::Unsigned(value) if value <= i64::MAX as u64 => LuaValue::Integer(value as i64),
        DbusValue::Unsigned(value) => LuaValue::Number(value as f64),
        DbusValue::Number(value) => LuaValue::Number(value),
        DbusValue::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        DbusValue::List(values) => {
            let table = Table::new(&ctx);
            for (index, value) in values.into_iter().enumerate() {
                table
                    .set(ctx, index as i64 + 1, dbus_value_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        DbusValue::Map(values) => {
            let table = Table::new(&ctx);
            for (key, value) in values {
                table
                    .set(
                        ctx,
                        ctx.intern(key.as_bytes()),
                        dbus_value_to_lua(ctx, value)?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        DbusValue::Typed { signature, value } => {
            let table = Table::new(&ctx);
            table.set_field(ctx, "signature", signature.as_str());
            table.set_field(ctx, "value", dbus_value_to_lua(ctx, *value)?);
            LuaValue::Table(table)
        }
    })
}

fn lua_to_dbus<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
    depth: usize,
) -> Result<DbusValue, String> {
    if depth > 8 {
        return Err("D-Bus value exceeds maximum depth 8".to_owned());
    }
    match value {
        LuaValue::Nil => Ok(DbusValue::Nil),
        LuaValue::Boolean(value) => Ok(DbusValue::Bool(value)),
        LuaValue::Integer(value) => Ok(DbusValue::Integer(value)),
        LuaValue::Number(value) if value.is_finite() => Ok(DbusValue::Number(value)),
        LuaValue::String(value) => Ok(DbusValue::String(value.display_lossy().to_string())),
        LuaValue::Table(table) => {
            if let LuaValue::String(signature) = table.get_value(ctx, "signature") {
                let value = table.get_value(ctx, "value");
                return Ok(DbusValue::Typed {
                    signature: signature.display_lossy().to_string(),
                    value: Box::new(lua_to_dbus(ctx, value, depth + 1)?),
                });
            }
            let entries = table.iter(ctx).collect::<Vec<_>>();
            if entries.len() > 256 {
                return Err("D-Bus table exceeds 256 entries".to_owned());
            }
            if entries.is_empty()
                || entries
                    .iter()
                    .all(|(key, _)| matches!(key, LuaValue::Integer(_)))
            {
                let mut values = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let LuaValue::Integer(index) = key else {
                            unreachable!()
                        };
                        Ok((index, lua_to_dbus(ctx, value, depth + 1)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                values.sort_by_key(|(index, _)| *index);
                for (offset, (index, _)) in values.iter().enumerate() {
                    if *index != offset as i64 + 1 {
                        return Err("D-Bus list must be a dense sequence".to_owned());
                    }
                }
                Ok(DbusValue::List(
                    values.into_iter().map(|(_, value)| value).collect(),
                ))
            } else if entries
                .iter()
                .all(|(key, _)| matches!(key, LuaValue::String(_)))
            {
                let mut values = BTreeMap::new();
                for (key, value) in entries {
                    let LuaValue::String(key) = key else {
                        unreachable!()
                    };
                    values.insert(
                        key.display_lossy().to_string(),
                        lua_to_dbus(ctx, value, depth + 1)?,
                    );
                }
                Ok(DbusValue::Map(values))
            } else {
                Err("D-Bus table keys must be all integers or all strings".to_owned())
            }
        }
        _ => Err("unsupported D-Bus value".to_owned()),
    }
}

fn lua_index(index: i64) -> Result<usize, HostError> {
    let index = index
        .checked_sub(1)
        .ok_or_else(|| HostError("list-model indexes start at one".into()))?;
    usize::try_from(index).map_err(|_| HostError("list-model index is out of range".into()))
}

fn lua_insert_index(index: i64, length: usize) -> Result<usize, HostError> {
    if index == length as i64 + 1 {
        Ok(length)
    } else {
        lua_index(index)
    }
}

fn scene_to_lua<'gc>(ctx: Context<'gc>, value: &SceneValue) -> Result<LuaValue<'gc>, String> {
    Ok(match value {
        SceneValue::Nil => LuaValue::Nil,
        SceneValue::Bool(value) => LuaValue::Boolean(*value),
        SceneValue::Number(value) => LuaValue::Number(*value),
        SceneValue::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
        SceneValue::Color(color) => {
            let table = Table::new(&ctx);
            table.set_field(ctx, "r", color.red as f64);
            table.set_field(ctx, "g", color.green as f64);
            table.set_field(ctx, "b", color.blue as f64);
            table.set_field(ctx, "a", color.alpha as f64);
            LuaValue::Table(table)
        }
        SceneValue::List(values) => {
            let table = Table::new(&ctx);
            for (index, value) in values.iter().enumerate() {
                table
                    .set(ctx, index as i64 + 1, scene_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
        SceneValue::Map(values) => {
            let table = Table::new(&ctx);
            for (key, value) in values {
                table
                    .set(ctx, ctx.intern(key.as_bytes()), scene_to_lua(ctx, value)?)
                    .map_err(|error| error.to_string())?;
            }
            LuaValue::Table(table)
        }
    })
}

fn xkb_keymap_to_lua<'gc>(ctx: Context<'gc>, keymap: &XkbKeymap) -> Table<'gc> {
    let result = Table::new(&ctx);
    result.set_field(ctx, "source", keymap.source.as_str());
    let keys = Table::new(&ctx);
    for (key_index, key) in keymap.keys.iter().enumerate() {
        let value = Table::new(&ctx);
        value.set_field(ctx, "keycode", i64::from(key.keycode));
        value.set_field(ctx, "evdev_code", i64::from(key.evdev_code));
        value.set_field(ctx, "name", key.name.as_str());
        value.set_field(ctx, "repeats", key.repeats);
        let layouts = Table::new(&ctx);
        for (layout_index, layout) in key.layouts.iter().enumerate() {
            let levels = Table::new(&ctx);
            for (level_index, level) in layout.iter().enumerate() {
                let symbols = Table::new(&ctx);
                for (symbol_index, symbol) in level.iter().enumerate() {
                    let item = Table::new(&ctx);
                    item.set_field(ctx, "keysym", i64::from(symbol.keysym));
                    item.set_field(ctx, "name", symbol.name.as_str());
                    item.set_field(ctx, "text", symbol.text.as_str());
                    symbols
                        .set(ctx, symbol_index as i64 + 1, item)
                        .expect("XKB symbol table accepts integer keys");
                }
                levels
                    .set(ctx, level_index as i64 + 1, symbols)
                    .expect("XKB level table accepts integer keys");
            }
            layouts
                .set(ctx, layout_index as i64 + 1, levels)
                .expect("XKB layout table accepts integer keys");
        }
        value.set_field(ctx, "layouts", layouts);
        keys.set(ctx, key_index as i64 + 1, value)
            .expect("XKB key table accepts integer keys");
    }
    result.set_field(ctx, "keys", keys);
    result
}

fn view_transition_to_lua(ctx: Context<'_>, transition: ViewTransition) -> Table<'_> {
    let table = Table::new(&ctx);
    let (kind, item, from, targets) = match transition {
        ViewTransition::Populate(item) => ("populate", item, None, Vec::new()),
        ViewTransition::Add(item) => ("add", item, None, Vec::new()),
        ViewTransition::Remove(item) => ("remove", item, None, Vec::new()),
        ViewTransition::Move {
            item,
            from,
            target_indexes,
        } => ("move", item, Some(from), target_indexes),
        ViewTransition::Displaced {
            item,
            from,
            target_indexes,
        } => ("displaced", item, Some(from), target_indexes),
    };
    table.set_field(ctx, "kind", kind);
    table.set_field(ctx, "id", item.id.raw() as i64);
    table.set_field(ctx, "index", item.index as i64 + 1);
    table.set_field(ctx, "destination", item.destination);
    table.set_field(
        ctx,
        "from",
        from.map_or(LuaValue::Nil, |index| LuaValue::Integer(index as i64 + 1)),
    );
    let target_indexes = Table::new(&ctx);
    for (index, target) in targets.into_iter().enumerate() {
        target_indexes
            .set(ctx, index as i64 + 1, target as i64 + 1)
            .expect("target-index table accepts integer keys");
    }
    table.set_field(ctx, "target_indexes", target_indexes);
    table
}

fn execute_module<'gc>(
    ctx: Context<'gc>,
    name: &str,
    source: &[u8],
    limits: Limits,
) -> Result<LuaValue<'gc>, String> {
    let closure = Closure::load(ctx, Some(name), source).map_err(|error| error.to_string())?;
    let executor = Executor::start(ctx, closure.into(), ());
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua module fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            return match executor.take_result::<LuaValue>(ctx) {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
        }
    }
}

fn element_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
    element: Element,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let node = create_node(&state, element);
        configure_element(&state, ctx, limits, node, properties).map_err(HostError)?;
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

fn loader_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let clean = Table::new(&ctx);
        let mut source = None;
        for (key, value) in properties.iter(ctx) {
            if matches!(key, LuaValue::String(name) if name.display_lossy().to_string() == "source")
            {
                let LuaValue::Function(Function::Closure(factory)) = value else {
                    return Err(HostError("Loader source must be a function".into()).into());
                };
                source = Some(ctx.stash(factory));
            } else {
                clean.set(ctx, key, value)?;
            }
        }
        let node = create_node(&state, Element::Loader);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        if let Some(source) = source.clone() {
            state.borrow_mut().loader_factories.insert(node, source);
        }
        if state
            .borrow()
            .scene
            .bool_value(node, "active")
            .map_err(|error| HostError(error.to_string()))?
            && let Some(source) = source
        {
            let child = execute_node_factory(ctx, &source, limits).map_err(HostError)?;
            state
                .borrow_mut()
                .scene
                .reparent(child, Some(node))
                .map_err(|error| HostError(error.to_string()))?;
            state.borrow_mut().loaded_loaders.insert(node);
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

fn timer_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let properties: Table = stack.consume(ctx)?;
        let clean = Table::new(&ctx);
        let mut callback = None;
        for (key, value) in properties.iter(ctx) {
            if matches!(key, LuaValue::String(name) if name.display_lossy().to_string() == "on_triggered")
            {
                let LuaValue::Function(Function::Closure(closure)) = value else {
                    return Err(HostError("Timer on_triggered must be a function".into()).into());
                };
                callback = Some(ctx.stash(closure));
            } else {
                clean.set(ctx, key, value)?;
            }
        }
        let node = create_node(&state, Element::Timer);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        let (interval, repeat, running) = {
            let state = state.borrow();
            let interval = state
                .scene
                .number(node, "interval")
                .map_err(|error| HostError(error.to_string()))?;
            let repeat = state
                .scene
                .bool_value(node, "repeat")
                .map_err(|error| HostError(error.to_string()))?;
            let running = state
                .scene
                .bool_value(node, "running")
                .map_err(|error| HostError(error.to_string()))?;
            (interval, repeat, running)
        };
        if running {
            if !interval.is_finite() || interval <= 0.0 {
                return Err(HostError("Timer interval must be finite and positive".into()).into());
            }
            let callback =
                callback.ok_or_else(|| HostError("running Timer requires on_triggered".into()))?;
            let timer = IoTimer::every(Duration::from_secs_f64(interval / 1_000.0))
                .map_err(|error| HostError(error.to_string()))?;
            let interval = Duration::from_secs_f64(interval / 1_000.0);
            state.borrow_mut().timers.push(PendingTimer {
                timer,
                callback: callback.clone(),
                repeat,
                interval,
                node: Some(node),
            });
            state.borrow_mut().timer_callbacks.insert(node, callback);
        } else if let Some(callback) = callback {
            state.borrow_mut().timer_callbacks.insert(node, callback);
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

fn view_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
    kind: ViewKind,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let virtualized = !matches!(kind, ViewKind::Repeater);
        let properties: Table = stack.consume(ctx)?;
        let model = match properties.get_value(ctx, "model") {
            LuaValue::UserData(model) => model
                .downcast_static::<ListModelToken>()
                .map_err(|_| HostError("view model must be a mold list model".to_owned()))?,
            _ => return Err(HostError("view model must be a mold list model".to_owned()).into()),
        };
        let delegate = match properties.get_value(ctx, "delegate") {
            LuaValue::Function(Function::Closure(delegate)) => ctx.stash(delegate),
            _ => return Err(HostError("view delegate must be a function".to_owned()).into()),
        };
        let clean = Table::new(&ctx);
        for (key, value) in properties.iter(ctx) {
            let special = matches!(
                key,
                LuaValue::String(name)
                    if matches!(
                        name.display_lossy().to_string().as_str(),
                        "model"
                            | "delegate"
                            | "item_extent"
                            | "overscan"
                            | "content_y"
                            | "cell_width"
                            | "cell_height"
                            | "columns"
                    )
            );
            if !special {
                clean
                    .set(ctx, key, value)
                    .map_err(|error| HostError(error.to_string()))?;
            }
        }
        if virtualized {
            clean.set_field(ctx, "clip", true);
        }
        let node = create_node(&state, Element::Item);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        let model_handle = Rc::clone(&model.model);
        let model = model_handle.borrow();
        let mut configured_view = None;
        let (range, item_extent, offset, columns, column_extent) = match kind {
            ViewKind::Repeater => (0..model.len(), 0.0, 0.0, 1, 0.0),
            ViewKind::List => {
                let item_extent =
                    table_number(ctx, properties, "item_extent", 1.0).map_err(HostError)?;
                let height = table_number(ctx, properties, "height", 0.0).map_err(HostError)?;
                let offset = table_number(ctx, properties, "content_y", 0.0).map_err(HostError)?;
                let overscan = table_number(ctx, properties, "overscan", 1.0).map_err(HostError)?;
                if item_extent <= 0.0 || height < 0.0 || offset < 0.0 || overscan < 0.0 {
                    return Err(HostError("invalid ListView dimensions".to_owned()).into());
                }
                let mut view = VirtualList::new(item_extent, height, overscan as usize)
                    .ok_or_else(|| HostError("invalid ListView dimensions".to_owned()))?;
                view.set_offset(offset);
                let range = view.visible_range(model.len());
                configured_view = Some(view);
                (range, item_extent, offset, 1, 0.0)
            }
            ViewKind::Grid => {
                let cell_width =
                    table_number(ctx, properties, "cell_width", 1.0).map_err(HostError)?;
                let cell_height =
                    table_number(ctx, properties, "cell_height", 1.0).map_err(HostError)?;
                let width = table_number(ctx, properties, "width", 0.0).map_err(HostError)?;
                let height = table_number(ctx, properties, "height", 0.0).map_err(HostError)?;
                let offset = table_number(ctx, properties, "content_y", 0.0).map_err(HostError)?;
                let overscan = table_number(ctx, properties, "overscan", 1.0).map_err(HostError)?;
                let default_columns = (width / cell_width).floor().max(1.0);
                let columns =
                    table_number(ctx, properties, "columns", default_columns).map_err(HostError)?;
                if cell_width <= 0.0
                    || cell_height <= 0.0
                    || width < 0.0
                    || height < 0.0
                    || offset < 0.0
                    || overscan < 0.0
                    || columns < 1.0
                    || columns.fract() != 0.0
                {
                    return Err(HostError("invalid GridView dimensions".to_owned()).into());
                }
                let columns = columns as usize;
                let mut view =
                    VirtualList::new_grid(cell_height, height, overscan as usize, columns)
                        .ok_or_else(|| HostError("invalid GridView dimensions".to_owned()))?;
                view.set_offset(offset);
                let range = view.visible_range(model.len());
                configured_view = Some(view);
                (range, cell_height, offset, columns, cell_width)
            }
        };
        let mut active = HashMap::new();
        for index in range {
            let (id, item) = model
                .get(index)
                .expect("view range contains live model indexes");
            let child = execute_delegate(ctx, &delegate, item, index, limits).map_err(HostError)?;
            if virtualized {
                position_view_child(
                    &mut state.borrow_mut().scene,
                    child,
                    index,
                    item_extent,
                    offset,
                    columns,
                    column_extent,
                )
                .map_err(HostError)?;
            }
            state
                .borrow_mut()
                .scene
                .reparent(child, Some(node))
                .map_err(|error| HostError(error.to_string()))?;
            active.insert(id, child);
        }
        drop(model);
        if let Some(mut view) = configured_view {
            let _ = view.sync(&model_handle.borrow(), &[]);
            state.borrow_mut().views.insert(
                node,
                LuaVirtualView {
                    model: model_handle,
                    view,
                    delegate,
                    active,
                    column_extent,
                },
            );
        }
        stack.replace(ctx, node_userdata(ctx, Rc::clone(&state), node));
        Ok(CallbackReturn::Return)
    })
}

fn execute_delegate(
    ctx: Context<'_>,
    delegate: &StashedClosure,
    item: &SceneValue,
    index: usize,
    limits: Limits,
) -> Result<NodeHandle, String> {
    let args = Variadic(vec![
        scene_to_lua(ctx, item)?,
        LuaValue::Integer(index as i64 + 1),
    ]);
    let executor = Executor::start(ctx, ctx.fetch(delegate).into(), args);
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua delegate fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    match executor.take_result::<UserRef<NodeToken>>(ctx) {
        Ok(Ok(node)) => Ok(node.handle),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn execute_node_factory(
    ctx: Context<'_>,
    factory: &StashedClosure,
    limits: Limits,
) -> Result<NodeHandle, String> {
    let executor = Executor::start(ctx, ctx.fetch(factory).into(), ());
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua Loader source fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    match executor.take_result::<UserRef<NodeToken>>(ctx) {
        Ok(Ok(node)) => Ok(node.handle),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn position_view_child(
    scene: &mut Scene,
    node: NodeHandle,
    index: usize,
    row_extent: f64,
    offset: f64,
    columns: usize,
    column_extent: f64,
) -> Result<(), String> {
    scene
        .assign(node, "x", (index % columns) as f64 * column_extent)
        .map_err(|error| error.to_string())?;
    scene
        .assign(node, "y", (index / columns) as f64 * row_extent - offset)
        .map_err(|error| error.to_string())
}

fn reconcile_lua_view(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    parent: NodeHandle,
    offset: f64,
    view: &mut LuaVirtualView,
) -> Result<Vec<ViewTransition>, String> {
    if !offset.is_finite() || offset < 0.0 {
        return Err("ListView offset must be finite and non-negative".to_owned());
    }
    view.view.set_offset(offset);
    let changes = view.model.borrow_mut().take_changes();
    let updated = changes
        .iter()
        .filter_map(|change| match change {
            ListChange::Updated { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let model = view.model.borrow();
    let transitions = view.view.sync(&model, &changes);
    let visible = view
        .view
        .visible_range(model.len())
        .filter_map(|index| {
            model
                .get(index)
                .map(|(id, value)| (id, index, value.clone()))
        })
        .collect::<Vec<_>>();
    drop(model);
    let visible_ids = visible.iter().map(|(id, _, _)| *id).collect::<HashSet<_>>();
    let mut created = Vec::new();
    for (id, index, item) in &visible {
        if !view.active.contains_key(id) || updated.contains(id) {
            match execute_delegate(ctx, &view.delegate, item, *index, limits) {
                Ok(node) => created.push((*id, *index, node)),
                Err(error) => {
                    for (_, _, node) in created {
                        let _ = state.borrow_mut().scene.remove(node);
                    }
                    return Err(error);
                }
            }
        }
    }
    let removed = view
        .active
        .iter()
        .filter(|(id, _)| !visible_ids.contains(id) || updated.contains(id))
        .map(|(id, node)| (*id, *node))
        .collect::<Vec<_>>();
    for (id, node) in removed {
        state
            .borrow_mut()
            .scene
            .remove(node)
            .map_err(|error| error.to_string())?;
        view.active.remove(&id);
    }
    for (id, index, node) in created {
        position_view_child(
            &mut state.borrow_mut().scene,
            node,
            index,
            view.view.item_extent(),
            offset,
            view.view.columns(),
            view.column_extent,
        )?;
        state
            .borrow_mut()
            .scene
            .reparent(node, Some(parent))
            .map_err(|error| error.to_string())?;
        view.active.insert(id, node);
    }
    for (id, index, _) in visible {
        if let Some(node) = view.active.get(&id) {
            position_view_child(
                &mut state.borrow_mut().scene,
                *node,
                index,
                view.view.item_extent(),
                offset,
                view.view.columns(),
                view.column_extent,
            )?;
        }
    }
    Ok(transitions)
}

fn configure_element<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    limits: Limits,
    node: NodeHandle,
    properties: Table<'gc>,
) -> Result<(), String> {
    let entries: Vec<_> = properties.iter(ctx).collect();
    let mut children = Vec::<(i64, NodeHandle)>::new();
    let mut named = Vec::<(String, LuaValue<'gc>)>::new();
    let mut state_value = None;
    for (key, value) in entries {
        match key {
            LuaValue::Integer(index) => {
                let LuaValue::UserData(child) = value else {
                    return Err(format!("child {index} must be a mold node"));
                };
                let child = child
                    .downcast_static::<NodeToken>()
                    .map_err(|_| format!("child {index} must be a mold node"))?;
                children.push((index, child.handle));
            }
            LuaValue::String(property) => {
                named.push((property.display_lossy().to_string(), value));
            }
            value => {
                return Err(format!(
                    "element table key must be a string or integer, found {}",
                    value.type_name()
                ));
            }
        }
    }
    if let Some((_, behavior)) = named.iter().find(|(name, _)| name == "behavior") {
        configure_behaviors(state, ctx, node, *behavior)?;
    }
    if let Some((_, states)) = named.iter().find(|(name, _)| name == "states") {
        let transitions = named
            .iter()
            .find(|(name, _)| name == "transitions")
            .map_or(LuaValue::Nil, |(_, value)| *value);
        configure_states(state, ctx, node, *states, transitions)?;
    }
    for (property, value) in named {
        if matches!(property.as_str(), "behavior" | "states" | "transitions") {
            continue;
        }
        if property == "state" {
            state_value = Some(value);
            continue;
        }
        if let Some(event) = handler_event(&property) {
            let LuaValue::Function(Function::Closure(closure)) = value else {
                return Err(format!("{property} must be a function"));
            };
            state
                .borrow_mut()
                .handlers
                .insert((node, event), ctx.stash(closure));
            continue;
        }
        if let LuaValue::Function(Function::Closure(closure)) = value {
            if !state
                .borrow()
                .scene
                .has_property(node, &property)
                .map_err(|error| error.to_string())?
            {
                let element = state
                    .borrow()
                    .scene
                    .element(node)
                    .map_err(|error| error.to_string())?;
                return Err(format!("unknown {element:?} property `{property}`"));
            }
            register_property_binding(state, ctx, limits, node, property, closure);
        } else {
            let value = lua_to_scene(ctx, value, 0)?;
            assign_scene_property(&mut state.borrow_mut(), node, &property, value)?;
        }
    }
    children.sort_by_key(|(index, _)| *index);
    for (_, child) in children {
        state
            .borrow_mut()
            .scene
            .reparent(child, Some(node))
            .map_err(|error| error.to_string())?;
    }
    if let Some(value) = state_value {
        match value {
            LuaValue::Function(Function::Closure(closure)) => {
                register_state_binding(state, ctx, limits, node, closure);
            }
            LuaValue::String(name) => {
                let mut remaining = limits.frame_fuel;
                apply_state(
                    state,
                    ctx,
                    limits,
                    &mut remaining,
                    node,
                    &name.display_lossy().to_string(),
                )?;
            }
            _ => return Err("state must be a string or binding function".into()),
        }
    }
    Ok(())
}

fn handler_event(property: &str) -> Option<UiEvent> {
    match property {
        "on_entered" => Some(UiEvent::PointerEntered),
        "on_exited" => Some(UiEvent::PointerExited),
        "on_position_changed" => Some(UiEvent::PointerMoved),
        "on_pressed" => Some(UiEvent::Pressed),
        "on_released" => Some(UiEvent::Released),
        "on_clicked" => Some(UiEvent::Clicked),
        "on_drag_started" => Some(UiEvent::DragStarted),
        "on_dragged" => Some(UiEvent::Dragged),
        "on_drag_finished" => Some(UiEvent::DragFinished),
        "on_key_pressed" => Some(UiEvent::KeyPressed),
        "on_touch_pressed" => Some(UiEvent::TouchPressed),
        "on_touch_moved" => Some(UiEvent::TouchMoved),
        "on_touch_released" => Some(UiEvent::TouchReleased),
        "on_touch_canceled" => Some(UiEvent::TouchCanceled),
        _ => None,
    }
}

fn configure_states<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: NodeHandle,
    states: LuaValue<'gc>,
    transitions: LuaValue<'gc>,
) -> Result<(), String> {
    let LuaValue::Table(states) = states else {
        return Err("states must be a name-keyed table".into());
    };
    let mut definitions = HashMap::new();
    for (name, definition) in states.iter(ctx) {
        let LuaValue::String(name) = name else {
            return Err("state names must be strings".into());
        };
        let LuaValue::Table(definition) = definition else {
            return Err("each state must be a table".into());
        };
        let mut properties = Vec::new();
        let mut anchors = None;
        let mut parent = None;
        for (key, value) in definition.iter(ctx) {
            let LuaValue::String(key) = key else {
                return Err("state fields must be strings".into());
            };
            match key.display_lossy().to_string().as_str() {
                "property_changes" => {
                    let LuaValue::Table(changes) = value else {
                        return Err("property_changes must be a table".into());
                    };
                    for (property, value) in changes.iter(ctx) {
                        let LuaValue::String(property) = property else {
                            return Err("property_changes keys must be strings".into());
                        };
                        let property = property.display_lossy().to_string();
                        if !state
                            .borrow()
                            .scene
                            .has_property(node, &property)
                            .map_err(|error| error.to_string())?
                        {
                            return Err(format!("state changes unknown property `{property}`"));
                        }
                        let value = match value {
                            LuaValue::Function(Function::Closure(closure)) => {
                                StateValue::Binding(ctx.stash(closure))
                            }
                            value => StateValue::Value(lua_to_scene(ctx, value, 0)?),
                        };
                        properties.push((property, value));
                    }
                }
                "anchors" | "anchor_changes" => {
                    let SceneValue::Map(value) = lua_to_scene(ctx, value, 0)? else {
                        return Err("anchor_changes must be a table".into());
                    };
                    anchors = Some(value);
                }
                "parent" | "parent_change" => {
                    let LuaValue::UserData(value) = value else {
                        return Err("parent_change must be a mold node".into());
                    };
                    parent = Some(
                        value
                            .downcast_static::<NodeToken>()
                            .map_err(|_| "parent_change must be a mold node".to_owned())?
                            .handle,
                    );
                }
                field => return Err(format!("unknown state field `{field}`")),
            }
        }
        definitions.insert(
            name.display_lossy().to_string(),
            StateDefinition {
                properties,
                anchors,
                parent,
            },
        );
    }
    let mut parsed_transitions = Vec::new();
    if let LuaValue::Table(transitions) = transitions {
        for (_, transition) in transitions.iter(ctx) {
            let LuaValue::Table(transition) = transition else {
                return Err("each transition must be a table".into());
            };
            let from = table_string(ctx, transition, "from", "*")?;
            let to = table_string(ctx, transition, "to", "*")?;
            let reversible = match transition.get_value(ctx, "reversible") {
                LuaValue::Nil => false,
                LuaValue::Boolean(value) => value,
                _ => return Err("transition reversible must be boolean".into()),
            };
            let duration = table_number(ctx, transition, "duration", 250.0)?;
            if duration < 0.0 {
                return Err("transition duration cannot be negative".into());
            }
            parsed_transitions.push(StateTransition {
                from,
                to,
                reversible,
                behavior: Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing: parse_easing(transition.get_value(ctx, "easing"))?,
                },
            });
        }
    } else if !matches!(transitions, LuaValue::Nil) {
        return Err("transitions must be an array table".into());
    }
    state.borrow_mut().states.insert(
        node,
        StateSet {
            definitions,
            transitions: parsed_transitions,
            current: None,
        },
    );
    Ok(())
}

fn configure_behaviors<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: NodeHandle,
    value: LuaValue<'gc>,
) -> Result<(), String> {
    let LuaValue::Table(behaviors) = value else {
        return Err("behavior must be a property-keyed table".to_owned());
    };
    for (property, behavior) in behaviors.iter(ctx) {
        let LuaValue::String(property) = property else {
            return Err("behavior keys must be property names".to_owned());
        };
        let LuaValue::Table(behavior) = behavior else {
            return Err("each behavior must be a table".to_owned());
        };
        let property = property.display_lossy().to_string();
        let kind = match behavior.get_value(ctx, "kind") {
            LuaValue::Nil => None,
            LuaValue::String(value) => Some(value.display_lossy().to_string()),
            _ => return Err("behavior kind must be a string".to_owned()),
        };
        if kind.as_deref() == Some("spring") {
            let physics = Physics::Spring {
                mass: table_number(ctx, behavior, "mass", 1.0)?,
                damping: table_number(ctx, behavior, "damping", 18.0)?,
                stiffness: table_number(ctx, behavior, "stiffness", 180.0)?,
                epsilon: table_number(ctx, behavior, "epsilon", 0.001)?,
            };
            state
                .borrow_mut()
                .scene
                .set_physics(node, &property, Some(physics))
                .map_err(|error| error.to_string())?;
            continue;
        }
        if kind.as_deref() == Some("smoothed") {
            let physics = Physics::Smoothed {
                velocity: table_number(ctx, behavior, "velocity", 1_000.0)?,
            };
            state
                .borrow_mut()
                .scene
                .set_physics(node, &property, Some(physics))
                .map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(kind) = kind {
            return Err(format!("unknown behavior kind `{kind}`"));
        }
        let duration = match behavior.get_value(ctx, "duration") {
            LuaValue::Integer(value) => value as f64,
            LuaValue::Number(value) if value.is_finite() => value,
            _ => return Err("behavior duration must be milliseconds".to_owned()),
        };
        if duration < 0.0 {
            return Err("behavior duration cannot be negative".to_owned());
        }
        let easing = parse_easing(behavior.get_value(ctx, "easing"))?;
        state
            .borrow_mut()
            .scene
            .set_behavior(
                node,
                &property,
                Some(Behavior {
                    duration: Duration::from_secs_f64(duration / 1_000.0),
                    easing,
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn table_number<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: f64,
) -> Result<f64, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default),
        LuaValue::Integer(value) => Ok(value as f64),
        LuaValue::Number(value) if value.is_finite() => Ok(value),
        _ => Err(format!("{field} must be a finite number")),
    }
}

fn table_string<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    field: &str,
    default: &str,
) -> Result<String, String> {
    match table.get_value(ctx, field) {
        LuaValue::Nil => Ok(default.to_owned()),
        LuaValue::String(value) => Ok(value.display_lossy().to_string()),
        _ => Err(format!("{field} must be a string")),
    }
}

fn table_string_array<'gc>(
    ctx: Context<'gc>,
    table: Table<'gc>,
    maximum: usize,
) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for (key, value) in table.iter(ctx) {
        let LuaValue::Integer(index) = key else {
            return Err("argument list keys must be integers".to_owned());
        };
        let LuaValue::String(value) = value else {
            return Err("process arguments must be strings".to_owned());
        };
        values.push((index, value.display_lossy().to_string()));
    }
    if values.len() > maximum {
        return Err(format!("argument list exceeds {maximum} entries"));
    }
    values.sort_by_key(|(index, _)| *index);
    for (offset, (index, _)) in values.iter().enumerate() {
        if *index != offset as i64 + 1 {
            return Err("argument list must be a dense sequence".to_owned());
        }
    }
    Ok(values.into_iter().map(|(_, value)| value).collect())
}

fn string_table<'gc>(ctx: Context<'gc>, values: impl IntoIterator<Item = String>) -> Table<'gc> {
    let table = Table::new(&ctx);
    for (index, value) in values.into_iter().enumerate() {
        table
            .set(ctx, index as i64 + 1, value)
            .expect("string table accepts integer keys");
    }
    table
}

fn greetd_response<'gc>(ctx: Context<'gc>, response: GreetdResponse) -> Table<'gc> {
    let value = Table::new(&ctx);
    match response {
        GreetdResponse::Success => {
            value.set_field(ctx, "type", "success");
        }
        GreetdResponse::AuthMessage { kind, message } => {
            value.set_field(ctx, "type", "auth_message");
            value.set_field(
                ctx,
                "auth_message_type",
                match kind {
                    AuthMessageType::Visible => "visible",
                    AuthMessageType::Secret => "secret",
                    AuthMessageType::Info => "info",
                    AuthMessageType::Error => "error",
                },
            );
            value.set_field(ctx, "auth_message", message.as_str());
        }
        GreetdResponse::Error {
            authentication,
            description,
        } => {
            value.set_field(ctx, "type", "error");
            value.set_field(ctx, "authentication", authentication);
            value.set_field(ctx, "description", description.as_str());
        }
    }
    value
}

fn bounded_timeout(milliseconds: i64) -> Result<Duration, String> {
    u64::try_from(milliseconds)
        .ok()
        .filter(|milliseconds| *milliseconds <= 5_000)
        .map(Duration::from_millis)
        .ok_or_else(|| "timeout must be between 0 and 5000 milliseconds".to_owned())
}

fn parse_easing(value: LuaValue<'_>) -> Result<Easing, String> {
    match value {
        LuaValue::Nil => Ok(Easing::Linear),
        LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
            "linear" => Ok(Easing::Linear),
            "in_cubic" => Ok(Easing::InCubic),
            "out_cubic" => Ok(Easing::OutCubic),
            "in_out_cubic" => Ok(Easing::InOutCubic),
            name => Err(format!("unknown easing `{name}`")),
        },
        _ => Err("easing must be a string".to_owned()),
    }
}

fn register_property_binding<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    limits: Limits,
    node: NodeHandle,
    property: String,
    closure: Closure<'gc>,
) {
    let name = format!("{node:?}.{property}");
    {
        let mut state = state.borrow_mut();
        let token = state.next_effect;
        state.next_effect = state.next_effect.wrapping_add(1);
        state.effects.insert(
            token,
            LuaEffect {
                closure: ctx.stash(closure),
                sink: Some(EffectSink::Property(PropertySink { node, property })),
            },
        );
        state
            .graph
            .as_mut()
            .expect("reactive graph unavailable outside evaluation")
            .external_effect(name, token);
    }
    let _ = flush_reactive(state, ctx, limits);
}

fn register_state_binding<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    limits: Limits,
    node: NodeHandle,
    closure: Closure<'gc>,
) {
    {
        let mut state = state.borrow_mut();
        let token = state.next_effect;
        state.next_effect = state.next_effect.wrapping_add(1);
        state.effects.insert(
            token,
            LuaEffect {
                closure: ctx.stash(closure),
                sink: Some(EffectSink::State(node)),
            },
        );
        state
            .graph
            .as_mut()
            .expect("reactive graph unavailable outside evaluation")
            .external_effect(format!("{node:?}.state"), token);
    }
    let _ = flush_reactive(state, ctx, limits);
}

fn apply_state(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    frame_remaining: &mut u64,
    node: NodeHandle,
    name: &str,
) -> Result<(), String> {
    let (definition, old, transition) = {
        let state = state.borrow();
        let set = state
            .states
            .get(&node)
            .ok_or_else(|| format!("node has no states for `{name}`"))?;
        let definition = set
            .definitions
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown state `{name}`"))?;
        let old = set.current.clone().unwrap_or_default();
        let transition = set.transitions.iter().find_map(|transition| {
            let forward = (transition.from == "*" || transition.from == old)
                && (transition.to == "*" || transition.to == name);
            let reverse = transition.reversible
                && (transition.from == "*" || transition.from == name)
                && (transition.to == "*" || transition.to == old);
            (forward || reverse).then_some(transition.behavior)
        });
        (definition, old, transition)
    };
    let transition = (old != name).then_some(transition).flatten();
    let mut properties = Vec::new();
    for (property, value) in definition.properties {
        let value = match value {
            StateValue::Value(value) => value,
            StateValue::Binding(closure) => {
                execute_effect(ctx, &closure, limits, frame_remaining, true)?
                    .ok_or_else(|| format!("state property `{property}` returned no value"))?
                    .to_scene()
            }
        };
        properties.push((property, value));
    }
    let mut state = state.borrow_mut();
    for (property, value) in properties {
        let animated = transition.is_some()
            && matches!(value, SceneValue::Number(_) | SceneValue::Color(_))
            && matches!(
                state.scene.current(node, &property),
                Ok(SceneValue::Number(_) | SceneValue::Color(_))
            );
        if animated {
            let from = state
                .scene
                .current(node, &property)
                .map_err(|error| error.to_string())?
                .clone();
            animate_scene_property(
                &mut state,
                node,
                &property,
                from,
                value,
                transition.unwrap(),
            )?;
        } else {
            assign_scene_property(&mut state, node, &property, value)?;
        }
    }
    if old != name && (definition.parent.is_some() || definition.anchors.is_some()) {
        let parent = definition.parent.or(state
            .scene
            .parent(node)
            .map_err(|error| error.to_string())?);
        if let Some(parent) = parent {
            if old.is_empty() && transition.is_none() {
                if let Some(anchors) = definition.anchors {
                    assign_scene_property(&mut state, node, "anchors", SceneValue::Map(anchors))?;
                }
                state
                    .scene
                    .reparent(node, Some(parent))
                    .map_err(|error| error.to_string())?;
            } else {
                state.parent_transitions.push(ParentTransitionRequest {
                    node,
                    parent,
                    anchors: definition.anchors,
                    behavior: transition.unwrap_or(Behavior {
                        duration: Duration::ZERO,
                        easing: Easing::Linear,
                    }),
                });
            }
        }
    }
    state.states.get_mut(&node).unwrap().current = Some(name.to_owned());
    Ok(())
}

fn lua_to_scene<'gc>(
    ctx: Context<'gc>,
    value: LuaValue<'gc>,
    depth: usize,
) -> Result<SceneValue, String> {
    if depth >= 16 {
        return Err("declarative value nesting exceeds 16 levels".to_owned());
    }
    match value {
        LuaValue::Nil => Ok(SceneValue::Nil),
        LuaValue::Boolean(value) => Ok(SceneValue::Bool(value)),
        LuaValue::Integer(value) => Ok(SceneValue::Number(value as f64)),
        LuaValue::Number(value) if value.is_finite() => Ok(SceneValue::Number(value)),
        LuaValue::String(value) => Ok(SceneValue::String(value.display_lossy().to_string())),
        LuaValue::Table(table) => {
            let entries: Vec<_> = table.iter(ctx).collect();
            let is_list = entries
                .iter()
                .all(|(key, _)| matches!(key, LuaValue::Integer(index) if *index > 0));
            if is_list {
                let mut items = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let LuaValue::Integer(index) = key else {
                            unreachable!()
                        };
                        Ok((index, lua_to_scene(ctx, value, depth + 1)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                items.sort_by_key(|(index, _)| *index);
                Ok(SceneValue::List(
                    items.into_iter().map(|(_, value)| value).collect(),
                ))
            } else {
                let mut map = std::collections::BTreeMap::new();
                for (key, value) in entries {
                    let LuaValue::String(key) = key else {
                        return Err("declarative maps require string keys".to_owned());
                    };
                    map.insert(
                        key.display_lossy().to_string(),
                        lua_to_scene(ctx, value, depth + 1)?,
                    );
                }
                Ok(SceneValue::Map(map))
            }
        }
        value => Err(format!(
            "scene properties do not support {} values",
            value.type_name()
        )),
    }
}

fn replace_status<'gc>(
    ctx: Context<'gc>,
    stack: &mut luna::Stack<'gc, '_>,
    result: Result<(), String>,
) {
    match result {
        Ok(()) => stack.replace(ctx, (true, LuaValue::Nil)),
        Err(message) => stack.replace(ctx, (false, message)),
    }
}

fn flush_reactive(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
) -> Result<(), String> {
    let mut graph = state
        .borrow_mut()
        .graph
        .take()
        .ok_or_else(|| "reactive graph is already running".to_owned())?;
    let mut remaining = limits.frame_fuel;
    let result = graph.flush_external(|token, effect| {
        evaluate_effect(state, ctx, limits, &mut remaining, token, effect)
    });

    let mut state = state.borrow_mut();
    for signal in state.signals.clone() {
        if let Ok(value) = graph.read(signal) {
            state.values.insert(signal, value.clone());
        }
    }
    state.graph = Some(graph);

    match result {
        Ok(report) if report.errors.is_empty() => Ok(()),
        Ok(report) => {
            let message = report
                .errors
                .into_iter()
                .map(|error| format!("{}: {}", error.effect, error.message))
                .collect::<Vec<_>>()
                .join("; ");
            state.logs.push(message.clone());
            Err(message)
        }
        Err(error) => {
            let message = error.to_string();
            state.logs.push(message.clone());
            Err(message)
        }
    }
}

fn evaluate_effect(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'_>,
    limits: Limits,
    frame_remaining: &mut u64,
    token: u64,
    effect: &mut EffectContext<'_, ScriptValue>,
) -> Result<(), String> {
    let lua_effect = {
        let mut state = state.borrow_mut();
        if state.active.is_some() {
            return Err("reactive effects cannot run recursively".to_owned());
        }
        state.active = Some(Capture::default());
        state.effect_runs = state.effect_runs.saturating_add(1);
        state
            .effects
            .get(&token)
            .cloned()
            .ok_or_else(|| format!("missing Lua closure for effect {token}"))?
    };
    let result = execute_effect(
        ctx,
        &lua_effect.closure,
        limits,
        frame_remaining,
        lua_effect.sink.is_some(),
    );
    let state_result = if let (Ok(Some(value)), Some(EffectSink::State(node))) =
        (&result, lua_effect.sink.clone())
    {
        match value {
            ScriptValue::String(name) => {
                apply_state(state, ctx, limits, frame_remaining, node, name)
            }
            _ => Err("state binding must return a string".into()),
        }
    } else {
        Ok(())
    };
    let state_result = state_result.and_then(|()| {
        if let (Ok(Some(value)), Some(sink)) = (&result, lua_effect.sink) {
            match sink {
                EffectSink::Property(sink) => assign_scene_property(
                    &mut state.borrow_mut(),
                    sink.node,
                    &sink.property,
                    value.to_scene(),
                ),
                EffectSink::State(_) => Ok(()),
            }
        } else {
            Ok(())
        }
    });
    let capture = state.borrow_mut().active.take().unwrap_or_default();
    for (node, property, target) in capture.property_reads {
        let key = (node, property.clone(), target);
        let signal = if let Some(signal) = state.borrow().property_signals.get(&key).copied() {
            signal
        } else {
            let name = format!("{node:?}.{property}{}", if target { "_target" } else { "" });
            let value = ScriptValue::Integer(state.borrow().property_revision);
            let signal = effect.signal(name.clone(), value.clone());
            let mut state = state.borrow_mut();
            state.property_signals.insert(key, signal);
            if !target {
                state.current_property_names.insert(name, (node, property));
            }
            state.values.insert(signal, value);
            state.signals.push(signal);
            signal
        };
        effect.get(signal).map_err(|error| error.to_string())?;
    }
    for signal in capture.reads {
        effect.get(signal).map_err(|error| error.to_string())?;
    }
    if result.is_ok() {
        for (signal, value) in capture.writes {
            effect
                .set(signal, value.clone())
                .map_err(|error| error.to_string())?;
            state.borrow_mut().values.insert(signal, value);
        }
    }
    state_result?;
    result.map(|_| ())
}

fn execute_effect(
    ctx: Context<'_>,
    closure: &StashedClosure,
    limits: Limits,
    frame_remaining: &mut u64,
    capture_value: bool,
) -> Result<Option<ScriptValue>, String> {
    let budget = limits.effect_fuel.min(*frame_remaining);
    if budget == 0 {
        return Err("Lua frame fuel exhausted".to_owned());
    }
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), ());
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            *frame_remaining = frame_remaining.saturating_sub(budget);
            return Err(format!(
                "Lua effect fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            let spent = budget - remaining;
            *frame_remaining = frame_remaining.saturating_sub(spent);
            return if capture_value {
                match executor.take_result::<LuaValue>(ctx) {
                    Ok(Ok(value)) => ScriptValue::from_lua(value).map(Some),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                }
            } else {
                match executor.take_result::<()>(ctx) {
                    Ok(Ok(())) => Ok(None),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                }
            };
        }
    }
}

fn execute_handler_args(
    ctx: Context<'_>,
    closure: &StashedClosure,
    args: &[IpcValue],
    limits: Limits,
) -> Result<(), String> {
    let args = Variadic(
        args.iter()
            .map(|value| value.to_lua(ctx))
            .collect::<Vec<_>>(),
    );
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua handler fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn execute_screencopy_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    result: Result<Screencopy, String>,
    limits: Limits,
) -> Result<(), String> {
    let args = match result {
        Ok(frame) => {
            let value = Table::new(&ctx);
            value.set_field(ctx, "width", i64::from(frame.width));
            value.set_field(ctx, "height", i64::from(frame.height));
            value.set_field(ctx, "stride", i64::from(frame.stride));
            value.set_field(ctx, "format", frame.format.as_str());
            value.set_field(ctx, "y_invert", frame.y_invert);
            value.set_field(ctx, "pixels", ctx.intern(&frame.pixels));
            Variadic(vec![LuaValue::Table(value), LuaValue::Nil])
        }
        Err(error) => Variadic(vec![
            LuaValue::Nil,
            LuaValue::String(ctx.intern(error.as_bytes())),
        ]),
    };
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua handler fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn execute_dbus_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    value: DbusValue,
    limits: Limits,
) -> Result<(), String> {
    let argument = dbus_value_to_lua(ctx, value)?;
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), Variadic(vec![argument]));
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua handler fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    match executor.take_result::<()>(ctx) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn udev_event_value(event: UdevEvent) -> DbusValue {
    let properties = event
        .properties
        .into_iter()
        .map(|(key, value)| (key, DbusValue::String(value)))
        .collect();
    DbusValue::Map(BTreeMap::from([
        ("action".to_owned(), DbusValue::String(event.action)),
        ("devpath".to_owned(), DbusValue::String(event.devpath)),
        (
            "subsystem".to_owned(),
            event.subsystem.map_or(DbusValue::Nil, DbusValue::String),
        ),
        (
            "devname".to_owned(),
            event.devname.map_or(DbusValue::Nil, DbusValue::String),
        ),
        ("properties".to_owned(), DbusValue::Map(properties)),
    ]))
}

fn status_notifier_value(items: Vec<StatusNotifierAddress>) -> DbusValue {
    DbusValue::List(
        items
            .into_iter()
            .map(|item| {
                DbusValue::Map(BTreeMap::from([
                    ("service".to_owned(), DbusValue::String(item.service)),
                    ("path".to_owned(), DbusValue::String(item.path)),
                ]))
            })
            .collect(),
    )
}

fn execute_ipc_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    args: &[IpcValue],
    limits: Limits,
) -> Result<Vec<IpcValue>, String> {
    let args = Variadic(
        args.iter()
            .map(|value| value.to_lua(ctx))
            .collect::<Vec<_>>(),
    );
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), args);
    let budget = limits.effect_fuel;
    let mut remaining = budget;
    loop {
        if remaining == 0 {
            executor.stop(&ctx);
            return Err(format!(
                "Lua IPC handler fuel exhausted after {budget} instructions"
            ));
        }
        let allowance = remaining.min(limits.slice_fuel.max(1) as u64) as i32;
        let mut fuel = Fuel::with(allowance);
        let finished = executor
            .step(ctx, &mut fuel)
            .map_err(|error| error.to_string())?;
        let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
        remaining = remaining.saturating_sub(consumed.max(1));
        if finished {
            break;
        }
    }
    let values = match executor.take_result::<Variadic<Vec<LuaValue>>>(ctx) {
        Ok(Ok(values)) => values,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(error) => return Err(error.to_string()),
    };
    values.into_iter().map(IpcValue::from_lua).collect()
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    use super::*;

    #[test]
    fn executes_a_chunk() {
        let mut runtime = Runtime::default();
        runtime
            .execute("test.lua", b"local answer = 40 + 2")
            .unwrap();
    }

    #[test]
    fn runtimepath_loads_user_modules_without_rust_registration() {
        let root = std::env::temp_dir().join(format!("mold-runtime-{}", std::process::id()));
        let module = root.join("lua/user/widget.lua");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, b"return { answer = 42 }").unwrap();
        let shell = root.join("shell.lua");
        let mut runtime = Runtime::default();

        runtime
            .execute(
                &shell.to_string_lossy(),
                b"local widget = require('user.widget'); assert(widget.answer == 42)",
            )
            .unwrap();

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ipc_registry_calls_named_bounded_handlers() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "ipc.lua",
                br#"
                    mold.ipc["launcher.toggle"] = function(name, count)
                        return "hello " .. name, count + 1, true
                    end
                "#,
            )
            .unwrap();

        assert_eq!(runtime.ipc_verbs(), ["launcher.toggle"]);
        assert_eq!(
            runtime
                .call_ipc(
                    "launcher.toggle",
                    &[IpcValue::String("mold".into()), IpcValue::Integer(2)],
                )
                .unwrap(),
            [
                IpcValue::String("hello mold".into()),
                IpcValue::Integer(3),
                IpcValue::Boolean(true),
            ]
        );
        assert!(runtime.call_ipc("missing", &[]).is_err());
    }

    #[test]
    fn ipc_handlers_are_fuel_bounded() {
        let mut runtime = Runtime::new(Limits {
            effect_fuel: 256,
            ..Limits::default()
        });
        runtime
            .execute(
                "ipc-fuel.lua",
                b"mold.ipc.loop = function() while true do end end",
            )
            .unwrap();

        let error = runtime.call_ipc("loop", &[]).unwrap_err().to_string();
        assert!(error.contains("IPC handler fuel exhausted"), "{error}");
    }

    #[test]
    fn reloadable_signals_carry_state_into_a_new_runtime() {
        let source = br#"
            local visible = mold.reloadable("launcher.visible", false)
            mold.ipc["state.set"] = function(value) visible:set(value) end
            mold.ipc["state.get"] = function() return visible:get() end
        "#;
        let mut first = Runtime::default();
        first.execute("reloadable.lua", source).unwrap();
        first
            .call_ipc("state.set", &[IpcValue::Boolean(true)])
            .unwrap();

        let mut second = Runtime::default();
        second.restore_reloadable_state(first.reloadable_state());
        second.execute("reloadable.lua", source).unwrap();

        assert_eq!(
            second.call_ipc("state.get", &[]).unwrap(),
            [IpcValue::Boolean(true)]
        );
    }

    #[test]
    fn idle_callbacks_receive_compositor_state() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "idle.lua",
                br#"
                    local idle = mold.signal("idle", false)
                    mold.idle.subscribe(30000, function(value) idle:set(value) end)
                    mold.ipc["idle.get"] = function() return idle:get() end
                "#,
            )
            .unwrap();

        assert_eq!(runtime.idle_timeouts(), [30_000]);
        assert!(runtime.dispatch_idle(30_000, true));
        assert_eq!(
            runtime.call_ipc("idle.get", &[]).unwrap(),
            [IpcValue::Boolean(true)]
        );
    }

    #[test]
    fn output_power_requests_are_bounded_and_ordered() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "power.lua",
                br#"
                    mold.output_power.set("off")
                    mold.output_power.set("on")
                "#,
            )
            .unwrap();

        assert_eq!(runtime.take_output_power_requests(), [false, true]);
        assert!(runtime.take_output_power_requests().is_empty());
    }

    #[test]
    fn clipboard_bridges_publications_and_selections() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "clipboard.lua",
                br#"
                    local current = mold.signal("clipboard", "")
                    mold.clipboard.subscribe(function(text) current:set(text or "none") end)
                    mold.clipboard.set("copied")
                    mold.ipc["clipboard.get"] = function() return current:get() end
                "#,
            )
            .unwrap();

        assert_eq!(runtime.take_clipboard_requests(), ["copied"]);
        assert!(runtime.dispatch_clipboard(Some("pasted".to_owned())));
        assert_eq!(
            runtime.call_ipc("clipboard.get", &[]).unwrap(),
            [IpcValue::String("pasted".to_owned())]
        );
        assert!(runtime.dispatch_clipboard(None));
        assert_eq!(
            runtime.call_ipc("clipboard.get", &[]).unwrap(),
            [IpcValue::String("none".to_owned())]
        );
    }

    #[test]
    fn screencopy_bridges_bounded_requests_and_pixels() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "screencopy.lua",
                br#"
                    local result = mold.signal("capture", "pending")
                    mold.screencopy.capture(true, function(frame, err)
                        if err then
                            result:set(err)
                        else
                            result:set(frame.format .. ":" .. frame.width .. ":" ..
                                #frame.pixels .. ":" .. string.byte(frame.pixels, 1))
                        end
                    end)
                    local second = mold.signal("second", "pending")
                    mold.screencopy.capture(false, function(_, err) second:set(err) end)
                    mold.ipc["capture.get"] = function() return result:get() end
                    mold.ipc["second.get"] = function() return second:get() end
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.take_screencopy_requests(),
            [
                ScreencopyRequest {
                    id: 0,
                    include_cursor: true,
                },
                ScreencopyRequest {
                    id: 1,
                    include_cursor: false,
                },
            ]
        );
        assert!(runtime.dispatch_screencopy(1, Err("second failed".to_owned())));
        assert!(runtime.dispatch_screencopy(
            0,
            Ok(Screencopy {
                width: 2,
                height: 1,
                stride: 8,
                format: "argb8888".to_owned(),
                y_invert: false,
                pixels: vec![7; 8],
            })
        ));
        assert_eq!(
            runtime.call_ipc("capture.get", &[]).unwrap(),
            [IpcValue::String("argb8888:2:8:7".to_owned())]
        );
        assert_eq!(
            runtime.call_ipc("second.get", &[]).unwrap(),
            [IpcValue::String("second failed".to_owned())]
        );
    }

    #[test]
    fn virtual_keyboard_requests_preserve_protocol_order() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "keyboard.lua",
                br#"
                    mold.virtual_keyboard.modifiers(1, 2, 4, 0)
                    mold.virtual_keyboard.key(30, true)
                    mold.virtual_keyboard.key(30, false)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.take_virtual_keyboard_requests(),
            [
                VirtualKeyboardRequest::Modifiers {
                    depressed: 1,
                    latched: 2,
                    locked: 4,
                    group: 0,
                },
                VirtualKeyboardRequest::Key {
                    keycode: 30,
                    pressed: true,
                },
                VirtualKeyboardRequest::Key {
                    keycode: 30,
                    pressed: false,
                },
            ]
        );
    }

    #[test]
    fn xkb_facade_builds_osk_layout_tables() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "xkb.lua",
                br#"
                    local keymap = mold.xkb.compile { layout = "us" }
                    assert(string.find(keymap.source, "xkb_keymap", 1, true))
                    local found = false
                    for _, key in ipairs(keymap.keys) do
                        if key.name == "AC01" then
                            assert(key.evdev_code == 30)
                            assert(key.layouts[1][1][1].text == "a")
                            assert(key.layouts[1][2][1].text == "A")
                            found = true
                        end
                    end
                    assert(found)
                "#,
            )
            .unwrap();
    }

    #[test]
    fn input_method_bridges_context_and_edits() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "input-method.lua",
                br#"
                    local active = mold.signal("input.active", false)
                    mold.input_method.subscribe(function(value) active:set(value) end)
                    mold.input_method.preedit("hel", 3, 3)
                    mold.input_method.commit("hello")
                    mold.input_method.delete(1, 2)
                    mold.ipc["input.active"] = function() return active:get() end
                "#,
            )
            .unwrap();

        assert!(runtime.take_input_method_enable_request());
        assert_eq!(
            runtime.take_input_method_requests(),
            [
                InputMethodRequest::Preedit {
                    text: "hel".to_owned(),
                    begin: 3,
                    end: 3,
                },
                InputMethodRequest::Commit("hello".to_owned()),
                InputMethodRequest::Delete {
                    before: 1,
                    after: 2,
                },
            ]
        );
        assert!(runtime.dispatch_input_method(true, Some("hello".to_owned()), 5, 5, 1));
        assert_eq!(
            runtime.call_ipc("input.active", &[]).unwrap(),
            [IpcValue::Boolean(true)]
        );
    }

    #[test]
    fn text_input_bridges_state_and_edit_batches() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "text-input.lua",
                br#"
                    local committed = mold.signal("text.committed", "")
                    mold.text_input.subscribe(function(_, _, _, _, text)
                        if text then committed:set(text) end
                    end)
                    mold.text_input.surrounding("draft", 5, 5)
                    mold.text_input.content_type(3, 0)
                    mold.text_input.cursor_rect(10, 20, 2, 18)
                    mold.ipc["text.get"] = function() return committed:get() end
                "#,
            )
            .unwrap();

        assert!(runtime.take_text_input_enable_request());
        assert_eq!(runtime.take_text_input_requests().len(), 3);
        assert!(runtime.dispatch_text_input(true, None, 0, 0, Some("done".to_owned()), 0, 0, 1,));
        assert_eq!(
            runtime.call_ipc("text.get", &[]).unwrap(),
            [IpcValue::String("done".to_owned())]
        );
    }

    #[test]
    fn variants_builds_the_current_screen_instance() {
        let mut runtime = Runtime::for_screen(
            Limits::default(),
            Screen {
                name: "DP-1".to_owned(),
                width: Some(1920),
                height: Some(1080),
                scale: 2,
            },
        );
        runtime
            .execute(
                "variants.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    mold.variants(mold.screens, function(screen)
                        return ui.Text { text = screen.name, width = screen.width }
                    end)
                "#,
            )
            .unwrap();
        let node = runtime.scene().roots()[0];
        assert_eq!(runtime.scene().string_value(node, "text").unwrap(), "DP-1");
        assert_eq!(runtime.scene().number(node, "width").unwrap(), 1920.0);
    }

    #[test]
    fn pure_lua_system_service_modules_load() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "services.lua",
                br#"
                    local UPower = require("patin.services.upower")
                    local Network = require("patin.services.network")
                    local Volume = require("patin.services.volume")
                    local Brightness = require("patin.services.brightness")
                    local Logind = require("patin.services.logind")
                    local Mpris = require("patin.services.mpris")
                    local Oxin = require("patin.services.oxin")
                    local BatteryIndicator = require("patin.indicators.battery")
                    local BrightnessIndicator = require("patin.indicators.brightness")
                    local CellularIndicator = require("patin.indicators.cellular")
                    local ClockIndicator = require("patin.indicators.clock")
                    local NetworkIndicator = require("patin.indicators.network")
                    local VolumeIndicator = require("patin.indicators.volume")
                    assert(type(UPower.new) == "function")
                    assert(type(Network.new) == "function")
                    assert(type(Volume.new) == "function")
                    assert(type(Brightness.new) == "function")
                    assert(type(Logind.new) == "function")
                    assert(type(Mpris.new) == "function")
                    assert(type(Oxin.new) == "function")
                    assert(type(mold.greetd.connect) == "function")
                    assert(type(mold.udev.subscribe) == "function")
                    mold.udev.subscribe("input", function(event)
                      assert(event.subsystem == "input")
                    end)
                    assert(type(BatteryIndicator) == "function")
                    assert(type(NetworkIndicator) == "function")
                    assert(type(VolumeIndicator) == "function")

                    BatteryIndicator {
                      service = { percentage = function() return 72 end },
                    }
                    NetworkIndicator {
                      service = {
                        networking_enabled = function() return true end,
                        wireless_enabled = function() return true end,
                      },
                    }
                    VolumeIndicator {
                      service = {
                        level = function() return 0.42 end,
                        muted = function() return false end,
                      },
                    }
                    BrightnessIndicator {
                      service = { level = function() return 0.65 end },
                    }
                    CellularIndicator {
                      service = { state = function() return 70 end },
                    }
                    ClockIndicator {}
                "#,
            )
            .unwrap();
        let roots = runtime.scene().roots();
        assert_eq!(
            runtime.scene().string_value(roots[0], "text").unwrap(),
            "72%"
        );
        assert_eq!(
            runtime.scene().string_value(roots[1], "text").unwrap(),
            "wifi"
        );
        assert_eq!(
            runtime.scene().string_value(roots[2], "text").unwrap(),
            "42%"
        );
        assert_eq!(
            runtime.scene().string_value(roots[3], "text").unwrap(),
            "65%"
        );
        assert_eq!(
            runtime.scene().string_value(roots[4], "text").unwrap(),
            "cell"
        );
    }

    #[test]
    fn lua_greetd_client_handles_authentication_prompts() {
        let path = std::env::temp_dir().join(format!("mold-greetd-{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).unwrap();
            let mut request = vec![0_u8; u32::from_ne_bytes(length) as usize];
            stream.read_exact(&mut request).unwrap();
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .contains("create_session")
            );
            let response = br#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#;
            stream
                .write_all(&(response.len() as u32).to_ne_bytes())
                .unwrap();
            stream.write_all(response).unwrap();
        });
        let mut runtime = Runtime::default();
        let source = format!(
            r#"
                local client = mold.greetd.connect({:?})
                local response = client:create_session("mold")
                assert(response.type == "auth_message")
                assert(response.auth_message_type == "secret")
                assert(response.auth_message == "Password:")
            "#,
            path.to_string_lossy()
        );

        runtime.execute("greetd.lua", source.as_bytes()).unwrap();
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pure_lua_patin_builds_complete_phone_shell() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "phone.lua",
                br#"
                    local mold = require("mold")
                    local patin = require("patin")
                    local apps = {}
                    for index = 1, 500 do apps[index] = "App " .. index end
                    patin.shells.Phone {
                        width = 720,
                        height = 1280,
                        apps = mold.list_model(apps),
                        notifications = mold.list_model({ "Ready" }),
                        launcher_visible = true,
                        locked = false,
                    }
                "#,
            )
            .unwrap();
        let scene = runtime.scene();
        let root = scene.roots()[0];

        assert_eq!(scene.number(root, "width").unwrap(), 720.0);
        assert_eq!(scene.children(root).unwrap().len(), 6);
        assert!(scene.roots().len() == 1);
    }

    #[test]
    fn patin_lock_routes_keys_through_native_pam() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "lock.lua",
                br#"
                    local Lock = require("patin.shells.lock")
                    Lock { pam_service = "mold\0test", username = "user" }
                "#,
            )
            .unwrap();
        let target = runtime.first_key_target().unwrap();
        assert!(runtime.dispatch_key_event(target, 65, Some("a")));
        assert!(runtime.dispatch_key_event(target, 65293, None));
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !runtime.poll_services() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(!runtime.take_session_unlock_request());
    }

    #[test]
    fn reports_syntax_errors_with_the_source_name() {
        let mut runtime = Runtime::default();
        let error = runtime.execute("broken.lua", b"local =").unwrap_err();
        assert!(matches!(error, Error::Load(_)));
        assert!(error.to_string().contains("broken.lua"));
    }

    #[test]
    fn stops_an_infinite_loop_on_fuel_exhaustion() {
        let limits = Limits {
            fuel: 2_000,
            slice_fuel: 128,
            ..Limits::default()
        };
        let mut runtime = Runtime::new(limits);
        let error = runtime
            .execute("loop.lua", b"while true do end")
            .unwrap_err();
        assert_eq!(error, Error::FuelExhausted { budget: 2_000 });
    }

    #[test]
    fn lua_signal_change_reruns_exactly_one_effect() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "reactive.lua",
                br#"
                    local mold = require("mold")
                    local source = mold.signal("source", 1)
                    local other = mold.signal("other", 2)
                    local source_runs = 0
                    local other_runs = 0
                    assert(mold.effect("source effect", function()
                        source:get()
                        source_runs = source_runs + 1
                    end))
                    assert(mold.effect("other effect", function()
                        other:get()
                        other_runs = other_runs + 1
                    end))
                    source_runs = 0
                    other_runs = 0
                    local ok, err = source:set(7)
                    assert(ok, err)
                    assert(source_runs == 1)
                    assert(other_runs == 0)
                "#,
            )
            .unwrap();
    }

    #[test]
    fn binding_dependencies_ignore_settled_signals() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "dependencies.lua",
                br#"
                    local mold = require("mold")
                    local source = mold.signal("source", 1)
                    assert(mold.effect("source binding", function()
                        source:get()
                    end))
                "#,
            )
            .unwrap();

        assert!(runtime.binding_dependencies().is_empty());
    }

    #[test]
    fn binding_dependencies_flag_animated_property_reads() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "animated-dependency.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local width = mold.signal("width", 0)
                    local source = ui.Rect {
                        behavior = {
                            width = { duration = 200, easing = "linear" },
                        },
                        width = function() return width:get() end,
                    }
                    ui.Item {
                        height = function() return source.width end,
                        x = function() return source.width_target end,
                    }
                    local ok, err = width:set(100)
                    assert(ok, err)
                "#,
            )
            .unwrap();

        let diagnostics = runtime.binding_dependencies();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains(".height <- "));
        assert!(diagnostics[0].contains(".width (1 evaluation/frame)"));
        let runs = runtime.effect_runs();

        runtime.tick_animations(Duration::from_millis(100)).unwrap();

        assert_eq!(runtime.effect_runs(), runs + 1);
    }

    #[test]
    fn lua_binding_loop_names_the_property_chain() {
        let mut runtime = Runtime::new(Limits {
            effect_fuel: 10_000,
            frame_fuel: 100_000,
            ..Limits::default()
        });
        runtime
            .execute(
                "loop.lua",
                br#"
                    local mold = require("mold")
                    local left = mold.signal("left", 0)
                    local right = mold.signal("right", 0)
                    assert(mold.effect("left binding", function()
                        left:set(right:get() + 1)
                    end))
                    local ok, err = mold.effect("right binding", function()
                        right:set(left:get() + 1)
                    end)
                    assert(not ok, "loop unexpectedly succeeded")
                    assert(string.find(err, "left binding", 1, true), err)
                    assert(string.find(err, "right binding", 1, true), err)
                    assert(string.find(err, "left", 1, true), err)
                    assert(string.find(err, "right", 1, true), err)
                "#,
            )
            .unwrap();
    }

    #[test]
    fn runaway_lua_effect_exhausts_its_own_fuel() {
        let mut runtime = Runtime::new(Limits {
            effect_fuel: 1_000,
            frame_fuel: 10_000,
            slice_fuel: 64,
            ..Limits::default()
        });
        runtime
            .execute(
                "effect-fuel.lua",
                br#"
                    local mold = require("mold")
                    local ok, err = mold.effect("runaway", function()
                        while true do end
                    end)
                    assert(not ok)
                    assert(string.find(err, "effect fuel exhausted", 1, true))
                "#,
            )
            .unwrap();
        assert!(runtime.take_logs()[0].contains("runaway"));
    }

    #[test]
    fn lua_builds_a_scene_tree_with_bound_properties() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "scene.lua",
                br##"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local clock = mold.signal("clock", "12:00")
                    ui.Row {
                        spacing = 6,
                        ui.Text {
                            text = function() return clock:get() end,
                            color = "#ffffff",
                        },
                        ui.Rect {
                            width = 20,
                            height = 10,
                            color = "#7c3aed",
                        },
                    }
                    local ok, err = clock:set("12:01")
                    assert(ok, err)
                "##,
            )
            .unwrap();

        let scene = runtime.scene();
        let roots = scene.roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(scene.element(roots[0]).unwrap(), Element::Row);
        let children = scene.children(roots[0]).unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(
            scene.current(children[0], "text").unwrap(),
            &SceneValue::String("12:01".to_owned())
        );
        assert_eq!(scene.number(children[1], "width").unwrap(), 20.0);
    }

    #[test]
    fn lua_constructs_image_icon_and_shape_elements() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "images.lua",
                br#"
                    local ui = require("mold.ui")
                    ui.Item {
                        ui.Image { source = "/tmp/picture.png", width = 64, height = 32 },
                        ui.Icon { name = "battery", theme = "hicolor", width = 24, height = 24 },
                        ui.Shape {
                          path = "M0 0 L16 0 L8 16 Z",
                          fill_color = "white",
                          stroke_width = 1,
                        },
                    }
                "#,
            )
            .unwrap();

        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();
        assert_eq!(
            runtime.scene().element(children[0]).unwrap(),
            Element::Image
        );
        assert_eq!(runtime.scene().element(children[1]).unwrap(), Element::Icon);
        assert_eq!(
            runtime.scene().element(children[2]).unwrap(),
            Element::Shape
        );
    }

    #[test]
    fn clock_service_recomputes_text_bindings() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "clock.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    ui.Text { text = function() return mold.clock:get() end }
                "#,
            )
            .unwrap();
        runtime.update_clock("12:34:56").unwrap();

        let node = runtime.scene().roots()[0];
        assert_eq!(
            runtime.scene().string_value(node, "text").unwrap(),
            "12:34:56"
        );
    }

    #[test]
    fn component_mouse_area_emits_clicked() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "button.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local count = mold.signal("count", 0)
                    local Button = ui.component(function(props)
                        return ui.MouseArea {
                            width = 80,
                            height = 24,
                            accepted_buttons = { "right", 274 },
                            on_clicked = props.on_clicked,
                        }
                    end)
                    ui.Item {
                        ui.Text { text = function() return "" .. count:get() end },
                        Button { on_clicked = function() count:set(count:get() + 1) end },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();

        assert!(!runtime.accepts_pointer_button(children[1], 0x110));
        assert!(runtime.accepts_pointer_button(children[1], 0x111));
        assert!(runtime.accepts_pointer_button(children[1], 0x112));
        assert!(runtime.dispatch_ui_event(children[1], UiEvent::Clicked));

        assert_eq!(
            runtime.scene().string_value(children[0], "text").unwrap(),
            "1"
        );
    }

    #[test]
    fn key_handlers_receive_keysym_and_text() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "key.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local value = mold.signal("key", "")
                    ui.Item {
                        ui.MouseArea {
                            width = 100,
                            height = 40,
                            on_key_pressed = function(keysym, text)
                                value:set(keysym .. ":" .. text)
                            end,
                        },
                        ui.Text { text = function() return value:get() end },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();

        assert!(runtime.dispatch_key_event(children[0], 65, Some("A")));
        assert_eq!(
            runtime.scene().string_value(children[1], "text").unwrap(),
            "65:A"
        );
    }

    #[test]
    fn keyboard_focus_routes_ancestors_and_cycles() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "focus.lua",
                br#"
                    local ui = require("mold.ui")
                    ui.Item {
                      ui.MouseArea {
                        ui.Rect {},
                        on_key_pressed = function() end,
                      },
                      ui.MouseArea {
                        focus = true,
                        on_key_pressed = function() end,
                      },
                      ui.MouseArea {
                        enabled = false,
                        on_key_pressed = function() end,
                      },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();
        let nested = runtime.scene().children(children[0]).unwrap()[0];

        assert_eq!(runtime.first_key_target(), Some(children[1]));
        assert_eq!(runtime.key_target_for_node(nested), Some(children[0]));
        assert_eq!(
            runtime.next_key_target(Some(children[1])),
            Some(children[0])
        );
    }

    #[test]
    fn pam_callbacks_return_asynchronously_to_lua() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "pam.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local result = mold.signal("pam.result", "pending")
                    mold.pam.authenticate("mold\0test", "user", "secret", function(ok, error)
                        result:set(ok and "ok" or error)
                    end)
                    ui.Text { text = function() return result:get() end }
                "#,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !runtime.poll_services() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let root = runtime.scene().roots()[0];

        assert_eq!(
            runtime.scene().string_value(root, "text").unwrap(),
            "service contains a null byte"
        );
    }

    #[test]
    fn failed_pam_authentication_cannot_request_unlock() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "unlock.lua",
                br#"
                    local mold = require("mold")
                    mold.pam.authenticate_unlock("mold\0test", "user", "secret", function() end)
                "#,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !runtime.poll_services() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(!runtime.take_session_unlock_request());
    }

    #[test]
    fn native_timer_callbacks_recompute_lua_bindings() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "timer.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local count = mold.signal("timer.count", 0)
                    mold.timer(1, function() count:set(count:get() + 1) end, false)
                    ui.Text { text = function() return "" .. count:get() end }
                "#,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !runtime.poll_services() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let root = runtime.scene().roots()[0];

        assert_eq!(runtime.scene().string_value(root, "text").unwrap(), "1");
    }

    #[test]
    fn loader_and_timer_build_native_scene_objects() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "scene-objects.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local count = mold.signal("scene.timer.count", 0)
                    ui.Item {
                      ui.Loader {
                        source = function() return ui.Text { text = "loaded" } end,
                      },
                      ui.Timer {
                        interval = 1,
                        running = true,
                        on_triggered = function() count:set(count:get() + 1) end,
                      },
                      ui.Text { text = function() return "" .. count:get() end },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();
        let loader_children = runtime.scene().children(children[0]).unwrap();

        assert_eq!(
            runtime.scene().element(children[0]).unwrap(),
            Element::Loader
        );
        assert_eq!(
            runtime.scene().element(children[1]).unwrap(),
            Element::Timer
        );
        assert_eq!(
            runtime
                .scene()
                .string_value(loader_children[0], "text")
                .unwrap(),
            "loaded"
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !runtime.poll_services() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(
            runtime.scene().string_value(children[2], "text").unwrap(),
            "1"
        );
        assert!(!runtime.scene().bool_value(children[1], "running").unwrap());
    }

    #[test]
    fn loader_and_timer_follow_dynamic_properties() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "dynamic-loader-timer.lua",
                br#"
                    local ui = require("mold.ui")
                    local active = mold.signal("loader.active", false)
                    local running = mold.signal("timer.running", false)
                    local loader = ui.Loader {
                      active = function() return active:get() end,
                      source = function() return ui.Text { text = "loaded" } end,
                    }
                    local timer = ui.Timer {
                      interval = 1,
                      ["repeat"] = false,
                      running = function() return running:get() end,
                      on_triggered = function() end,
                    }
                    mold.ipc["dynamic.start"] = function()
                      active:set(true)
                      running:set(true)
                    end
                    mold.ipc["dynamic.stop"] = function() active:set(false) end
                    ui.Item { loader, timer }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();
        let loader = children[0];
        let timer = children[1];
        assert!(runtime.scene().children(loader).unwrap().is_empty());

        runtime.call_ipc("dynamic.start", &[]).unwrap();
        assert!(runtime.poll_services());
        assert_eq!(runtime.scene().children(loader).unwrap().len(), 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while runtime.scene().bool_value(timer, "running").unwrap()
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
            runtime.poll_services();
        }
        assert!(!runtime.scene().bool_value(timer, "running").unwrap());

        runtime.call_ipc("dynamic.stop", &[]).unwrap();
        assert!(runtime.poll_services());
        assert!(runtime.scene().children(loader).unwrap().is_empty());
    }

    #[test]
    fn lua_io_primitives_stream_processes_and_bound_files() {
        let path = std::env::temp_dir().join(format!("mold-lua-file-{}", std::process::id()));
        std::fs::write(&path, "old").unwrap();
        let source = format!(
            r#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local output = mold.signal("process.output", "pending")
                local file = mold.file("{}")
                assert(file:read() == "old")
                file:write("new")
                assert(file:read() == "new")
                local process = mold.process("sh", {{ "-c", "printf streamed" }})
                mold.timer(1, function()
                    local event = process:next()
                    if event and event.kind == "stdout" then output:set(event.data) end
                end)
                ui.Text {{ text = function() return output:get() end }}
            "#,
            path.display()
        );
        let mut runtime = Runtime::default();
        runtime.execute("io.lua", source.as_bytes()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let root = runtime.scene().roots()[0];
        while runtime.scene().string_value(root, "text").unwrap() != "streamed"
            && std::time::Instant::now() < deadline
        {
            runtime.poll_services();
            std::thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(
            runtime.scene().string_value(root, "text").unwrap(),
            "streamed"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn lua_socket_uses_bounded_timeout_reads() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        let path = std::env::temp_dir().join(format!("mold-lua-socket-{}", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").unwrap();
        });
        let source = format!(
            r#"
                local mold = require("mold")
                local socket = mold.socket("{}")
                socket:send("ping")
                assert(socket:receive(4, 500) == "pong")
            "#,
            path.display()
        );
        let mut runtime = Runtime::default();

        runtime.execute("socket.lua", source.as_bytes()).unwrap();

        server.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn lua_exposes_stream_parsers_and_socket_servers() {
        let path = std::env::temp_dir().join(format!("mold-lua-server-{}", std::process::id()));
        let source = format!(
            r#"
                local mold = require("mold")
                local lines = mold.line_parser()
                local first = lines:push("one\ntw")
                assert(#first == 1 and first[1] == "one")
                local second = lines:push("o\r\nlast")
                assert(#second == 1 and second[1] == "two")
                assert(lines:finish() == "last")

                local split = mold.split_parser("--")
                local parts = split:push("a-b--c--tail")
                assert(#parts == 2 and parts[1] == "a-b" and parts[2] == "c")
                assert(split:finish() == "tail")

                local server = mold.socket_server("{}")
                assert(server:accept() == nil)
            "#,
            path.display()
        );
        let mut runtime = Runtime::default();

        runtime
            .execute("io-surfaces.lua", source.as_bytes())
            .unwrap();

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn lua_dbus_arguments_preserve_positional_lists() {
        let mut runtime = Runtime::default();
        let value = runtime.lua.enter(|ctx| {
            let arguments = Table::new(&ctx);
            arguments.set(ctx, 1, "device").unwrap();
            let typed = Table::new(&ctx);
            typed.set_field(ctx, "signature", "u");
            typed.set_field(ctx, "value", 7_i64);
            arguments.set(ctx, 2, typed).unwrap();
            arguments.set(ctx, 3, true).unwrap();
            lua_to_dbus(ctx, LuaValue::Table(arguments), 0)
        });

        assert_eq!(
            value.unwrap(),
            DbusValue::List(vec![
                DbusValue::String("device".to_owned()),
                DbusValue::Typed {
                    signature: "u".to_owned(),
                    value: Box::new(DbusValue::Integer(7)),
                },
                DbusValue::Bool(true),
            ])
        );
    }

    #[test]
    fn handler_fuel_failure_is_nonfatal() {
        let mut runtime = Runtime::new(Limits {
            effect_fuel: 1_000,
            slice_fuel: 64,
            ..Limits::default()
        });
        runtime
            .execute(
                "handler.lua",
                br#"
                    local ui = require("mold.ui")
                    ui.MouseArea { on_clicked = function() while true do end end }
                "#,
            )
            .unwrap();
        let node = runtime.scene().roots()[0];

        assert!(runtime.dispatch_ui_event(node, UiEvent::Clicked));
        assert!(runtime.take_logs()[0].contains("handler fuel exhausted"));
        assert!(runtime.scene().contains(node));
    }

    #[test]
    fn touch_handlers_receive_contact_identity_and_coordinates() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "touch.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local status = mold.signal("touch.status", "idle")
                    ui.MouseArea {
                      width = 100,
                      height = 100,
                      on_touch_pressed = function(id, x, y)
                        status:set(string.format("down:%d:%.0f:%.0f", id, x, y))
                      end,
                      on_touch_moved = function(id, x, y)
                        status:set(string.format("move:%d:%.0f:%.0f", id, x, y))
                      end,
                      on_touch_released = function(id)
                        status:set("up:" .. id)
                      end,
                      ui.Text { text = function() return status:get() end },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let text = runtime.scene().children(root).unwrap()[0];

        assert!(runtime.dispatch_touch_event(root, UiEvent::TouchPressed, 7, 12.0, 18.0));
        assert_eq!(
            runtime.scene().string_value(text, "text").unwrap(),
            "down:7:12:18"
        );
        assert!(runtime.dispatch_touch_event(root, UiEvent::TouchMoved, 7, 20.0, 30.0));
        assert_eq!(
            runtime.scene().string_value(text, "text").unwrap(),
            "move:7:20:30"
        );
        assert!(runtime.dispatch_touch_event(root, UiEvent::TouchReleased, 7, 20.0, 30.0));
        assert_eq!(runtime.scene().string_value(text, "text").unwrap(), "up:7");
    }

    #[test]
    fn pointer_drag_handlers_receive_position_and_displacement() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "drag.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local status = mold.signal("drag.status", "idle")
                    ui.MouseArea {
                      accepted_buttons = { "right" },
                      on_dragged = function(x, y, dx, dy)
                        status:set(string.format("%.0f:%.0f:%.0f:%.0f", x, y, dx, dy))
                      end,
                      ui.Text { text = function() return status:get() end },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let text = runtime.scene().children(root).unwrap()[0];

        assert!(!runtime.accepts_pointer_button(root, 0x110));
        assert!(runtime.accepts_pointer_button(root, 0x111));
        assert!(runtime.dispatch_pointer_event(root, UiEvent::Dragged, 20.0, 30.0, 9.0, 12.0));
        assert_eq!(
            runtime.scene().string_value(text, "text").unwrap(),
            "20:30:9:12"
        );
    }

    #[test]
    fn pure_lua_button_accepts_binding_and_emits_clicked() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "patin-button.lua",
                br#"
                    local mold = require("mold")
                    local Button = require("patin.widgets.button")
                    local count = mold.signal("count", 0)
                    Button {
                        text = function() return "Clicks " .. count:get() end,
                        on_clicked = function() count:set(count:get() + 1) end,
                    }
                "#,
            )
            .unwrap();
        let button = runtime.scene().roots()[0];
        let children = runtime.scene().children(button).unwrap();
        let rect_children = runtime.scene().children(children[0]).unwrap();

        assert!(runtime.dispatch_ui_event(children[1], UiEvent::Clicked));

        assert_eq!(
            runtime
                .scene()
                .string_value(rect_children[0], "text")
                .unwrap(),
            "Clicks 1"
        );
    }

    #[test]
    fn declared_components_validate_properties_and_default_slots() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "declared-component.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local count = mold.signal("component.count", 0)
                    local Label = ui.component {
                      name = "Label",
                      properties = {
                        text = { type = "string", default = "empty" },
                        content = { type = "table", default = {} },
                      },
                      signals = { "activated" },
                      default_slot = "content",
                      build = function(self)
                        return ui.Item {
                          ui.Text { text = self:binding("text") },
                          table.unpack(self.content),
                          ui.MouseArea { on_clicked = function() self:emit("activated") end },
                        }
                      end,
                    }
                    Label {
                      text = function() return "Count " .. count:get() end,
                      on_activated = function() count:set(count:get() + 1) end,
                      ui.Rect { width = 4, height = 4 },
                    }
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();

        assert_eq!(runtime.scene().element(children[1]).unwrap(), Element::Rect);
        assert!(runtime.dispatch_ui_event(children[2], UiEvent::Clicked));
        assert_eq!(
            runtime.scene().string_value(children[0], "text").unwrap(),
            "Count 1"
        );

        let error = runtime
            .execute(
                "bad-component.lua",
                br#"
                    local ui = require("mold.ui")
                    local Typed = ui.component {
                      name = "Typed",
                      properties = { count = { type = "number", default = 0 } },
                      build = function() return ui.Item {} end,
                    }
                    Typed { count = "wrong" }
                "#,
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Typed property `count` expects number")
        );
    }

    #[test]
    fn lua_scene_errors_name_unknown_properties() {
        let mut runtime = Runtime::default();
        let error = runtime
            .execute(
                "bad-scene.lua",
                br#"
                    local ui = require("mold.ui")
                    ui.Text { radius = 4 }
                "#,
            )
            .unwrap_err();

        assert!(error.to_string().contains("unknown Text property `radius`"));
    }

    #[test]
    fn lua_binding_glides_without_lua_on_animation_ticks() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "behavior.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local expanded = mold.signal("expanded", false)
                    ui.Rect {
                        behavior = {
                            width = { duration = 200, easing = "linear" },
                        },
                        width = function()
                            return expanded:get() and 100 or 0
                        end,
                    }
                    local ok, err = expanded:set(true)
                    assert(ok, err)
                "#,
            )
            .unwrap();
        let node = runtime.scene().roots()[0];
        assert_eq!(runtime.scene().number(node, "width").unwrap(), 0.0);
        assert_eq!(
            runtime.scene().target(node, "width").unwrap(),
            &SceneValue::Number(100.0)
        );
        let runs = runtime.effect_runs();

        let frame = runtime.tick_animations(Duration::from_millis(100)).unwrap();

        assert_eq!(runtime.scene().number(node, "width").unwrap(), 50.0);
        assert_eq!(runtime.effect_runs(), runs);
        assert!(frame.active);
    }

    #[test]
    fn lua_spring_chases_a_reactive_target_in_rust() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "spring.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local target = mold.signal("target", 0)
                    ui.Item {
                        behavior = {
                            x = ui.spring { damping = 18, stiffness = 180 },
                        },
                        x = function() return target:get() end,
                    }
                    local ok, err = target:set(100)
                    assert(ok, err)
                "#,
            )
            .unwrap();
        let node = runtime.scene().roots()[0];
        let runs = runtime.effect_runs();

        let frame = runtime.tick_animations(Duration::from_millis(50)).unwrap();

        assert!(runtime.scene().number(node, "x").unwrap() > 0.0);
        assert!(runtime.scene().number(node, "x").unwrap() < 100.0);
        assert_eq!(runtime.effect_runs(), runs);
        assert!(frame.active);
    }

    #[test]
    fn lua_list_model_virtualizes_five_hundred_items() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "list.lua",
                br#"
                    local mold = require("mold")
                    local items = {}
                    for index = 1, 500 do items[index] = { name = "app" .. index } end
                    local model = mold.list_model(items)
                    local view = mold.virtual_list(model, 40, 400, 1)
                    local initial = view:sync()
                    assert(#initial == 12)
                    assert(initial[1].kind == "populate")
                    assert(#view:visible() == 12)
                    model:move(3, 8)
                    local changes = view:sync()
                    local moved = false
                    local displaced = false
                    for _, change in ipairs(changes) do
                      moved = moved or change.kind == "move"
                      displaced = displaced or change.kind == "displaced"
                    end
                    assert(moved and displaced)
                    view:set_offset(4000)
                    assert(view:visible()[1].index == 100)
                "#,
            )
            .unwrap();
    }

    #[test]
    fn list_view_builds_only_visible_lua_delegates() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "list-view.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local items = {}
                    for index = 1, 500 do items[index] = "app" .. index end
                    local model = mold.list_model(items)
                    local view = ui.ListView {
                        model = model,
                        height = 400,
                        item_extent = 40,
                        overscan = 1,
                        content_y = 4000,
                        delegate = function(item, index)
                            return ui.Text { text = item, width = 100, height = 40 }
                        end,
                    }
                    model:set(100, "changed")
                    mold.sync_view(view, 4000)
                    mold.sync_view(view, 8000)
                "#,
            )
            .unwrap();
        let scene = runtime.scene();
        let root = scene.roots()[0];
        let children = scene.children(root).unwrap();

        assert_eq!(children.len(), 13);
        assert_eq!(scene.string_value(children[0], "text").unwrap(), "app200");
        assert_eq!(scene.number(children[0], "y").unwrap(), -40.0);
        assert!(scene.bool_value(root, "clip").unwrap());
    }

    #[test]
    fn grid_view_virtualizes_complete_rows_in_rust() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "grid-view.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local items = {}
                    for index = 1, 500 do items[index] = "tile" .. index end
                    ui.GridView {
                      model = mold.list_model(items),
                      width = 400,
                      height = 200,
                      cell_width = 100,
                      cell_height = 50,
                      columns = 4,
                      overscan = 1,
                      content_y = 75,
                      delegate = function(item)
                        return ui.Text { text = item, width = 100, height = 50 }
                      end,
                    }
                "#,
            )
            .unwrap();
        let scene = runtime.scene();
        let root = scene.roots()[0];
        let children = scene.children(root).unwrap();

        assert_eq!(children.len(), 28);
        assert_eq!(scene.string_value(children[5], "text").unwrap(), "tile6");
        assert_eq!(scene.number(children[5], "x").unwrap(), 100.0);
        assert_eq!(scene.number(children[5], "y").unwrap(), -25.0);
        assert!(scene.bool_value(root, "clip").unwrap());
    }

    #[test]
    fn repeater_builds_one_delegate_per_model_entry() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "repeater.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local model = mold.list_model({ "one", "two", "three" })
                    ui.Repeater {
                        model = model,
                        delegate = function(item) return ui.Text { text = item } end,
                    }
                "#,
            )
            .unwrap();
        let scene = runtime.scene();
        let children = scene.children(scene.roots()[0]).unwrap();

        assert_eq!(children.len(), 3);
        assert_eq!(scene.string_value(children[2], "text").unwrap(), "three");
    }

    #[test]
    fn flickable_state_drags_and_ticks_in_rust() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "flickable.lua",
                br#"
                    local mold = require("mold")
                    local flick = mold.flickable {
                        offset = 100,
                        minimum = 0,
                        maximum = 500,
                        deceleration = 100,
                    }
                    assert(flick:drag_by(25) == 125)
                    flick:release(200)
                    local offset, active = flick:tick(100)
                    assert(offset == 145 and active)
                "#,
            )
            .unwrap();
    }

    #[test]
    fn lua_queues_parent_and_anchor_transition() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "parent.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local tile = ui.Rect { width = 20, height = 20 }
                    local left = ui.Item { x = 10, width = 100, height = 100, tile }
                    local right = ui.Item { x = 200, width = 100, height = 100 }
                    ui.Item { left, right }
                    mold.transition_parent(tile, right, {
                      duration = 300,
                      easing = "out_cubic",
                      anchors = { center_in = true },
                    })
                "#,
            )
            .unwrap();

        let transitions = runtime.take_parent_transitions();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].behavior.duration, Duration::from_millis(300));
        assert_eq!(transitions[0].behavior.easing, Easing::OutCubic);
        assert_eq!(
            transitions[0].anchors.as_ref().unwrap().get("center_in"),
            Some(&SceneValue::Bool(true))
        );
    }

    #[test]
    fn lua_named_state_animates_properties_and_queues_reparent() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "states.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local expanded = mold.signal("expanded", false)
                    local shelf = ui.Item { width = 100, height = 100 }
                    local page = ui.Item { x = 200, width = 200, height = 100 }
                    local tile = ui.Rect {
                      states = {
                        compact = {
                          property_changes = { width = 40, height = 40 },
                          parent_change = shelf,
                        },
                        expanded = {
                          property_changes = { width = 180, height = 80 },
                          anchor_changes = { center_in = true },
                          parent_change = page,
                        },
                      },
                      transitions = {
                        {
                          from = "compact",
                          to = "expanded",
                          reversible = true,
                          duration = 200,
                          easing = "linear",
                        },
                      },
                      state = function()
                        return expanded:get() and "expanded" or "compact"
                      end,
                    }
                    ui.Item { shelf, page }
                    local ok, err = expanded:set(true)
                    assert(ok, err)
                "#,
            )
            .unwrap();
        let root = runtime.scene().roots()[0];
        let children = runtime.scene().children(root).unwrap();
        let tile = runtime.scene().children(children[0]).unwrap()[0];

        assert_eq!(runtime.scene().number(tile, "width").unwrap(), 40.0);
        assert_eq!(
            runtime.scene().target(tile, "width").unwrap(),
            &SceneValue::Number(180.0)
        );
        let transitions = runtime.take_parent_transitions();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].parent, children[1]);
        runtime.tick_animations(Duration::from_millis(100)).unwrap();
        assert_eq!(runtime.scene().number(tile, "width").unwrap(), 110.0);
    }

    #[test]
    fn state_property_bindings_recapture_dependencies() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "state-binding.lua",
                br#"
                    local mold = require("mold")
                    local ui = require("mold.ui")
                    local size = mold.signal("size", 40)
                    ui.Rect {
                      states = {
                        active = {
                          property_changes = {
                            width = function() return size:get() end,
                          },
                        },
                      },
                      state = function() return "active" end,
                    }
                    local ok, err = size:set(80)
                    assert(ok, err)
                "#,
            )
            .unwrap();
        let node = runtime.scene().roots()[0];
        assert_eq!(runtime.scene().number(node, "width").unwrap(), 80.0);
    }
}
