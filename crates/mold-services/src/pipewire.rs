use libloading::Library;
use std::collections::BTreeMap;
use std::env;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::fmt;
use std::mem;
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;

const NODE_INTERFACE: &[u8] = b"PipeWire:Interface:Node\0";
const NODE_VERSION: u32 = 3;
const PARAM_PROPS: u32 = 2;
const TYPE_BOOL: u32 = 2;
const TYPE_FLOAT: u32 = 6;
const TYPE_ARRAY: u32 = 13;
const TYPE_OBJECT: u32 = 15;
const TYPE_OBJECT_PROPS: u32 = 0x40002;
const PROP_VOLUME: u32 = 0x10003;
const PROP_MUTE: u32 = 0x10004;
const PROP_CHANNEL_VOLUMES: u32 = 0x10008;

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

pub struct PipeWire {
    _library: Library,
    symbols: Symbols,
    thread_loop: *mut c_void,
    context: *mut c_void,
    core: *mut c_void,
    registry: *mut c_void,
    state: Box<CallbackState>,
    _core_hook: Box<SpaHook>,
    _registry_hook: Box<SpaHook>,
}

unsafe impl Send for PipeWire {}

impl PipeWire {
    pub fn connect() -> Result<Self, PipeWireError> {
        let library = load_library()?;
        let symbols = unsafe { Symbols::load(&library)? };
        unsafe { (symbols.init)(ptr::null_mut(), ptr::null_mut()) };
        let name = b"mold\0";
        let thread_loop = unsafe { (symbols.thread_loop_new)(name.as_ptr().cast(), ptr::null()) };
        if thread_loop.is_null() {
            return Err(PipeWireError(
                "failed to create PipeWire thread loop".into(),
            ));
        }
        let context = unsafe {
            (symbols.context_new)(
                (symbols.thread_loop_get_loop)(thread_loop),
                ptr::null_mut(),
                0,
            )
        };
        if context.is_null() {
            unsafe { (symbols.thread_loop_destroy)(thread_loop) };
            return Err(PipeWireError("failed to create PipeWire context".into()));
        }
        let core = unsafe { (symbols.context_connect)(context, ptr::null_mut(), 0) };
        if core.is_null() {
            unsafe {
                (symbols.context_destroy)(context);
                (symbols.thread_loop_destroy)(thread_loop);
            }
            return Err(PipeWireError("failed to connect to PipeWire".into()));
        }
        let registry = unsafe { (symbols.core_get_registry)(core, 0, 0) };
        if registry.is_null() {
            unsafe {
                (symbols.core_disconnect)(core);
                (symbols.context_destroy)(context);
                (symbols.thread_loop_destroy)(thread_loop);
            }
            return Err(PipeWireError("failed to get PipeWire registry".into()));
        }

        let mut state = Box::new(CallbackState {
            loop_ptr: thread_loop,
            nodes: Mutex::new(BTreeMap::new()),
            done_seq: Mutex::new(-1),
            error: Mutex::new(None),
            signal: symbols.thread_loop_signal,
        });
        let mut core_hook = Box::new(SpaHook::empty());
        let mut registry_hook = Box::new(SpaHook::empty());
        let state_ptr = (&mut *state as *mut CallbackState).cast();
        unsafe {
            (symbols.core_add_listener)(core, &mut *core_hook, &CORE_EVENTS, state_ptr);
            (symbols.registry_add_listener)(
                registry,
                &mut *registry_hook,
                &REGISTRY_EVENTS,
                state_ptr,
            );
        }
        let started = unsafe { (symbols.thread_loop_start)(thread_loop) };
        if started < 0 {
            unsafe {
                (symbols.core_disconnect)(core);
                (symbols.context_destroy)(context);
                (symbols.thread_loop_destroy)(thread_loop);
            }
            return Err(PipeWireError(format!(
                "failed to start PipeWire loop: {started}"
            )));
        }

        let service = Self {
            _library: library,
            symbols,
            thread_loop,
            context,
            core,
            registry,
            state,
            _core_hook: core_hook,
            _registry_hook: registry_hook,
        };
        service.sync()?;
        Ok(service)
    }

    pub fn nodes(&self) -> Vec<PipeWireNode> {
        self.state.nodes.lock().unwrap().values().cloned().collect()
    }

    pub fn volume(&self, id: u32) -> Result<PipeWireVolume, PipeWireError> {
        self.ensure_node(id)?;
        let mut state = Box::new(VolumeState {
            volume: Mutex::new(PipeWireVolume {
                channels: Vec::new(),
                muted: false,
            }),
        });
        let mut hook = Box::new(SpaHook::empty());
        unsafe { (self.symbols.thread_loop_lock)(self.thread_loop) };
        let node = unsafe {
            (self.symbols.registry_bind)(
                self.registry,
                id,
                NODE_INTERFACE.as_ptr().cast(),
                NODE_VERSION,
                0,
            )
        };
        if node.is_null() {
            unsafe { (self.symbols.thread_loop_unlock)(self.thread_loop) };
            return Err(PipeWireError(format!("failed to bind PipeWire node {id}")));
        }
        let state_ptr = (&mut *state as *mut VolumeState).cast();
        unsafe {
            (self.symbols.node_add_listener)(node, &mut *hook, &NODE_EVENTS, state_ptr);
            (self.symbols.node_enum_params)(node, 0, PARAM_PROPS, 0, u32::MAX, ptr::null());
        }
        let result = self.sync_locked();
        unsafe {
            (self.symbols.proxy_destroy)(node);
            (self.symbols.thread_loop_unlock)(self.thread_loop);
        }
        result?;
        Ok(state.volume.lock().unwrap().clone())
    }

    pub fn set_volume(&self, id: u32, channels: &[f32], muted: bool) -> Result<(), PipeWireError> {
        self.ensure_node(id)?;
        if channels.is_empty()
            || channels
                .iter()
                .any(|volume| !volume.is_finite() || *volume < 0.0)
        {
            return Err(PipeWireError(
                "volume needs finite non-negative channels".into(),
            ));
        }
        let pod = volume_pod(channels, muted);
        unsafe { (self.symbols.thread_loop_lock)(self.thread_loop) };
        let node = unsafe {
            (self.symbols.registry_bind)(
                self.registry,
                id,
                NODE_INTERFACE.as_ptr().cast(),
                NODE_VERSION,
                0,
            )
        };
        if node.is_null() {
            unsafe { (self.symbols.thread_loop_unlock)(self.thread_loop) };
            return Err(PipeWireError(format!("failed to bind PipeWire node {id}")));
        }
        let result =
            unsafe { (self.symbols.node_set_param)(node, PARAM_PROPS, 0, pod.as_ptr().cast()) };
        let synced = if result < 0 {
            Err(PipeWireError(format!(
                "failed to set PipeWire node {id} volume: {result}"
            )))
        } else {
            self.sync_locked()
        };
        unsafe {
            (self.symbols.proxy_destroy)(node);
            (self.symbols.thread_loop_unlock)(self.thread_loop);
        }
        synced
    }

    fn ensure_node(&self, id: u32) -> Result<(), PipeWireError> {
        if self.state.nodes.lock().unwrap().contains_key(&id) {
            Ok(())
        } else {
            Err(PipeWireError(format!("unknown PipeWire node {id}")))
        }
    }

    fn sync(&self) -> Result<(), PipeWireError> {
        unsafe { (self.symbols.thread_loop_lock)(self.thread_loop) };
        let result = self.sync_locked();
        unsafe { (self.symbols.thread_loop_unlock)(self.thread_loop) };
        result
    }

    fn sync_locked(&self) -> Result<(), PipeWireError> {
        let seq = unsafe { (self.symbols.core_sync)(self.core, 0, 0) };
        if seq < 0 {
            return Err(PipeWireError(format!("PipeWire sync failed: {seq}")));
        }
        loop {
            if let Some(error) = self.state.error.lock().unwrap().take() {
                return Err(PipeWireError(error));
            }
            if *self.state.done_seq.lock().unwrap() == seq {
                return Ok(());
            }
            unsafe { (self.symbols.thread_loop_wait)(self.thread_loop) };
        }
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        unsafe {
            (self.symbols.thread_loop_lock)(self.thread_loop);
            (self.symbols.core_disconnect)(self.core);
            (self.symbols.context_destroy)(self.context);
            (self.symbols.thread_loop_unlock)(self.thread_loop);
            (self.symbols.thread_loop_stop)(self.thread_loop);
            (self.symbols.thread_loop_destroy)(self.thread_loop);
        }
    }
}

static CORE_EVENTS: CoreEvents = CoreEvents {
    version: 1,
    info: None,
    done: Some(core_done),
    ping: None,
    error: Some(core_error),
    remove_id: None,
    bound_id: None,
    add_mem: None,
    remove_mem: None,
    bound_props: None,
};

static REGISTRY_EVENTS: RegistryEvents = RegistryEvents {
    version: 0,
    global: Some(registry_global),
    global_remove: Some(registry_global_remove),
};

static NODE_EVENTS: NodeEvents = NodeEvents {
    version: 0,
    info: None,
    param: Some(node_param),
};

unsafe extern "C" fn core_done(data: *mut c_void, _id: u32, seq: c_int) {
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    *state.done_seq.lock().unwrap() = seq;
    unsafe { (state.signal)(state.loop_ptr, false) };
}

unsafe extern "C" fn core_error(
    data: *mut c_void,
    _id: u32,
    _seq: c_int,
    result: c_int,
    message: *const c_char,
) {
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    let message = c_string(message).unwrap_or_else(|| "unknown error".into());
    *state.error.lock().unwrap() = Some(format!("PipeWire error {result}: {message}"));
    unsafe { (state.signal)(state.loop_ptr, false) };
}

unsafe extern "C" fn registry_global(
    data: *mut c_void,
    id: u32,
    _permissions: u32,
    interface: *const c_char,
    _version: u32,
    props: *const SpaDict,
) {
    if c_string(interface).as_deref() != Some("PipeWire:Interface:Node") || props.is_null() {
        return;
    }
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    let properties = unsafe { dict(props) };
    let media_class = properties.get("media.class").cloned().unwrap_or_default();
    let name = properties
        .get("node.name")
        .or_else(|| properties.get("media.name"))
        .cloned()
        .unwrap_or_else(|| format!("node-{id}"));
    let description = properties
        .get("node.description")
        .or_else(|| properties.get("node.nick"))
        .cloned()
        .unwrap_or_else(|| name.clone());
    let serial = properties
        .get("object.serial")
        .and_then(|value| value.parse().ok());
    state.nodes.lock().unwrap().insert(
        id,
        PipeWireNode {
            id,
            serial,
            name,
            description,
            media_class,
        },
    );
}

unsafe extern "C" fn registry_global_remove(data: *mut c_void, id: u32) {
    let state = unsafe { &*(data.cast::<CallbackState>()) };
    state.nodes.lock().unwrap().remove(&id);
}

unsafe extern "C" fn node_param(
    data: *mut c_void,
    _seq: c_int,
    id: u32,
    _index: u32,
    _next: u32,
    pod: *const SpaPod,
) {
    if id != PARAM_PROPS || pod.is_null() {
        return;
    }
    let state = unsafe { &*(data.cast::<VolumeState>()) };
    if let Some(volume) = unsafe { parse_volume(pod) } {
        *state.volume.lock().unwrap() = volume;
    }
}

unsafe fn dict(props: *const SpaDict) -> BTreeMap<String, String> {
    let props = unsafe { &*props };
    let mut result = BTreeMap::new();
    for index in 0..props.n_items as usize {
        let item = unsafe { &*props.items.add(index) };
        if let (Some(key), Some(value)) = (c_string(item.key), c_string(item.value)) {
            result.insert(key, value);
        }
    }
    result
}

fn c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn load_library() -> Result<Library, PipeWireError> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("MOLD_PIPEWIRE_LIBRARY") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib64/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu/libpipewire-0.3.so.0"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu/libpipewire-0.3.so.0"),
    ]);
    let mut errors = Vec::new();
    for candidate in candidates {
        match unsafe { Library::new(&candidate) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{}: {error}", candidate.display())),
        }
    }
    Err(PipeWireError(format!(
        "could not load PipeWire ({})",
        errors.join("; ")
    )))
}

fn align(value: usize) -> usize {
    (value + 7) & !7
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_ne_bytes());
}

fn pad(bytes: &mut Vec<u8>) {
    bytes.resize(align(bytes.len()), 0);
}

fn volume_pod(channels: &[f32], muted: bool) -> Vec<u8> {
    let mut properties = Vec::new();
    push_u32(&mut properties, PROP_MUTE);
    push_u32(&mut properties, 0);
    push_u32(&mut properties, 4);
    push_u32(&mut properties, TYPE_BOOL);
    push_u32(&mut properties, u32::from(muted));
    pad(&mut properties);

    push_u32(&mut properties, PROP_CHANNEL_VOLUMES);
    push_u32(&mut properties, 0);
    push_u32(&mut properties, 8 + channels.len() as u32 * 4);
    push_u32(&mut properties, TYPE_ARRAY);
    push_u32(&mut properties, 4);
    push_u32(&mut properties, TYPE_FLOAT);
    for channel in channels {
        push_u32(&mut properties, channel.to_bits());
    }
    pad(&mut properties);

    let mut pod = Vec::new();
    push_u32(&mut pod, 8 + properties.len() as u32);
    push_u32(&mut pod, TYPE_OBJECT);
    push_u32(&mut pod, TYPE_OBJECT_PROPS);
    push_u32(&mut pod, PARAM_PROPS);
    pod.extend(properties);
    pod
}

unsafe fn parse_volume(pod: *const SpaPod) -> Option<PipeWireVolume> {
    let pod = unsafe { &*pod };
    if pod.kind != TYPE_OBJECT || pod.size < 8 {
        return None;
    }
    let base = pod as *const SpaPod as *const u8;
    let end = 8usize.checked_add(pod.size as usize)?;
    let mut offset = 16usize;
    let mut channels = Vec::new();
    let mut muted = false;
    while offset.checked_add(16)? <= end {
        let key = unsafe { ptr::read_unaligned(base.add(offset).cast::<u32>()) };
        let value_size =
            unsafe { ptr::read_unaligned(base.add(offset + 8).cast::<u32>()) } as usize;
        let value_type = unsafe { ptr::read_unaligned(base.add(offset + 12).cast::<u32>()) };
        if offset.checked_add(16)?.checked_add(value_size)? > end {
            return None;
        }
        if key == PROP_MUTE && value_type == TYPE_BOOL && value_size >= 4 {
            muted = unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) } != 0;
        } else if key == PROP_VOLUME && value_type == TYPE_FLOAT && value_size >= 4 {
            let bits = unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) };
            channels = vec![f32::from_bits(bits)];
        } else if key == PROP_CHANNEL_VOLUMES && value_type == TYPE_ARRAY && value_size >= 8 {
            let child_size =
                unsafe { ptr::read_unaligned(base.add(offset + 16).cast::<u32>()) } as usize;
            let child_type = unsafe { ptr::read_unaligned(base.add(offset + 20).cast::<u32>()) };
            if child_size == 4 && child_type == TYPE_FLOAT {
                channels.clear();
                let count = (value_size - 8) / child_size;
                for index in 0..count {
                    let bits = unsafe {
                        ptr::read_unaligned(base.add(offset + 24 + index * 4).cast::<u32>())
                    };
                    channels.push(f32::from_bits(bits));
                }
            }
        }
        offset = align(offset + 16 + value_size);
    }
    Some(PipeWireVolume { channels, muted })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_pod_round_trips() {
        let pod = volume_pod(&[0.25, 0.75], true);
        let parsed = unsafe { parse_volume(pod.as_ptr().cast()) }.unwrap();
        assert_eq!(parsed.channels, vec![0.25, 0.75]);
        assert!(parsed.muted);
        assert_eq!(parsed.average(), 0.5);
    }

    #[test]
    fn volume_pod_uses_eight_byte_alignment() {
        assert_eq!(align(9), 16);
        assert_eq!(align(16), 16);
    }
}
