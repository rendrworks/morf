pub(crate) use crate::api_shader::RegisteredShader;
use crate::states::{Capture, StateSet};
use luna::{StashedClosure, StashedTable};
use morf_layout::{TransformTracker, TransformWatcher as NativeTransformWatcher};
use morf_lifecycle::Retention;
use morf_reactive::{Graph, SignalId};
use morf_scene::{GroupId, ListModel, ModelId, NodeHandle, Scene, VirtualList};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use crate::{
    events::*,
    surface_types::*,
    types::{LogEntry, LogLevel, ToplevelRequest},
};
// Re-exported, because these moved out of this file only to satisfy the line
// gate: every consumer reaches for them through `state::*` and there is no
// reason to make them all learn a second module name.
pub(crate) use crate::state_pending::*;
pub(crate) use crate::state_tokens::*;

pub(crate) struct LuaVirtualView {
    pub(crate) model: Rc<RefCell<ListModel>>,
    pub(crate) view: VirtualList,
    pub(crate) delegate: StashedClosure,
    pub(crate) active: HashMap<ModelId, DelegateInstance>,
    pub(crate) reusable: HashMap<ModelId, DelegateInstance>,
    pub(crate) reuse_order: VecDeque<ModelId>,
    pub(crate) reuse_limit: usize,
    pub(crate) pool_root: Option<NodeHandle>,
    pub(crate) column_extent: f64,
}

pub(crate) struct DelegateInstance {
    pub(crate) node: NodeHandle,
    pub(crate) updater: Option<StashedClosure>,
}

#[derive(Clone, Copy)]
pub(crate) enum ViewKind {
    Repeater,
    List,
    Grid,
}

pub(crate) struct LuaTransformWatcher {
    pub(crate) a: NodeHandle,
    pub(crate) b: NodeHandle,
    pub(crate) watcher: NativeTransformWatcher,
    pub(crate) callback: Option<StashedClosure>,
    pub(crate) revision: u64,
    pub(crate) pending: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PopupNodeAnchor {
    pub(crate) node: NodeHandle,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: Option<i32>,
    pub(crate) height: Option<i32>,
    pub(crate) margin_top: i32,
    pub(crate) margin_right: i32,
    pub(crate) margin_bottom: i32,
    pub(crate) margin_left: i32,
}

#[derive(Clone)]
pub(crate) struct LuaEffect {
    pub(crate) closure: StashedClosure,
    pub(crate) sink: Option<EffectSink>,
}

#[derive(Clone, Default)]
pub(crate) struct RetainCallbacks {
    pub(crate) dropped: Option<StashedClosure>,
    pub(crate) about_to_destroy: Option<StashedClosure>,
}

#[derive(Clone)]
pub(crate) struct PropertySink {
    pub(crate) node: NodeHandle,
    pub(crate) property: String,
}

#[derive(Clone)]
pub(crate) enum EffectSink {
    Property(PropertySink),
    State(NodeHandle),
}

pub(crate) struct ReactiveState {
    pub(crate) graph: Option<Graph<IpcValue>>,
    pub(crate) values: HashMap<SignalId, IpcValue>,
    pub(crate) signals: Vec<SignalId>,
    pub(crate) property_signals: HashMap<(NodeHandle, String, bool), SignalId>,
    pub(crate) current_property_names: HashMap<String, (NodeHandle, String)>,
    pub(crate) property_revision: i64,
    /// Advances whenever the scene actually changes: a property lands on a new
    /// value, or a node is created, reparented, or removed.
    ///
    /// This is what tells the host a repaint is due. A service callback merely
    /// *running* is not a reason to repaint — a timer that polls a file and
    /// finds it unchanged would otherwise force a full render of every output,
    /// at its own interval, forever.
    pub(crate) scene_revision: u64,
    pub(crate) reload_seed: HashMap<String, IpcValue>,
    pub(crate) reloadable: HashMap<String, SignalId>,
    pub(crate) reload_request: Option<bool>,
    pub(crate) watch_files: bool,
    pub(crate) watch_files_changed: bool,
    /// Whether the configuration has asked the shell to stop.
    ///
    /// One-way: nothing clears it but the supervisor reading it, and by then
    /// the process is on its way out. A configuration cannot un-quit.
    pub(crate) quit_requested: bool,
    /// Whether the configuration is holding the session awake, and whether that
    /// has changed since the compositor was last told.
    /// The workspace a configuration has asked to switch to, if any.
    pub(crate) workspace_activation: Option<String>,
    /// What a configuration asked to do to other windows this frame.
    pub(crate) toplevel_requests: Vec<ToplevelRequest>,
    pub(crate) idle_inhibited: bool,
    pub(crate) idle_inhibit_changed: bool,
    pub(crate) shortcuts_inhibited: bool,
    pub(crate) shortcuts_inhibit_changed: bool,
    /// Told the compositor's answer, which is not always yes.
    pub(crate) shortcuts_callbacks: Vec<StashedClosure>,
    pub(crate) reload_completed_callbacks: Vec<StashedClosure>,
    pub(crate) reload_failed_callbacks: Vec<StashedClosure>,
    pub(crate) effects: HashMap<u64, LuaEffect>,
    pub(crate) next_effect: u64,
    pub(crate) active: Option<Capture>,
    pub(crate) logs: Vec<LogEntry>,
    /// Shaders the configuration registered, by name.
    ///
    /// Compiled once at load. The renderer is handed the generated WGSL when
    /// the host starts up, and a node only ever carries the program's hash.
    pub(crate) shaders: HashMap<String, RegisteredShader>,
    pub(crate) scene: Scene,
    pub(crate) effect_runs: u64,
    pub(crate) clock: SignalId,
    pub(crate) handlers: HashMap<(NodeHandle, UiEvent), StashedClosure>,
    pub(crate) parent_transitions: Vec<ParentTransitionRequest>,
    pub(crate) states: HashMap<NodeHandle, StateSet>,
    pub(crate) ipc_handlers: HashMap<String, StashedClosure>,
    /// Keyed on the threshold and whether it ignores inhibitors, because the
    /// same number of milliseconds means two different things to the compositor.
    pub(crate) idle_callbacks: HashMap<(u32, bool), Vec<StashedClosure>>,
    pub(crate) output_power_requests: Vec<bool>,
    pub(crate) clipboard_requests: Vec<String>,
    pub(crate) clipboard_callbacks: Vec<StashedClosure>,
    pub(crate) screencopy_requests: Vec<ScreencopyRequest>,
    pub(crate) screencopy_callbacks: HashMap<u64, StashedClosure>,
    pub(crate) next_screencopy: u64,
    pub(crate) virtual_keyboard_requests: Vec<VirtualKeyboardRequest>,
    pub(crate) input_method_enable_requested: bool,
    pub(crate) input_method_requests: Vec<InputMethodRequest>,
    pub(crate) input_method_callbacks: Vec<StashedClosure>,
    pub(crate) text_input_enable_requested: bool,
    pub(crate) text_input_requests: Vec<TextInputRequest>,
    pub(crate) text_input_callbacks: Vec<StashedClosure>,
    pub(crate) views: HashMap<NodeHandle, LuaVirtualView>,
    pub(crate) pam_tasks: Vec<PendingPam>,
    pub(crate) pam_sessions: Vec<PendingPamSession>,
    pub(crate) timers: Vec<PendingTimer>,
    pub(crate) timer_callbacks: HashMap<NodeHandle, StashedClosure>,
    pub(crate) animation_callbacks: HashMap<(NodeHandle, String), StashedClosure>,
    pub(crate) group_callbacks: HashMap<GroupId, StashedClosure>,
    pub(crate) loader_factories: HashMap<NodeHandle, StashedClosure>,
    pub(crate) loaded_loaders: HashSet<NodeHandle>,
    pub(crate) retention: Retention<NodeHandle>,
    pub(crate) retain_callbacks: HashMap<NodeHandle, RetainCallbacks>,
    pub(crate) retained_destroy_queue: HashSet<NodeHandle>,
    pub(crate) window_surfaces: HashMap<u64, WindowSurfaceConfig>,
    pub(crate) next_window_surface: u64,
    pub(crate) window_surfaces_changed: bool,
    pub(crate) layer_surface_changed: bool,
    pub(crate) window_surface_actions: Vec<WindowSurfaceAction>,
    pub(crate) popup_node_anchors: HashMap<u64, PopupNodeAnchor>,
    pub(crate) transform_tracker: TransformTracker,
    /// The one metatable every scene-node handle shares.
    ///
    /// Built on first use rather than at install time, because it needs the
    /// arena. Every node used to get its own — a fresh table and two fresh
    /// closures per node, neither of which captured anything node-specific, so
    /// a thousand-node tree allocated three thousand objects that were all the
    /// same. Every other userdata type in this crate already shares one.
    pub(crate) node_metatable: Option<StashedTable>,
    pub(crate) transform_watchers: HashMap<u64, LuaTransformWatcher>,
    pub(crate) next_transform_watcher: u64,
    pub(crate) dbus_signals: Vec<PendingDbusSignal>,
    pub(crate) dbus_services: Vec<PendingDbusService>,
    pub(crate) udev_monitors: Vec<PendingUdev>,
    pub(crate) status_notifiers: Vec<PendingStatusNotifier>,
    pub(crate) session_unlock_requested: bool,
    pub(crate) layer_surface: LayerSurfaceConfig,
    pub(crate) shell_root: PathBuf,
}

impl ReactiveState {
    /// Records one line, stamped with when it happened.
    ///
    /// The one way in, so every entry gets a level and a time rather than the
    /// flat strings this used to hold -- a shell running for a day accumulates
    /// thousands, and without either there is no way to ask which are serious
    /// or recent.
    pub(crate) fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis() as u64)
            .unwrap_or(0);
        self.logs.push(LogEntry {
            level,
            at_ms,
            message: message.into(),
        });
    }

    pub(crate) fn new() -> Self {
        let mut graph = Graph::default();
        let initial_clock = IpcValue::String(String::new());
        let clock = graph.signal("morf.clock", initial_clock.clone());
        let mut values = HashMap::new();
        values.insert(clock, initial_clock);
        Self {
            graph: Some(graph),
            values,
            signals: vec![clock],
            property_signals: HashMap::new(),
            current_property_names: HashMap::new(),
            property_revision: 0,
            scene_revision: 0,
            reload_seed: HashMap::new(),
            reloadable: HashMap::new(),
            reload_request: None,
            watch_files: true,
            watch_files_changed: false,
            quit_requested: false,
            workspace_activation: None,
            toplevel_requests: Vec::new(),
            idle_inhibited: false,
            idle_inhibit_changed: false,
            shortcuts_inhibited: false,
            shortcuts_inhibit_changed: false,
            shortcuts_callbacks: Vec::new(),
            reload_completed_callbacks: Vec::new(),
            reload_failed_callbacks: Vec::new(),
            effects: HashMap::new(),
            next_effect: 0,
            active: None,
            logs: Vec::new(),
            shaders: HashMap::new(),
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
            pam_sessions: Vec::new(),
            timers: Vec::new(),
            timer_callbacks: HashMap::new(),
            animation_callbacks: HashMap::new(),
            group_callbacks: HashMap::new(),
            loader_factories: HashMap::new(),
            loaded_loaders: HashSet::new(),
            retention: Retention::default(),
            retain_callbacks: HashMap::new(),
            retained_destroy_queue: HashSet::new(),
            window_surfaces: HashMap::new(),
            next_window_surface: 0,
            window_surfaces_changed: false,
            layer_surface_changed: false,
            window_surface_actions: Vec::new(),
            popup_node_anchors: HashMap::new(),
            transform_tracker: TransformTracker::default(),
            node_metatable: None,
            transform_watchers: HashMap::new(),
            next_transform_watcher: 0,
            dbus_signals: Vec::new(),
            dbus_services: Vec::new(),
            udev_monitors: Vec::new(),
            status_notifiers: Vec::new(),
            session_unlock_requested: false,
            layer_surface: LayerSurfaceConfig::default(),
            shell_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}
