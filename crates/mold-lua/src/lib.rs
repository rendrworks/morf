//! Sandboxed execution of mold configuration code.

use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use luna::{
    Callback, CallbackReturn, Closure, Context, Executor, ExecutorMode, Fuel, Function, Lua,
    StashedClosure, Table, UserData, UserRef, Value as LuaValue,
};
use mold_io::{Bus, DbusProxy, DbusValue};
use mold_reactive::{EffectContext, Graph, SignalId};
use mold_scene::{
    AnimationFrame, Behavior, Easing, Element, ListModel, NodeHandle, Physics, Scene,
    Value as SceneValue, ViewTransition, VirtualList,
};
use mold_services::PipeWire;

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
        install_reactive_api(&mut lua, Rc::clone(&reactive), limits, screen.as_ref());
        Self {
            lua,
            limits,
            reactive,
        }
    }

    /// Compiles and executes a Lua chunk.
    pub fn execute(&mut self, name: &str, source: &[u8]) -> Result<(), Error> {
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
        let handler = self.reactive.borrow().handlers.get(&(node, event)).cloned();
        let Some(handler) = handler else {
            return false;
        };
        let result = self
            .lua
            .enter(|ctx| execute_handler(ctx, &handler, self.limits));
        if let Err(message) = result {
            self.reactive.borrow_mut().logs.push(format!(
                "{:?}.{}: {message}",
                node,
                event.property()
            ));
        }
        true
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

struct ListModelToken {
    model: Rc<RefCell<ListModel>>,
}

struct VirtualListToken {
    model: Rc<RefCell<ListModel>>,
    view: RefCell<VirtualList>,
}

#[derive(Clone)]
struct LuaEffect {
    closure: StashedClosure,
    sink: Option<PropertySink>,
}

#[derive(Clone)]
struct PropertySink {
    node: NodeHandle,
    property: String,
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

        let ui = Table::new(&ctx);
        for (name, element) in [
            ("Item", Element::Item),
            ("Rect", Element::Rect),
            ("Text", Element::Text),
            ("MouseArea", Element::MouseArea),
            ("Row", Element::Row),
            ("Column", Element::Column),
        ] {
            ui.set_field(
                ctx,
                name,
                element_constructor(ctx, Rc::clone(&state), limits, element),
            );
        }
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
                        return Err(HostError(format!("module `{name}` is not available")).into());
                    }
                }
                Ok(CallbackReturn::Return)
            }),
        );
    });
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
    for (property, value) in named {
        if property == "behavior" {
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
        let easing = match behavior.get_value(ctx, "easing") {
            LuaValue::Nil => Easing::Linear,
            LuaValue::String(value) => match value.display_lossy().to_string().as_str() {
                "linear" => Easing::Linear,
                "in_cubic" => Easing::InCubic,
                "out_cubic" => Easing::OutCubic,
                "in_out_cubic" => Easing::InOutCubic,
                name => return Err(format!("unknown easing `{name}`")),
            },
            _ => return Err("behavior easing must be a string".to_owned()),
        };
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
                sink: Some(PropertySink { node, property }),
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
    if let (Ok(Some(value)), Some(sink)) = (&result, lua_effect.sink) {
        state
            .borrow_mut()
            .scene
            .assign(sink.node, &sink.property, value.to_scene())
            .map_err(|error| error.to_string())?;
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

fn execute_handler(
    ctx: Context<'_>,
    closure: &StashedClosure,
    limits: Limits,
) -> Result<(), String> {
    let executor = Executor::start(ctx, ctx.fetch(closure).into(), ());
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
}
