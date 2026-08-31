/// Event name accepted by Lua element handlers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UiEvent {
    /// Pointer entered the target.
    PointerEntered,
    /// Pointer left the target.
    PointerExited,
    /// Pointer moved over or while grabbing the target.
    PointerMoved,
    /// Pointer button was pressed on the target.
    Pressed,
    /// Pointer button was released after pressing the target.
    Released,
    /// Pointer press and release completed on the same target.
    Clicked,
    /// A pointer drag crossed the movement threshold.
    DragStarted,
    /// A pointer drag moved after crossing the threshold.
    Dragged,
    /// A pointer drag ended.
    DragFinished,
    /// A pointer wheel or touchpad axis changed.
    Wheel,
    /// A key was pressed while the target held focus.
    KeyPressed,
    /// A touch contact began on the target.
    TouchPressed,
    /// A grabbed touch contact moved.
    TouchMoved,
    /// A grabbed touch contact ended.
    TouchReleased,
    /// A grabbed touch contact was cancelled.
    TouchCanceled,
}

/// Every event a configuration can handle, and the property it writes.
///
/// One table, read in both directions. It used to be written out twice — once
/// each way, in two files — with nothing keeping the two in step, so an event
/// added to one and forgotten in the other would either be unbindable or
/// unnameable, and nothing would say which.
pub(crate) const EVENT_PROPERTIES: &[(UiEvent, &str)] = &[
    (UiEvent::PointerEntered, "on_entered"),
    (UiEvent::PointerExited, "on_exited"),
    (UiEvent::PointerMoved, "on_position_changed"),
    (UiEvent::Pressed, "on_pressed"),
    (UiEvent::Released, "on_released"),
    (UiEvent::Clicked, "on_clicked"),
    (UiEvent::DragStarted, "on_drag_started"),
    (UiEvent::Dragged, "on_dragged"),
    (UiEvent::DragFinished, "on_drag_finished"),
    (UiEvent::Wheel, "on_wheel"),
    (UiEvent::KeyPressed, "on_key_pressed"),
    (UiEvent::TouchPressed, "on_touch_pressed"),
    (UiEvent::TouchMoved, "on_touch_moved"),
    (UiEvent::TouchReleased, "on_touch_released"),
    (UiEvent::TouchCanceled, "on_touch_canceled"),
];

impl UiEvent {
    pub(crate) fn property(self) -> &'static str {
        EVENT_PROPERTIES
            .iter()
            .find(|(event, _)| *event == self)
            .map_or("on_unknown", |(_, property)| *property)
    }
}
