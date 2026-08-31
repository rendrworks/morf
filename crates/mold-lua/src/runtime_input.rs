use mold_scene::{NodeHandle, Value as SceneValue};

use crate::{events::*, surface_types::*, types::*};

/// One pointer or touch position, in both spaces a Lua handler may want.
///
/// `surface_x`/`surface_y` are the coordinates the compositor delivered, shared
/// by every node on the surface — the space [`Layout::hit_test`] is queried in.
/// `local_x`/`local_y` are the same point inside the node whose handler runs:
/// `0.0` at its own top-left corner, its width and height at the far edges,
/// with every ancestor offset and transform removed. A handler that wants a
/// fraction of its own extent divides the local pair and nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventPoint {
    /// Pointer x in surface space.
    pub surface_x: f64,
    /// Pointer y in surface space.
    pub surface_y: f64,
    /// Pointer x inside the handling node.
    pub local_x: f64,
    /// Pointer y inside the handling node.
    pub local_y: f64,
}

impl EventPoint {
    /// Builds a point from surface coordinates and their node-local pair.
    pub fn new(surface: (f64, f64), local: (f64, f64)) -> Self {
        Self {
            surface_x: surface.0,
            surface_y: surface.1,
            local_x: local.0,
            local_y: local.1,
        }
    }

    pub(crate) fn args(self) -> [IpcValue; 4] {
        [
            IpcValue::Number(self.surface_x),
            IpcValue::Number(self.surface_y),
            IpcValue::Number(self.local_x),
            IpcValue::Number(self.local_y),
        ]
    }
}

impl Runtime {
    /// Runs one bounded key handler with keysym and UTF-8 text arguments.
    pub fn dispatch_key_event(
        &mut self,
        node: NodeHandle,
        keysym: u32,
        text: Option<&str>,
    ) -> bool {
        self.dispatch_ui_event_with_args(
            node,
            UiEvent::KeyPressed,
            &[
                IpcValue::Integer(keysym as i64),
                text.map_or(IpcValue::Nil, |value| IpcValue::String(value.to_owned())),
            ],
        )
    }

    /// Dispatches one touch event with contact identity and both coordinate
    /// spaces: `(id, surface_x, surface_y, local_x, local_y)`.
    pub fn dispatch_touch_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        id: i32,
        point: EventPoint,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::TouchPressed
                | UiEvent::TouchMoved
                | UiEvent::TouchReleased
                | UiEvent::TouchCanceled
        ) {
            return false;
        }
        let point = point.args();
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Integer(i64::from(id)),
                point[0].clone(),
                point[1].clone(),
                point[2].clone(),
                point[3].clone(),
            ],
        )
    }

    /// Dispatches one pointer event, whatever kind it is.
    ///
    /// The single entry a host uses. A press, a release and a click carry only
    /// a position; a motion or a drag also carries how far the pointer has
    /// travelled since the press. Routing on the event here rather than at the
    /// call site is deliberate: when the two were separate public methods, a
    /// host that reached for the wrong one got `false` and silence, and every
    /// click in the shell was dropped for exactly that reason. There is now no
    /// wrong one to reach for.
    pub fn dispatch_pointer(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        point: EventPoint,
        delta: (f64, f64),
    ) -> bool {
        match event {
            UiEvent::Pressed | UiEvent::Released | UiEvent::Clicked => {
                self.dispatch_button_event(node, event, point)
            }
            UiEvent::PointerMoved
            | UiEvent::DragStarted
            | UiEvent::Dragged
            | UiEvent::DragFinished => self.dispatch_pointer_event(node, event, point, delta),
            _ => false,
        }
    }

    /// Dispatches a pointer button event as `(surface_x, surface_y, local_x,
    /// local_y)`, the position the button was pressed or released at.
    pub(crate) fn dispatch_button_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        point: EventPoint,
    ) -> bool {
        if !matches!(
            event,
            UiEvent::Pressed | UiEvent::Released | UiEvent::Clicked
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(node, event, &point.args())
    }

    /// Dispatches pointer coordinates and displacement to a movement handler as
    /// `(surface_x, surface_y, delta_x, delta_y, local_x, local_y)`.
    ///
    /// The displacement stays measured in surface space: it is the distance the
    /// pointer has travelled since the press, and a drag is free to leave the
    /// node it started on — in which case the local pair runs past the node's
    /// own bounds rather than clamping.
    pub(crate) fn dispatch_pointer_event(
        &mut self,
        node: NodeHandle,
        event: UiEvent,
        point: EventPoint,
        delta: (f64, f64),
    ) -> bool {
        if !matches!(
            event,
            UiEvent::PointerMoved | UiEvent::DragStarted | UiEvent::Dragged | UiEvent::DragFinished
        ) {
            return false;
        }
        self.dispatch_ui_event_with_args(
            node,
            event,
            &[
                IpcValue::Number(point.surface_x),
                IpcValue::Number(point.surface_y),
                IpcValue::Number(delta.0),
                IpcValue::Number(delta.1),
                IpcValue::Number(point.local_x),
                IpcValue::Number(point.local_y),
            ],
        )
    }

    /// Dispatches one wheel or touchpad-axis event to a MouseArea as
    /// `(surface_x, surface_y, pixel_x, pixel_y, step_x, step_y, local_x,
    /// local_y)`.
    pub fn dispatch_wheel_event(
        &mut self,
        node: NodeHandle,
        point: EventPoint,
        pixels: (f64, f64),
        steps: (i32, i32),
    ) -> bool {
        self.dispatch_ui_event_with_args(
            node,
            UiEvent::Wheel,
            &[
                IpcValue::Number(point.surface_x),
                IpcValue::Number(point.surface_y),
                IpcValue::Number(pixels.0),
                IpcValue::Number(pixels.1),
                IpcValue::Integer(i64::from(steps.0)),
                IpcValue::Integer(i64::from(steps.1)),
                IpcValue::Number(point.local_x),
                IpcValue::Number(point.local_y),
            ],
        )
    }

    /// Returns whether a MouseArea accepts one Linux input button code.
    pub fn accepts_pointer_button(&self, node: NodeHandle, button: u32) -> bool {
        let state = self.reactive.borrow();
        let Ok(value) = state.scene.current(node, "accepted_buttons") else {
            return false;
        };
        let accepted = |value: &SceneValue| match value {
            SceneValue::String(name) => match name.as_str() {
                "all" => true,
                "left" => button == 0x110,
                "right" => button == 0x111,
                "middle" => button == 0x112,
                _ => false,
            },
            SceneValue::Number(code) => *code == f64::from(button),
            _ => false,
        };
        match value {
            SceneValue::List(values) => values.iter().any(accepted),
            value => accepted(value),
        }
    }
}
