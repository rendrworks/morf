use crate::*;

use super::*;

// `morf.screens`: the compositor's whole output list, own output first.

fn output(name: &str, x: i32, width: i32, height: i32) -> Screen {
    Screen {
        id: 1,
        name: name.to_owned(),
        position: Some((x, 0)),
        width: Some(width),
        height: Some(height),
        scale: 1,
        transform: "normal".to_owned(),
        ..Screen::default()
    }
}

#[test]
fn screens_list_every_output_with_the_instance_own_output_first() {
    let mut runtime = Runtime::for_screen(Limits::default(), output("DP-2", 1920, 2560, 1440));

    runtime.set_screens(&[
        output("eDP-1", 0, 1920, 1080),
        output("DP-2", 1920, 2560, 1440),
        output("HDMI-A-1", 4480, 1280, 1024),
    ]);

    runtime
        .execute(
            "screens.lua",
            br#"
                local screens = morf.screens
                assert(#screens == 3)
                assert(screens[1].name == "DP-2")
                assert(screens[2].name == "eDP-1")
                assert(screens[3].name == "HDMI-A-1")
                assert(screens[1].x == 1920 and screens[1].width == 2560)
                assert(screens[2].x == 0 and screens[2].height == 1080)
                assert(screens[3].scale == 1 and screens[3].transform == "normal")
                -- `Workspace.qml`'s barOnRight, with no compositor query.
                local main = screens[2]
                local own = screens[1]
                local main_centre = main.x + main.width / 2
                local own_centre = own.x + own.width / 2
                assert(own_centre > main_centre)
            "#,
        )
        .unwrap();
}

#[test]
fn unplugging_an_output_drops_it_from_the_screen_list() {
    let mut runtime = Runtime::for_screen(Limits::default(), output("DP-2", 1920, 2560, 1440));
    runtime.set_screens(&[
        output("eDP-1", 0, 1920, 1080),
        output("DP-2", 1920, 2560, 1440),
    ]);

    runtime.set_screens(&[output("eDP-1", 0, 1920, 1080)]);

    runtime
        .execute(
            "unplugged.lua",
            br#"
                -- The instance keeps its own output at index 1 even once the
                -- compositor stops reporting it; the supervisor is what stops
                -- this worker.
                assert(#morf.screens == 2)
                assert(morf.screens[1].name == "DP-2")
                assert(morf.screens[2].name == "eDP-1")
                assert(morf.screens[3] == nil)
            "#,
        )
        .unwrap();
}

#[test]
fn a_runtime_with_no_compositor_keeps_the_screens_it_was_built_with() {
    let mut runtime = Runtime::for_screen(Limits::default(), output("eDP-1", 0, 1920, 1080));

    runtime.set_screens(&[]);

    runtime
        .execute(
            "no-outputs.lua",
            br#"
                assert(#morf.screens == 1)
                assert(morf.screens[1].name == "eDP-1")
            "#,
        )
        .unwrap();
}

#[test]
fn outputs_the_compositor_left_unnamed_stay_separate_entries() {
    let mut runtime = Runtime::for_screen(Limits::default(), output("", 0, 1920, 1080));

    runtime.set_screens(&[output("", 0, 1920, 1080), output("", 1920, 2560, 1440)]);

    runtime
        .execute(
            "unnamed.lua",
            br#"
                -- Nothing addresses a nameless output, but it still occupies
                -- the desktop, so it may not collapse into its neighbour.
                assert(#morf.screens == 2)
                assert(morf.screens[1].width == 1920)
                assert(morf.screens[2].width == 2560)
            "#,
        )
        .unwrap();
}
