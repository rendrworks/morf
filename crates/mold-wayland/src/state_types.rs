/// Owned Wayland display and surface handles for graphics APIs.
#[derive(Clone, Debug)]
pub struct WaylandWindowTarget {
    backend: wayland_backend::client::Backend,
    surface: wl_surface::WlSurface,
}

impl HasDisplayHandle for WaylandWindowTarget {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.backend.display_handle()
    }
}

impl HasWindowHandle for WaylandWindowTarget {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer =
            NonNull::new(self.surface.id().as_ptr().cast()).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

struct LayerState {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    seats: SeatState,
    xdg_shell: XdgShell,
    layer: Option<LayerSurface>,
    popups: HashMap<u64, Popup>,
    floatings: HashMap<u64, Window>,
    floating_sizes: HashMap<u64, (u32, u32)>,
    _fractional_manager: Option<WpFractionalScaleManagerV1>,
    fractional_scale: Option<WpFractionalScaleV1>,
    _viewporter: Option<WpViewporter>,
    viewport: Option<WpViewport>,
    width: u32,
    height: u32,
    scale_120: u32,
    events: VecDeque<LayerEvent>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    touch_points: HashMap<i32, ((f64, f64), SurfaceRole)>,
    keyboard_surface: Option<SurfaceRole>,
    idle_notifier: Option<ExtIdleNotifierV1>,
    idle_notifications: Vec<ExtIdleNotificationV1>,
    idle_timeouts: Vec<u32>,
    data_device_manager: Option<DataDeviceManagerState>,
    data_devices: Vec<DataDevice>,
    clipboard_source: Option<CopyPasteSource>,
    clipboard_text: String,
    clipboard_tx: mpsc::Sender<Option<String>>,
    clipboard_rx: mpsc::Receiver<Option<String>>,
    clipboard_reads: Arc<AtomicUsize>,
    clipboard_writes: Arc<AtomicUsize>,
    latest_input_serial: Option<u32>,
    virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    virtual_keyboard_keymap: Option<String>,
    virtual_keyboard_keymap_file: Option<File>,
    virtual_keyboard_clock: Instant,
    input_method_manager: Option<ZwpInputMethodManagerV2>,
    input_method: Option<ZwpInputMethodV2>,
    input_method_pending: InputMethodState,
    input_method_state: InputMethodState,
    text_input_manager: Option<ZwpTextInputManagerV3>,
    text_input: Option<ZwpTextInputV3>,
    text_input_requested: bool,
    text_input_pending: TextInputState,
    output_power_manager: Option<ZwlrOutputPowerManagerV1>,
    output_power: Vec<OutputPowerControl>,
    output_power_target: Option<wl_output::WlOutput>,
    output_power_mode: Option<OutputPowerMode>,
    shm: Option<Shm>,
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    screencopies: Vec<PendingScreencopy>,
    screens: Vec<ScreenInfo>,
    session_locks: SessionLockState,
    session_lock: Option<SessionLock>,
    lock_surfaces: Vec<LockSurface>,
}

struct OutputPowerControl {
    output: wl_output::WlOutput,
    control: ZwlrOutputPowerV1,
}

struct PendingScreencopy {
    request_id: u64,
    frame: ZwlrScreencopyFrameV1,
    offer: Option<(wl_shm::Format, u32, u32, u32)>,
    pool: Option<SlotPool>,
    buffer: Option<ShmBuffer>,
    format: Option<ScreencopyFormat>,
    y_invert: bool,
}

struct LockSurface {
    surface: SessionLockSurface,
    output: wl_output::WlOutput,
    size: (u32, u32),
    scale: u32,
}

