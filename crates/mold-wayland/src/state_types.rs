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

/// One live wlr-layer-shell surface and the per-surface state the compositor
/// configures independently of every other layer surface this client owns.
struct LayerRecord {
    surface: LayerSurface,
    fractional_scale: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    width: u32,
    height: u32,
    scale_120: u32,
    /// Whether this surface should map itself with a blank buffer once the
    /// compositor has configured it.
    wants_blank: bool,
    /// Whether a configure has been acknowledged, which the protocol requires
    /// before any buffer may be attached.
    configured: bool,
    /// Backing store for a surface mapped with a blank buffer.
    ///
    /// A reserver has no renderer, but a layer surface that never attaches a
    /// buffer stays unmapped, and a compositor computes an output's usable area
    /// only from the layer surfaces it actually arranges. Holding the pool and
    /// the buffer here keeps the mapping alive for as long as the surface is.
    blank: Option<(SlotPool, ShmBuffer)>,
}

impl Drop for LayerRecord {
    fn drop(&mut self) {
        if let Some(scale) = self.fractional_scale.take() {
            scale.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}

struct LayerState {
    registry: RegistryState,
    compositor: CompositorState,
    outputs: OutputState,
    seats: SeatState,
    xdg_shell: XdgShell,
    layer_shell: LayerShell,
    layers: HashMap<u64, LayerRecord>,
    popups: HashMap<u64, Popup>,
    /// Reposition tokens sent to, and echoed back by, each live popup.
    popup_repositions: HashMap<u64, PopupReposition>,
    floatings: HashMap<u64, Window>,
    floating_sizes: HashMap<u64, (u32, u32)>,
    fractional_manager: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
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
