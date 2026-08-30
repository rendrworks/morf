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

