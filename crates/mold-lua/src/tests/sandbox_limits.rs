use crate::*;
use mold_scene::{Element, Value as SceneValue};

use super::*;

// What the sandbox actually bounds: fuel budgets, and the scene a bounded run
// is allowed to build.

#[test]
fn runaway_lua_effect_exhausts_its_own_fuel() {
    let mut runtime = Runtime::new(Limits {
        effect_fuel: 1_000,
        frame_fuel: 10_000,
        slice_fuel: 64,
        ..Limits::default()
    });
    runtime
        .execute(
            "effect-fuel.lua",
            br#"
                local mold = require("mold")
                local ok, err = mold.effect("runaway", function()
                    while true do end
                end)
                assert(not ok)
                assert(string.find(err, "effect fuel exhausted", 1, true))
            "#,
        )
        .unwrap();
    assert!(runtime.take_logs()[0].contains("runaway"));
}

#[test]
fn lua_builds_a_scene_tree_with_bound_properties() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "scene.lua",
            br##"
                local mold = require("mold")
                local ui = require("mold.ui")
                local clock = mold.signal("clock", "12:00")
                ui.Row {
                    spacing = 6,
                    ui.Text {
                        text = function() return clock:get() end,
                        color = "#ffffff",
                    },
                    ui.Rect {
                        width = 20,
                        height = 10,
                        color = "#7c3aed",
                    },
                }
                local ok, err = clock:set("12:01")
                assert(ok, err)
            "##,
        )
        .unwrap();

    let scene = runtime.scene();
    let roots = scene.roots();
    assert_eq!(roots.len(), 1);
    assert_eq!(scene.element(roots[0]).unwrap(), Element::Row);
    let children = scene.children(roots[0]).unwrap().to_vec();
    assert_eq!(children.len(), 2);
    assert_eq!(
        scene.current(children[0], "text").unwrap(),
        &SceneValue::String("12:01".to_owned())
    );
    assert_eq!(scene.number(children[1], "width").unwrap(), 20.0);
}

#[test]
fn pointer_handlers_receive_both_coordinate_spaces() {
    // Gap 9. A slider reads the press position directly; before this it had to
    // cache the last motion and hope one had arrived. The node sits at an
    // offset inside its parent so surface space and node-local space cannot be
    // confused for one another.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "pointer.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local seen = mold.signal("pointer.seen", "")
                local function record(name)
                  return function(sx, sy, lx, ly)
                    seen:set(("%s %g,%g %g,%g"):format(name, sx, sy, lx, ly))
                  end
                end
                mold.ipc["pointer.seen"] = function() return seen:get() end
                ui.Item {
                  width = 400,
                  height = 300,
                  ui.MouseArea {
                    x = 100,
                    y = 40,
                    width = 200,
                    height = 60,
                    on_pressed = record("pressed"),
                    on_released = record("released"),
                    on_clicked = record("clicked"),
                    on_dragged = function(sx, sy, dx, dy, lx, ly)
                      seen:set(("dragged %g,%g d%g,%g %g,%g"):format(sx, sy, dx, dy, lx, ly))
                    end,
                  },
                }
            "#,
        )
        .unwrap();

    let area = {
        let scene = runtime.scene();
        let root = scene.roots()[0];
        scene.children(root).unwrap()[0]
    };
    fn seen(runtime: &mut Runtime) -> String {
        match runtime.call_ipc("pointer.seen", &[]).unwrap().as_slice() {
            [IpcValue::String(value)] => value.clone(),
            other => panic!("pointer.seen returned {other:?}"),
        }
    }

    // The press lands 30 px in and 15 px down from the area's own corner.
    let point = EventPoint::new((130.0, 55.0), (30.0, 15.0));

    assert!(runtime.dispatch_pointer(area, UiEvent::Pressed, point, (0.0, 0.0)));
    assert_eq!(seen(&mut runtime), "pressed 130,55 30,15");

    assert!(runtime.dispatch_pointer(area, UiEvent::Released, point, (0.0, 0.0)));
    assert_eq!(seen(&mut runtime), "released 130,55 30,15");

    assert!(runtime.dispatch_pointer(area, UiEvent::Clicked, point, (0.0, 0.0)));
    assert_eq!(seen(&mut runtime), "clicked 130,55 30,15");

    // A drag keeps its displacement in surface space and lets the local pair
    // run past the node it started on.
    let dragged = EventPoint::new((350.0, 55.0), (250.0, 15.0));
    assert!(runtime.dispatch_pointer(area, UiEvent::Dragged, dragged, (220.0, 0.0)));
    assert_eq!(seen(&mut runtime), "dragged 350,55 d220,0 250,15");

    // One entry takes every pointer event there is, so a host cannot reach for
    // the wrong one and get silence — which is exactly how every click in the
    // shell came to be dropped. An event that carries no pointer position is
    // still refused.
    assert!(!runtime.dispatch_pointer(area, UiEvent::PointerEntered, point, (0.0, 0.0)));
    assert!(!runtime.dispatch_pointer(area, UiEvent::KeyPressed, point, (0.0, 0.0)));
}

#[test]
fn a_runaway_variant_factory_is_cut_off_once_not_once_per_entry() {
    // The fuel budget is what stands between a configuration and the shell
    // locking up. `mold.variants` runs its factory once per model entry, and
    // when each run took a fresh budget a 256-entry model could spend 256 times
    // what any other Lua entry point is allowed before anything stopped it.
    let mut runtime = Runtime::new(Limits {
        effect_fuel: 20_000,
        slice_fuel: 64,
        ..Limits::default()
    });
    let error = runtime
        .execute(
            "variant-fuel.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local model = {}
                for index = 1, 200 do model[index] = index end
                mold.variants(model, function(entry)
                    local total = 0
                    for step = 1, 100000 do total = total + step end
                    return ui.Rect { width = total }
                end)
            "#,
        )
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("variant factory fuel exhausted"),
        "the whole call is cut off, not each entry: {message}"
    );
}

#[test]
fn an_easing_curve_interpolates_between_an_integer_and_a_float() {
    // Lua writes `0` and `1.0` as different kinds of number, and matching the
    // two kinds pairwise rejected the mix — so the most ordinary way to say
    // "from nothing to one" was the one spelling that did not work.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "easing-mixed.lua",
            br#"
                local core = require("mold.core")
                local curve = core.easing_curve("in_out_cubic")
                assert(curve:interpolate(0.5, 0, 1.0) == curve:interpolate(0.5, 0.0, 1.0))
                assert(curve:interpolate(1.0, 0, 10) == 10.0)
                assert(curve:interpolate(1.0, 2.5, 4) == 4.0)
            "#,
        )
        .unwrap();
}

#[test]
fn a_shared_behavior_table_can_be_both_a_spring_and_a_smoothing() {
    // `ui.spring` used to write `kind` into the table it was handed and return
    // the same reference, so reusing one settings table for both wrote the
    // second kind over the first — in both places, including the node already
    // built, and with nothing to say it had happened.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "shared-behavior.lua",
            br#"
                local ui = require("mold.ui")
                local settle = { duration = 200 }
                local springy = ui.spring(settle)
                local smooth = ui.smoothed(settle)
                assert(springy.kind == "spring", "the first keeps its kind")
                assert(smooth.kind == "smoothed")
                assert(settle.kind == nil, "and the caller's table is untouched")
                assert(springy.duration == 200 and smooth.duration == 200)
            "#,
        )
        .unwrap();
}
