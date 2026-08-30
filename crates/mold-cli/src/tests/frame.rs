// The frame clock that decides how far a tick advances animations.

#[test]
fn an_idle_gap_is_not_charged_to_the_animation_that_follows_it() {
    // No timebase means the scene had settled, so however long ago the last
    // frame callback arrived, the tick that restarts motion advances by zero.
    assert_eq!(animation_delta(None, 10_000), Duration::ZERO);

    // While motion continues the real between-frame time is used.
    assert_eq!(
        animation_delta(Some(1_000), 1_016),
        Duration::from_millis(16)
    );

    // A compositor that fell behind is allowed to catch up, but only so far.
    assert_eq!(
        animation_delta(Some(1_000), 5_000),
        Duration::from_millis(MAX_FRAME_DELTA_MS.into())
    );

    // Frame times are a wrapping millisecond counter.
    assert_eq!(
        animation_delta(Some(u32::MAX - 4), 11),
        Duration::from_millis(16)
    );
}
