//! Sandboxed execution of mold configuration code.

use std::cell::{Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet};
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
    Bus, DbusProxy, DbusValue, FileEvent, FileView, FileWatcher, Process, ProcessEvent, Socket,
    Timer as IoTimer,
};
use mold_reactive::{EffectContext, Graph, SignalId};
use mold_scene::{
    AnimationFrame, Behavior, Easing, Element, FlickState, ListChange, ListModel, ModelId,
    NodeHandle, Physics, Scene, Value as SceneValue, ViewTransition, VirtualList,
};
use mold_services::{PamAuthenticator, PamTask, PipeWire};

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

/// Event name accepted by Lua element handlers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UiEvent {
    /// Pointer entered the target.
    PointerEntered,
    /// Pointer left the target.
    PointerExited,
    /// Pointer button was pressed on the target.
    Pressed,
    /// Pointer button was released after pressing the target.
    Released,
    /// Pointer press and release completed on the same target.
    Clicked,
    /// A key was pressed while the target held focus.
    KeyPressed,
}

impl UiEvent {
    fn property(self) -> &'static str {
        match self {
            Self::PointerEntered => "on_entered",
            Self::PointerExited => "on_exited",
            Self::Pressed => "on_pressed",
            Self::Released => "on_released",
            Self::Clicked => "on_clicked",
            Self::KeyPressed => "on_key_pressed",
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

    /// Advances the pure-Rust animation driver.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, Error> {
        self.reactive
            .borrow_mut()
            .scene
            .tick_animations(delta)
            .map_err(|error| Error::Runtime(error.to_string()))
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

    /// Returns the first scene node with a key handler in tree order.
    pub fn first_key_target(&self) -> Option<NodeHandle> {
        let state = self.reactive.borrow();
        let mut pending = state.scene.roots();
        pending.reverse();
        while let Some(node) = pending.pop() {
            if state.handlers.contains_key(&(node, UiEvent::KeyPressed)) {
                return Some(node);
            }
            let mut children = state.scene.children(node).ok()?;
            children.reverse();
            pending.extend(children);
        }
        None
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
            let mut index = 0;
            while index < state.timers.len() {
                if state.timers[index].timer.tick(Duration::ZERO) {
                    timers.push(state.timers[index].callback.clone());
                    if !state.timers[index].repeat {
                        state.timers.swap_remove(index);
                        continue;
                    }
                }
                index += 1;
            }
        }
        let changed = !ready.is_empty() || !timers.is_empty();
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
    writes: Vec<(SignalId, ScriptValue)>,
}

struct ReactiveState {
    graph: Option<Graph<ScriptValue>>,
    values: HashMap<SignalId, ScriptValue>,
    signals: Vec<SignalId>,
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
    views: HashMap<NodeHandle, LuaVirtualView>,
    pam_tasks: Vec<PendingPam>,
    timers: Vec<PendingTimer>,
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
            views: HashMap::new(),
            pam_tasks: Vec::new(),
            timers: Vec::new(),
            session_unlock_requested: false,
        }
    }
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
        mold.set_field(ctx, "effect", effect);
        let clock = UserData::new_static(
            &ctx,
            SignalToken {
                id: state.borrow().clock,
            },
        );
        clock.set_metatable(ctx, Some(ctx.fetch(&signal_metatable)));
        mold.set_field(ctx, "clock", clock);
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
            timer_state.borrow_mut().timers.push(PendingTimer {
                timer,
                callback: ctx.stash(callback),
                repeat,
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

        let dbus_get = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, property): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
            let value = proxy.proxy.get_value(&property).map_err(HostError)?;
            stack.replace(ctx, dbus_value_to_lua(ctx, value));
            Ok(CallbackReturn::Return)
        });
        let dbus_call = Callback::from_fn(&ctx, |ctx, _, mut stack| {
            let (proxy, method): (UserRef<DbusToken>, String) = stack.consume(ctx)?;
            let value = proxy.proxy.call_value(&method).map_err(HostError)?;
            stack.replace(ctx, dbus_value_to_lua(ctx, value));
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

        let ui = Table::new(&ctx);
        for (name, element) in [
            ("Item", Element::Item),
            ("Rect", Element::Rect),
            ("Text", Element::Text),
            ("Image", Element::Image),
            ("Icon", Element::Icon),
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
            view_constructor(ctx, Rc::clone(&state), limits, false),
        );
        ui.set_field(
            ctx,
            "ListView",
            view_constructor(ctx, Rc::clone(&state), limits, true),
        );
        ui.set_field(
            ctx,
            "Flickable",
            element_constructor(ctx, Rc::clone(&state), limits, Element::Flickable),
        );
        ui.set_field(
            ctx,
            "component",
            Callback::from_fn(&ctx, |ctx, _, mut stack| {
                let factory: Closure = stack.consume(ctx)?;
                stack.replace(ctx, factory);
                Ok(CallbackReturn::Return)
            }),
        );
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

fn dbus_value_to_lua(ctx: Context<'_>, value: DbusValue) -> LuaValue<'_> {
    match value {
        DbusValue::Bool(value) => LuaValue::Boolean(value),
        DbusValue::Integer(value) => LuaValue::Integer(value),
        DbusValue::Unsigned(value) if value <= i64::MAX as u64 => LuaValue::Integer(value as i64),
        DbusValue::Unsigned(value) => LuaValue::Number(value as f64),
        DbusValue::Number(value) => LuaValue::Number(value),
        DbusValue::String(value) => LuaValue::String(ctx.intern(value.as_bytes())),
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
        let node = state.borrow_mut().scene.create(element);
        configure_element(&state, ctx, limits, node, properties).map_err(HostError)?;
        stack.replace(ctx, UserData::new_static(&ctx, NodeToken { handle: node }));
        Ok(CallbackReturn::Return)
    })
}

fn view_constructor<'gc>(
    ctx: Context<'gc>,
    state: Rc<RefCell<ReactiveState>>,
    limits: Limits,
    virtualized: bool,
) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _, mut stack| {
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
                        "model" | "delegate" | "item_extent" | "overscan" | "content_y"
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
        let node = state.borrow_mut().scene.create(Element::Item);
        configure_element(&state, ctx, limits, node, clean).map_err(HostError)?;
        let model_handle = Rc::clone(&model.model);
        let model = model_handle.borrow();
        let mut configured_view = None;
        let (range, item_extent, offset) = if virtualized {
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
            (range, item_extent, offset)
        } else {
            (0..model.len(), 0.0, 0.0)
        };
        let mut active = HashMap::new();
        for index in range {
            let (id, item) = model
                .get(index)
                .expect("view range contains live model indexes");
            let child = execute_delegate(ctx, &delegate, item, index, limits).map_err(HostError)?;
            if virtualized {
                state
                    .borrow_mut()
                    .scene
                    .assign(child, "y", index as f64 * item_extent - offset)
                    .map_err(|error| HostError(error.to_string()))?;
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
                },
            );
        }
        stack.replace(ctx, UserData::new_static(&ctx, NodeToken { handle: node }));
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
        state
            .borrow_mut()
            .scene
            .assign(node, "y", index as f64 * view.view.item_extent() - offset)
            .map_err(|error| error.to_string())?;
        state
            .borrow_mut()
            .scene
            .reparent(node, Some(parent))
            .map_err(|error| error.to_string())?;
        view.active.insert(id, node);
    }
    for (id, index, _) in visible {
        if let Some(node) = view.active.get(&id) {
            state
                .borrow_mut()
                .scene
                .assign(*node, "y", index as f64 * view.view.item_extent() - offset)
                .map_err(|error| error.to_string())?;
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
            state
                .borrow_mut()
                .scene
                .assign(node, &property, value)
                .map_err(|error| error.to_string())?;
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
        "on_pressed" => Some(UiEvent::Pressed),
        "on_released" => Some(UiEvent::Released),
        "on_clicked" => Some(UiEvent::Clicked),
        "on_key_pressed" => Some(UiEvent::KeyPressed),
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
            state
                .scene
                .animate_from(node, &property, from, value, transition.unwrap())
                .map_err(|error| error.to_string())?;
        } else {
            state
                .scene
                .assign(node, &property, value)
                .map_err(|error| error.to_string())?;
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
                    state
                        .scene
                        .assign(node, "anchors", SceneValue::Map(anchors))
                        .map_err(|error| error.to_string())?;
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
    let capture = state.borrow_mut().active.take().unwrap_or_default();
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
    if let (Ok(Some(value)), Some(sink)) = (&result, lua_effect.sink) {
        match sink {
            EffectSink::Property(sink) => state
                .borrow_mut()
                .scene
                .assign(sink.node, &sink.property, value.to_scene())
                .map_err(|error| error.to_string())?,
            EffectSink::State(_) => {}
        }
    }
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
                    local BatteryIndicator = require("patin.indicators.battery")
                    local NetworkIndicator = require("patin.indicators.network")
                    local VolumeIndicator = require("patin.indicators.volume")
                    assert(type(UPower.new) == "function")
                    assert(type(Network.new) == "function")
                    assert(type(Volume.new) == "function")
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
    fn lua_constructs_image_and_icon_elements() {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "images.lua",
                br#"
                    local ui = require("mold.ui")
                    ui.Item {
                        ui.Image { source = "/tmp/picture.png", width = 64, height = 32 },
                        ui.Icon { name = "battery", theme = "hicolor", width = 24, height = 24 },
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
