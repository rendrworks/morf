#[derive(Debug)]
struct SignalToken {
    id: SignalId,
}

struct PersistentToken {
    properties: HashMap<String, SignalId>,
    reloaded: bool,
}

struct ScopeToken {
    prefix: String,
}

struct RetainableToken {
    node: NodeHandle,
}

struct WindowSurfaceToken {
    id: u64,
}

type PopupAnchorArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

type WindowMapRectArgs<'gc> = (
    UserRef<'gc, WindowSurfaceToken>,
    UserRef<'gc, NodeToken>,
    f64,
    f64,
    f64,
    f64,
);

struct TransformWatcherToken {
    id: u64,
}

struct RetainLockToken {
    node: NodeHandle,
    locked: Cell<bool>,
    state: Rc<RefCell<ReactiveState>>,
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
struct NodeToken {
    handle: NodeHandle,
}

struct GroupToken {
    id: GroupId,
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

struct ProcessViewToken {
    state: RefCell<ProcessViewState>,
}

struct ProcessViewState {
    config: ProcessConfig,
    process: Option<Process>,
}

struct FileToken {
    file: FileView,
}

struct FileWatcherToken {
    watcher: FileWatcher,
}

struct FileDocumentToken {
    file: RefCell<FileDocument>,
}

struct SocketToken {
    state: RefCell<SocketState>,
}

struct SocketState {
    path: String,
    socket: Option<Socket>,
}

struct SocketServerToken {
    state: RefCell<SocketServerState>,
}

struct SocketServerState {
    path: String,
    server: Option<SocketServer>,
}

struct LineParserToken {
    parser: RefCell<LineParser>,
}

struct SplitParserToken {
    parser: RefCell<SplitParser>,
}

struct StreamCollectorToken {
    collector: RefCell<StreamCollector>,
}

struct ListModelToken {
    model: Rc<RefCell<ListModel>>,
}

struct VirtualListToken {
    model: Rc<RefCell<ListModel>>,
    view: RefCell<VirtualList>,
}

struct ElapsedTimerToken {
    started: RefCell<Instant>,
}

struct EasingCurveToken {
    easing: Easing,
}

struct ColorQuantizerToken {
    state: RefCell<ColorQuantizerState>,
}

#[derive(Clone)]
struct ColorQuantizerState {
    source: PathBuf,
    depth: u8,
    crop: Option<QuantizeRect>,
    rescale_size: u32,
    colors: Vec<[u8; 4]>,
}

struct SystemClockToken {
    enabled: Cell<bool>,
    precision: RefCell<String>,
}

struct JsonNullToken;

struct DesktopEntriesToken {
    entries: RefCell<DesktopEntries>,
    paths: Vec<PathBuf>,
}

struct MenuToken {
    menu: RefCell<Menu>,
    callbacks: HashMap<String, StashedClosure>,
}

struct LuaVirtualView {
    model: Rc<RefCell<ListModel>>,
    view: VirtualList,
    delegate: StashedClosure,
    active: HashMap<ModelId, DelegateInstance>,
    reusable: HashMap<ModelId, DelegateInstance>,
    reuse_order: VecDeque<ModelId>,
    reuse_limit: usize,
    pool_root: Option<NodeHandle>,
    column_extent: f64,
}

struct DelegateInstance {
    node: NodeHandle,
    updater: Option<StashedClosure>,
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

struct LuaTransformWatcher {
    a: NodeHandle,
    b: NodeHandle,
    watcher: NativeTransformWatcher,
    callback: Option<StashedClosure>,
    revision: u64,
    pending: bool,
}

#[derive(Clone, Debug)]
struct PopupNodeAnchor {
    node: NodeHandle,
    x: i32,
    y: i32,
    width: Option<i32>,
    height: Option<i32>,
    margin_top: i32,
    margin_right: i32,
    margin_bottom: i32,
    margin_left: i32,
}

#[derive(Clone)]
struct LuaEffect {
    closure: StashedClosure,
    sink: Option<EffectSink>,
}

#[derive(Clone, Default)]
struct RetainCallbacks {
    dropped: Option<StashedClosure>,
    about_to_destroy: Option<StashedClosure>,
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
    /// Advances whenever the scene actually changes: a property lands on a new
    /// value, or a node is created, reparented, or removed.
    ///
    /// This is what tells the host a repaint is due. A service callback merely
    /// *running* is not a reason to repaint — a timer that polls a file and
    /// finds it unchanged would otherwise force a full render of every output,
    /// at its own interval, forever.
    scene_revision: u64,
    reload_seed: HashMap<String, ScriptValue>,
    reloadable: HashMap<String, SignalId>,
    reload_request: Option<bool>,
    watch_files: bool,
    watch_files_changed: bool,
    reload_completed_callbacks: Vec<StashedClosure>,
    reload_failed_callbacks: Vec<StashedClosure>,
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
    animation_callbacks: HashMap<(NodeHandle, String), StashedClosure>,
    group_callbacks: HashMap<GroupId, StashedClosure>,
    loader_factories: HashMap<NodeHandle, StashedClosure>,
    loaded_loaders: HashSet<NodeHandle>,
    retention: Retention<NodeHandle>,
    retain_callbacks: HashMap<NodeHandle, RetainCallbacks>,
    retained_destroy_queue: HashSet<NodeHandle>,
    window_surfaces: HashMap<u64, WindowSurfaceConfig>,
    next_window_surface: u64,
    window_surfaces_changed: bool,
    layer_surface_changed: bool,
    window_surface_actions: Vec<WindowSurfaceAction>,
    popup_node_anchors: HashMap<u64, PopupNodeAnchor>,
    transform_tracker: TransformTracker,
    transform_watchers: HashMap<u64, LuaTransformWatcher>,
    next_transform_watcher: u64,
    dbus_signals: Vec<PendingDbusSignal>,
    udev_monitors: Vec<PendingUdev>,
    status_notifiers: Vec<PendingStatusNotifier>,
    session_unlock_requested: bool,
    layer_surface: LayerSurfaceConfig,
    shell_root: PathBuf,
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
