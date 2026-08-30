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

impl UiEvent {
    fn property(self) -> &'static str {
        match self {
            Self::PointerEntered => "on_entered",
            Self::PointerExited => "on_exited",
            Self::PointerMoved => "on_position_changed",
            Self::Pressed => "on_pressed",
            Self::Released => "on_released",
            Self::Clicked => "on_clicked",
            Self::DragStarted => "on_drag_started",
            Self::Dragged => "on_dragged",
            Self::DragFinished => "on_drag_finished",
            Self::Wheel => "on_wheel",
            Self::KeyPressed => "on_key_pressed",
            Self::TouchPressed => "on_touch_pressed",
            Self::TouchMoved => "on_touch_moved",
            Self::TouchReleased => "on_touch_released",
            Self::TouchCanceled => "on_touch_canceled",
        }
    }
}

