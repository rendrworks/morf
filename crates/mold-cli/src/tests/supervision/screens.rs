use crate::lock::WorkerCommand;
use crate::supervisor::known_outputs;
use crate::supervisor::lua_screen;
use crate::supervisor::store_outputs;
use crate::workers::handle_worker_command;
use crate::*;
use mold_lua::{Limits, Runtime};
use mold_wayland::ScreenInfo;

// The compositor's output list reaching every worker's `mold.screens`.

#[test]
fn a_hotplug_reaches_every_worker_runtime() {
    let screens = [
        ScreenInfo {
            id: 7,
            name: Some("eDP-1".to_owned()),
            position: Some((0, 0)),
            size: Some((1920, 1080)),
            scale: 1,
            transform: "normal",
            ..ScreenInfo::default()
        },
        ScreenInfo {
            id: 9,
            name: Some("DP-2".to_owned()),
            position: Some((1920, 0)),
            size: Some((2560, 1440)),
            scale: 2,
            transform: "normal",
            ..ScreenInfo::default()
        },
    ];
    // The supervisor only tells the workers when the topology actually moved.
    assert!(store_outputs(&screens));
    assert!(!store_outputs(&screens));
    assert_eq!(
        known_outputs()
            .iter()
            .map(|screen| screen.name.clone())
            .collect::<Vec<_>>(),
        ["eDP-1", "DP-2"]
    );
    let own = lua_screen(&screens[1]);
    let mut runtime = Runtime::for_screen(Limits::default(), own.clone());

    let update = handle_worker_command(
        &mut runtime,
        &own,
        LoadPolicy::default(),
        WorkerCommand::Screens(screens.to_vec()),
    );

    assert!(!update.repaint);
    runtime
        .execute(
            "screens.lua",
            br#"
                assert(#mold.screens == 2)
                assert(mold.screens[1].name == "DP-2")
                assert(mold.screens[1].x == 1920)
                assert(mold.screens[1].device_pixel_ratio == 2)
                assert(mold.screens[2].name == "eDP-1")
                assert(mold.screens[2].width == 1920)
            "#,
        )
        .unwrap();
    // Left as the rest of the suite expects to find it.
    store_outputs(&[]);
}
