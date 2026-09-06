//! The backdrop: a click anywhere else, heard without covering the screen.
//!
//! A shell whose surface is only as big as what it shows still wants to close
//! on a click beside it. `morf.surface.backdrop` declares the blank surface
//! that hears one, and `morf.on_backdrop_click` is where the click lands.

use super::*;

#[test]
fn a_backdrop_is_declared_at_load_and_woken_later() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "backdrop.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                morf.surface.backdrop = false
                local clicks = morf.signal("backdrop.clicks", 0)
                morf.on_backdrop_click(function() clicks:set(clicks:get() + 1) end)
                morf.ipc.open = function() morf.surface.backdrop = true end
                morf.ipc.declared = function() return morf.surface.backdrop end
                ui.Text { text = function() return tostring(clicks:get()) end }
            "#,
        )
        .unwrap();
    assert_eq!(
        runtime.layer_surface_config().backdrop,
        Some(false),
        "declared but inert, so the surface exists from the start"
    );
    runtime.take_layer_surface_change();
    runtime.call_ipc("open", &[]).unwrap();
    assert!(
        runtime.take_layer_surface_change(),
        "waking it is a surface change"
    );
    assert_eq!(runtime.layer_surface_config().backdrop, Some(true));
    assert_eq!(
        runtime.call_ipc("declared", &[]).unwrap(),
        [IpcValue::Boolean(true)]
    );
    let root = runtime.scene().roots()[0];
    assert!(runtime.dispatch_backdrop_click());
    runtime.poll_services();
    assert_eq!(runtime.scene().string_value(root, "text").unwrap(), "1");
}

#[test]
fn an_undeclared_backdrop_stays_absent() {
    let mut runtime = Runtime::default();
    runtime
        .execute("plain.lua", b"require(\"morf.ui\").Text { text = \"x\" }")
        .unwrap();
    assert_eq!(runtime.layer_surface_config().backdrop, None);
    assert!(!runtime.dispatch_backdrop_click(), "nobody listening");
}
