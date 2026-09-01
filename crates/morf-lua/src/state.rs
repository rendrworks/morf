pub(crate) use crate::api_shader::RegisteredShader;
use crate::states::{Capture, StateSet};
use luna::{StashedClosure, StashedTable, UserRef};
use morf_desktop::DesktopEntries;
use morf_image::ImageRect as QuantizeRect;
use morf_io::{
    DbusProxy, DbusSignal, FileDocument, FileView, FileWatcher, Process, ProcessConfig, Socket,
    SocketServer, SplitParser, StreamCollector, Timer as IoTimer,
};
use morf_layout::{TransformTracker, TransformWatcher as NativeTransformWatcher};
use morf_lifecycle::Retention;
use morf_menu::Menu;
use morf_reactive::{Graph, SignalId};
use morf_scene::{Easing, GroupId, ListModel, ModelId, NodeHandle, Scene, VirtualList};
use morf_services::{GreetdClient, PamTask, PipeWire, StatusNotifierHost, UdevMonitor};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::{events::*, surface_types::*};

#[derive(Debug)]
pub(crate) struct SignalToken {
    pub(crate) id: SignalId,
}

pub(crate) struct PersistentToken {
    pub(crate) properties: HashMap<String, SignalId>,
    pub(crate) reloaded: bool,
}

pub(crate) struct ScopeToken {
    pub(crate) prefix: String,
}

pub(crate) struct RetainableToken {
    pub(crate) node: NodeHandle,
}

pub(crate) struct WindowSurfaceToken {
    pub(crate) id: u64,
}

pub(crate) type PopupAnchorArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

pub(crate) type WindowMapRectArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    UserRef<'gc, NodeToken>,
    f64,
    f64,
    f64,
    f64,
);

pub(crate) struct TransformWatcherToken {
    pub(crate) id: u64,
}

pub(crate) struct RetainLockToken {
    pub(crate) node: NodeHandle,
    pub(crate) locked: Cell<bool>,
    pub(crate) state: Rc<RefCell<ReactiveState>>,
}

impl Drop for RetainLockToken {
    fn drop(&mut self) {
        if !self.locked.get() {
            return;
        }
        if let Ok(mut state) = self.state.try_borrow_mut()
            && state.retention.unlock(self.node).is_ok()
            && state.retention.should_destroy(self.node).unwrap_or(false)
        {
            state.retained_destroy_queue.insert(self.node);
        }
    }
}

#[derive(Debug)]
pub(crate) struct NodeToken {
    pub(crate) handle: NodeHandle,
}

pub(crate) struct GroupToken {
    pub(crate) id: GroupId,
}

#[derive(Debug)]
pub(crate) struct DbusToken {
    pub(crate) proxy: DbusProxy,
}

pub(crate) struct PipeWireToken {
    pub(crate) service: PipeWire,
}

pub(crate) struct GreetdToken {
    pub(crate) client: RefCell<GreetdClient>,
}

pub(crate) struct ProcessToken {
    pub(crate) process: RefCell<Process>,
}

pub(crate) struct ProcessViewToken {
    pub(crate) state: RefCell<ProcessViewState>,
}

pub(crate) struct ProcessViewState {
    pub(crate) config: ProcessConfig,
    pub(crate) process: Option<Process>,
}

pub(crate) struct FileToken {
    pub(crate) file: FileView,
}

pub(crate) struct FileWatcherToken {
    pub(crate) watcher: FileWatcher,
}

pub(crate) struct FileDocumentToken {
    pub(crate) file: RefCell<FileDocument>,
}

pub(crate) struct SocketToken {
    pub(crate) state: RefCell<SocketState>,
}

pub(crate) struct SocketState {
    pub(crate) path: String,
    pub(crate) socket: Option<Socket>,
}

pub(crate) struct SocketServerToken {
    pub(crate) state: RefCell<SocketServerState>,
}

pub(crate) struct SocketServerState {
    pub(crate) path: String,
    pub(crate) server: Option<SocketServer>,
}

pub(crate) struct SplitParserToken {
    pub(crate) parser: RefCell<SplitParser>,
}

pub(crate) struct StreamCollectorToken {
    pub(crate) collector: RefCell<StreamCollector>,
}

pub(crate) struct ListModelToken {
    pub(crate) model: Rc<RefCell<ListModel>>,
}

pub(crate) struct VirtualListToken {
    pub(crate) model: Rc<RefCell<ListModel>>,
    pub(crate) view: RefCell<VirtualList>,
}

pub(crate) struct ElapsedTimerToken {
    pub(crate) started: RefCell<Instant>,
}

pub(crate) struct EasingCurveToken {
    pub(crate) easing: Easing,
}

pub(crate) struct ColorQuantizerToken {
    pub(crate) state: RefCell<ColorQuantizerState>,
}

#[derive(Clone)]
pub(crate) struct ColorQuantizerState {
    pub(crate) source: PathBuf,
    pub(crate) depth: u8,
    pub(crate) crop: Option<QuantizeRect>,
    pub(crate) rescale_size: u32,
    pub(crate) colors: Vec<[u8; 4]>,
}

pub(crate) struct SystemClockToken {
    pub(crate) enabled: Cell<bool>,
    pub(crate) precision: RefCell<String>,
}

pub(crate) struct JsonNullToken;

pub(crate) struct DesktopEntriesToken {
    pub(crate) entries: RefCell<DesktopEntries>,
    pub(crate) paths: Vec<PathBuf>,
}

pub(crate) struct MenuToken {
    pub(crate) menu: RefCell<Menu>,
    pub(crate) callbacks: HashMap<String, StashedClosure>,
}

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

pub(crate) struct PendingPam {
    pub(crate) task: PamTask,
    pub(crate) callback: StashedClosure,
    pub(crate) unlock_on_success: bool,
}

pub(crate) struct PendingTimer {
    pub(crate) timer: IoTimer,
    pub(crate) callback: StashedClosure,
    pub(crate) repeat: bool,
    pub(crate) interval: Duration,
    pub(crate) node: Option<NodeHandle>,
}

pub(crate) struct PendingDbusSignal {
    pub(crate) signal: DbusSignal,
    pub(crate) callback: StashedClosure,
}

pub(crate) struct PendingUdev {
    pub(crate) monitor: UdevMonitor,
    pub(crate) callback: StashedClosure,
}

pub(crate) struct PendingStatusNotifier {
    pub(crate) host: StatusNotifierHost,
    pub(crate) callback: StashedClosure,
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
    pub(crate) reload_completed_callbacks: Vec<StashedClosure>,
    pub(crate) reload_failed_callbacks: Vec<StashedClosure>,
    pub(crate) effects: HashMap<u64, LuaEffect>,
    pub(crate) next_effect: u64,
    pub(crate) active: Option<Capture>,
    pub(crate) logs: Vec<String>,
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
    pub(crate) idle_callbacks: HashMap<u32, Vec<StashedClosure>>,
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
    pub(crate) udev_monitors: Vec<PendingUdev>,
    pub(crate) status_notifiers: Vec<PendingStatusNotifier>,
    pub(crate) session_unlock_requested: bool,
    pub(crate) layer_surface: LayerSurfaceConfig,
    pub(crate) shell_root: PathBuf,
}

impl ReactiveState {
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
            udev_monitors: Vec::new(),
            status_notifiers: Vec::new(),
            session_unlock_requested: false,
            layer_surface: LayerSurfaceConfig::default(),
            shell_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}
