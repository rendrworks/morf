//! Idle thresholds that count only the person.
//!
//! A media player inhibits idle so the screen stays on through a film. A shell
//! that dims its own bar after a minute of nobody touching anything still
//! wants that minute -- and before this it could not ask, because the only
//! threshold it could register was the inhibitable one.

use super::*;

#[test]
fn an_input_only_threshold_is_its_own_key() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "idle-input.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local which = morf.signal("idle.which", "awake")
                morf.idle.subscribe(60000, function(idle) which:set(idle and "idle" or "awake") end)
                morf.idle.subscribe(60000, function(idle) which:set(idle and "input-idle" or "awake") end, true)
                ui.Text { text = function() return which:get() end }
            "#,
        )
        .unwrap();
    let mut timeouts = runtime.idle_timeouts();
    timeouts.sort_unstable();
    assert_eq!(
        timeouts,
        [(60_000, false), (60_000, true)],
        "the same number of milliseconds is two different requests"
    );
    let root = runtime.scene().roots()[0];
    assert!(runtime.dispatch_idle(60_000, true, true));
    runtime.poll_services();
    assert_eq!(
        runtime.scene().string_value(root, "text").unwrap(),
        "input-idle",
        "input idleness reaches the callback that asked for it, and only that one"
    );
}
