use crate::client_layer::PRIMARY_LAYER;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::protocol::{wl_output, wl_region, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ZwpTextInputV3},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::{helpers::*, state_types::*, surface_types::*, types::*};

impl LayerState {
    pub(crate) fn layer(&self) -> &LayerSurface {
        &self
            .layers
            .get(&PRIMARY_LAYER)
            .expect("layer surface is initialized before client use")
            .surface
    }

    /// Identifies which of this client's layer surfaces owns a wl_surface.
    pub(crate) fn layer_id(&self, surface: &wl_surface::WlSurface) -> Option<u64> {
        self.layers
            .iter()
            .find_map(|(id, layer)| (surface == layer.surface.wl_surface()).then_some(*id))
    }

    pub(crate) fn refresh_screens(&mut self) {
        let screens = self
            .outputs
            .outputs()
            .filter_map(|output| self.outputs.info(&output))
            .map(|info| ScreenInfo {
                id: info.id,
                name: info.name,
                make: info.make,
                model: info.model,
                description: info.description,
                position: info.logical_position,
                size: info.logical_size,
                physical_size: (info.physical_size.0 > 0 && info.physical_size.1 > 0)
                    .then_some(info.physical_size),
                scale: info.scale_factor,
                transform: output_transform_name(info.transform),
            })
            .collect::<Vec<_>>();
        if screens != self.screens {
            self.screens = screens.clone();
            self.events.push_back(LayerEvent::Screens(screens));
        }
    }

    pub(crate) fn surface_role(&self, surface: &wl_surface::WlSurface) -> Option<SurfaceRole> {
        if let Some(id) = self.layer_id(surface) {
            Some(SurfaceRole::Layer(id))
        } else if let Some(id) = self
            .popups
            .iter()
            .find_map(|(id, popup)| (surface == popup.wl_surface()).then_some(*id))
        {
            Some(SurfaceRole::Popup(id))
        } else {
            self.floatings
                .iter()
                .find_map(|(id, floating)| (surface == floating.wl_surface()).then_some(*id))
                .map(SurfaceRole::Floating)
        }
    }

    pub(crate) fn push_key(&mut self, event: KeyEvent, pressed: bool, repeat: bool) {
        self.events.push_back(LayerEvent::Key {
            surface: self
                .keyboard_surface
                .unwrap_or(SurfaceRole::Layer(PRIMARY_LAYER)),
            keysym: event.keysym.raw(),
            text: event.utf8,
            pressed,
            repeat,
        });
    }
}

impl Dispatch<WpFractionalScaleV1, u64> for LayerState {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &u64,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let Some(layer) = state.layers.get_mut(id) else {
            return;
        };
        layer.scale_120 = scale.max(1);
        let scale_120 = layer.scale_120;
        state
            .events
            .push_back(LayerEvent::Scale { id: *id, scale_120 });
    }
}

impl Dispatch<ExtIdleNotificationV1, u32> for LayerState {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        timeout_ms: &u32,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let idle = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        state.events.push_back(LayerEvent::Idle {
            timeout_ms: *timeout_ms,
            idle,
        });
    }
}

impl Dispatch<ZwlrOutputPowerV1, wl_output::WlOutput> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        output: &wl_output::WlOutput,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => {
                let mode = match mode {
                    wayland_client::WEnum::Value(zwlr_output_power_v1::Mode::Off) => {
                        OutputPowerMode::Off
                    }
                    wayland_client::WEnum::Value(zwlr_output_power_v1::Mode::On) => {
                        OutputPowerMode::On
                    }
                    _ => return,
                };
                // Recorded, not announced: nothing consumed the event.
                let _ = (output, mode);
            }
            zwlr_output_power_v1::Event::Failed => {
                if let Some(index) = state
                    .output_power
                    .iter()
                    .position(|control| control.control == *proxy)
                {
                    state.output_power.remove(index).control.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let format = match format {
                    wayland_client::WEnum::Value(format) => format,
                    wayland_client::WEnum::Unknown(value) => {
                        state.fail_screencopy(proxy, format!("unknown screencopy format {value}"));
                        return;
                    }
                };
                if let Some(pending) = state
                    .screencopies
                    .iter_mut()
                    .find(|pending| pending.frame == *proxy)
                {
                    pending.offer = Some((format, width, height, stride));
                }
                if proxy.version() < 3
                    && let Err(error) = state.start_screencopy(proxy)
                {
                    state.fail_screencopy(proxy, error);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                if let Err(error) = state.start_screencopy(proxy) {
                    state.fail_screencopy(proxy, error);
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                if let wayland_client::WEnum::Value(flags) = flags
                    && let Some(pending) = state
                        .screencopies
                        .iter_mut()
                        .find(|pending| pending.frame == *proxy)
                {
                    pending.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                let Some(request_id) = state
                    .screencopies
                    .iter()
                    .find(|pending| pending.frame == *proxy)
                    .map(|pending| pending.request_id)
                else {
                    return;
                };
                let result = state.finish_screencopy(proxy);
                proxy.destroy();
                state
                    .events
                    .push_back(LayerEvent::Screencopy { request_id, result });
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.fail_screencopy(proxy, "compositor rejected screencopy".to_owned());
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                state.input_method_pending = InputMethodState {
                    active: true,
                    serial: state.input_method_state.serial,
                    ..InputMethodState::default()
                };
            }
            zwp_input_method_v2::Event::Deactivate => {
                state.input_method_pending.active = false;
            }
            zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                state.input_method_pending.surrounding_text = Some(text);
                state.input_method_pending.cursor = cursor;
                state.input_method_pending.anchor = anchor;
            }
            zwp_input_method_v2::Event::Done => {
                state.input_method_pending.serial = state.input_method_state.serial.wrapping_add(1);
                state.input_method_state = state.input_method_pending.clone();
                state
                    .events
                    .push_back(LayerEvent::InputMethod(state.input_method_state.clone()));
            }
            zwp_input_method_v2::Event::Unavailable => {
                if state.input_method.as_ref() == Some(proxy) {
                    state.input_method = None;
                }
                state.input_method_state.active = false;
                state
                    .events
                    .push_back(LayerEvent::InputMethod(state.input_method_state.clone()));
                proxy.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for LayerState {
    fn event(
        state: &mut Self,
        proxy: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _data: &(),
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { .. } => {
                state.text_input_pending.focused = true;
                if state.text_input_requested {
                    proxy.enable();
                    proxy.commit();
                }
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                state.text_input_pending = TextInputState::default();
                state
                    .events
                    .push_back(LayerEvent::TextInput(state.text_input_pending.clone()));
            }
            zwp_text_input_v3::Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                state.text_input_pending.preedit = text;
                state.text_input_pending.preedit_begin = cursor_begin;
                state.text_input_pending.preedit_end = cursor_end;
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                state.text_input_pending.commit = text;
            }
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                state.text_input_pending.delete_before = before_length;
                state.text_input_pending.delete_after = after_length;
            }
            zwp_text_input_v3::Event::Done { serial } => {
                state.text_input_pending.serial = serial;
                state
                    .events
                    .push_back(LayerEvent::TextInput(state.text_input_pending.clone()));
                state.text_input_pending.preedit = None;
                state.text_input_pending.commit = None;
                state.text_input_pending.delete_before = 0;
                state.text_input_pending.delete_after = 0;
            }
            _ => {}
        }
    }
}

impl ProvidesRegistryState for LayerState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

impl ShmHandler for LayerState {
    fn shm_state(&mut self) -> &mut Shm {
        self.shm
            .as_mut()
            .expect("wl_shm handler requires bound state")
    }
}

delegate_registry!(LayerState);
smithay_client_toolkit::delegate_dispatch2!(LayerState);
wayland_client::delegate_noop!(LayerState: ignore WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(LayerState: ignore WpViewporter);
wayland_client::delegate_noop!(LayerState: ignore ExtIdleNotifierV1);
wayland_client::delegate_noop!(LayerState: ignore ZwlrOutputPowerManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwlrScreencopyManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(LayerState: ignore ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(LayerState: ignore ZwpTextInputManagerV3);
wayland_client::delegate_noop!(LayerState: ignore WpViewport);
wayland_client::delegate_noop!(LayerState: ignore wl_region::WlRegion);
// The per-surface object is inert: it is only ever a handle to call
// `set_blur_region` on, and it sends nothing back.
wayland_client::delegate_noop!(LayerState: ignore ExtBackgroundEffectSurfaceV1);

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for LayerState {
    /// The manager's one event: what the compositor is currently willing to do.
    ///
    /// It arrives when the manager is bound and again whenever it changes, so
    /// blur can be withdrawn while a session is running — and when it is, the
    /// compositor stops applying it even to regions that were already set.
    /// Tracking it means a configuration can be told the truth rather than
    /// being left to wonder why its panel is sharp.
    fn event(
        state: &mut Self,
        _manager: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.blur_capable = flags.into_result().is_ok_and(|capabilities| {
                capabilities.contains(ext_background_effect_manager_v1::Capability::Blur)
            });
        }
    }
}
