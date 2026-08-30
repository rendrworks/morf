#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeWireNode {
    pub id: u32,
    pub serial: Option<u64>,
    pub name: String,
    pub description: String,
    pub media_class: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipeWireVolume {
    pub channels: Vec<f32>,
    pub muted: bool,
}

impl PipeWireVolume {
    pub fn average(&self) -> f32 {
        if self.channels.is_empty() {
            0.0
        } else {
            self.channels.iter().sum::<f32>() / self.channels.len() as f32
        }
    }
}

#[derive(Debug)]
pub struct PipeWireError(String);

impl fmt::Display for PipeWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PipeWireError {}

#[repr(C)]
struct SpaList {
    next: *mut SpaList,
    prev: *mut SpaList,
}

#[repr(C)]
struct SpaCallbacks {
    funcs: *const c_void,
    data: *mut c_void,
}

#[repr(C)]
struct SpaHook {
    link: SpaList,
    cb: SpaCallbacks,
    removed: Option<unsafe extern "C" fn(*mut SpaHook)>,
    private: *mut c_void,
}

impl SpaHook {
    fn empty() -> Self {
        unsafe { mem::zeroed() }
    }
}

#[repr(C)]
struct SpaDictItem {
    key: *const c_char,
    value: *const c_char,
}

#[repr(C)]
struct SpaDict {
    flags: u32,
    n_items: u32,
    items: *const SpaDictItem,
}

#[repr(C)]
struct SpaPod {
    size: u32,
    kind: u32,
}

#[repr(C)]
struct RegistryEvents {
    version: u32,
    global: Option<unsafe extern "C" fn(*mut c_void, u32, u32, *const c_char, u32, *const SpaDict)>,
    global_remove: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}

#[repr(C)]
struct CoreEvents {
    version: u32,
    info: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    done: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    ping: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    error: Option<unsafe extern "C" fn(*mut c_void, u32, c_int, c_int, *const c_char)>,
    remove_id: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    bound_id: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
    add_mem: Option<unsafe extern "C" fn(*mut c_void, u32, u32, c_int, u32)>,
    remove_mem: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    bound_props: Option<unsafe extern "C" fn(*mut c_void, u32, u32, *const SpaDict)>,
}

#[repr(C)]
struct NodeEvents {
    version: u32,
    info: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    param: Option<unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const SpaPod)>,
}

struct CallbackState {
    loop_ptr: *mut c_void,
    nodes: Mutex<BTreeMap<u32, PipeWireNode>>,
    done_seq: Mutex<c_int>,
    error: Mutex<Option<String>>,
    signal: unsafe extern "C" fn(*mut c_void, bool),
}

struct VolumeState {
    volume: Mutex<PipeWireVolume>,
}

struct Symbols {
    init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char),
    thread_loop_new: unsafe extern "C" fn(*const c_char, *const SpaDict) -> *mut c_void,
    thread_loop_destroy: unsafe extern "C" fn(*mut c_void),
    thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    thread_loop_stop: unsafe extern "C" fn(*mut c_void),
    thread_loop_lock: unsafe extern "C" fn(*mut c_void),
    thread_loop_unlock: unsafe extern "C" fn(*mut c_void),
    thread_loop_wait: unsafe extern "C" fn(*mut c_void),
    thread_loop_signal: unsafe extern "C" fn(*mut c_void, bool),
    context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_destroy: unsafe extern "C" fn(*mut c_void),
    context_connect: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    core_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    core_add_listener:
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const CoreEvents, *mut c_void) -> c_int,
    core_get_registry: unsafe extern "C" fn(*mut c_void, u32, usize) -> *mut c_void,
    core_sync: unsafe extern "C" fn(*mut c_void, u32, c_int) -> c_int,
    registry_add_listener: unsafe extern "C" fn(
        *mut c_void,
        *mut SpaHook,
        *const RegistryEvents,
        *mut c_void,
    ) -> c_int,
    registry_bind: unsafe extern "C" fn(*mut c_void, u32, *const c_char, u32, usize) -> *mut c_void,
    node_add_listener:
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const NodeEvents, *mut c_void) -> c_int,
    node_enum_params:
        unsafe extern "C" fn(*mut c_void, c_int, u32, u32, u32, *const SpaPod) -> c_int,
    node_set_param: unsafe extern "C" fn(*mut c_void, u32, u32, *const SpaPod) -> c_int,
    proxy_destroy: unsafe extern "C" fn(*mut c_void),
}

impl Symbols {
    unsafe fn load(library: &Library) -> Result<Self, PipeWireError> {
        macro_rules! symbol {
            ($name:literal) => {
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|error| PipeWireError(format!("missing {}: {error}", $name)))?
            };
        }
        Ok(Self {
            init: symbol!("pw_init"),
            thread_loop_new: symbol!("pw_thread_loop_new"),
            thread_loop_destroy: symbol!("pw_thread_loop_destroy"),
            thread_loop_get_loop: symbol!("pw_thread_loop_get_loop"),
            thread_loop_start: symbol!("pw_thread_loop_start"),
            thread_loop_stop: symbol!("pw_thread_loop_stop"),
            thread_loop_lock: symbol!("pw_thread_loop_lock"),
            thread_loop_unlock: symbol!("pw_thread_loop_unlock"),
            thread_loop_wait: symbol!("pw_thread_loop_wait"),
            thread_loop_signal: symbol!("pw_thread_loop_signal"),
            context_new: symbol!("pw_context_new"),
            context_destroy: symbol!("pw_context_destroy"),
            context_connect: symbol!("pw_context_connect"),
            core_disconnect: symbol!("pw_core_disconnect"),
            core_add_listener: symbol!("pw_core_add_listener"),
            core_get_registry: symbol!("pw_core_get_registry"),
            core_sync: symbol!("pw_core_sync"),
            registry_add_listener: symbol!("pw_registry_add_listener"),
            registry_bind: symbol!("pw_registry_bind"),
            node_add_listener: symbol!("pw_node_add_listener"),
            node_enum_params: symbol!("pw_node_enum_params"),
            node_set_param: symbol!("pw_node_set_param"),
            proxy_destroy: symbol!("pw_proxy_destroy"),
        })
    }
}

