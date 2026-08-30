#[test]
fn lua_plays_a_sequence_of_property_steps() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "group.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local card = ui.Item { x = 0, y = 0, opacity = 0 }
                _G.card = card
                _G.group = mold.animation.play {
                    { node = card, property = "opacity", to = 1, duration = 100 },
                    { pause = 50 },
                    { parallel = {
                        { node = card, property = "x", to = 200, duration = 100 },
                        { node = card, property = "y", to = 100, duration = 100 },
                    }},
                    on_finished = function(reason) _G.reason = reason end,
                }
                assert(_G.group:active())
            "#,
        )
        .unwrap();
    let card = runtime.scene().roots()[0];

    // Only the first step is under way.
    runtime.tick_animations(Duration::from_millis(50)).unwrap();
    assert!(runtime.scene().is_animating(card, "opacity").unwrap());
    assert!(!runtime.scene().is_animating(card, "x").unwrap());

    // Past the first step and its pause, the parallel leg runs both at once.
    runtime.tick_animations(Duration::from_millis(110)).unwrap();
    assert_eq!(runtime.scene().number(card, "opacity").unwrap(), 1.0);
    assert!(runtime.scene().is_animating(card, "x").unwrap());
    assert!(runtime.scene().is_animating(card, "y").unwrap());

    let frame = runtime.tick_animations(Duration::from_millis(150)).unwrap();
    assert_eq!(runtime.scene().number(card, "x").unwrap(), 200.0);
    assert_eq!(runtime.scene().number(card, "y").unwrap(), 100.0);
    assert_eq!(frame.groups.len(), 1);
    assert_eq!(frame.groups[0].end, mold_scene::AnimationEnd::Completed);

    runtime
        .execute(
            "assert.lua",
            br#"
                assert(_G.reason == "completed", "group did not report completion")
                assert(not _G.group:active(), "group is still active")
            "#,
        )
        .unwrap();
}

#[test]
fn lua_stops_and_finishes_a_group_through_its_handle() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "control.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local card = ui.Item { x = 0 }
                _G.card = card
                _G.group = mold.animation.play {
                    { node = card, property = "x", to = 100, duration = 200 },
                    { node = card, property = "y", to = 50, duration = 200 },
                }
            "#,
        )
        .unwrap();
    let card = runtime.scene().roots()[0];
    runtime.tick_animations(Duration::from_millis(50)).unwrap();

    runtime
        .execute(
            "finish.lua",
            br#"
                local mold = require("mold")
                assert(_G.group:finish(), "finish did not act on a running group")
                assert(not _G.group:active())
                -- The handle stays usable after the group is gone and simply
                -- reports that there was nothing left to act on.
                assert(not _G.group:stop())
            "#,
        )
        .unwrap();
    assert_eq!(runtime.scene().number(card, "x").unwrap(), 100.0);
    assert_eq!(runtime.scene().number(card, "y").unwrap(), 50.0);
}

#[test]
fn a_group_specification_is_validated_where_it_is_declared() {
    let mut runtime = Runtime::default();
    let rejected = |source: &'static [u8]| {
        let mut runtime = Runtime::default();
        runtime.execute("bad.lua", source).is_err()
    };

    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "not_a_property", to = 1, duration = 100 },
            }
        "#
    ));
    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "x", to = 1 },
            }
        "#
    ));
    assert!(rejected(
        br#"
            local mold = require("mold")
            local ui = require("mold.ui")
            mold.animation.play {
                { node = ui.Item {}, property = "x", to = 1, duration = 100, loops = "forever" },
            }
        "#
    ));

    // A well-formed group is still accepted after the rejections above.
    runtime
        .execute(
            "good.lua",
            br#"
                local mold = require("mold")
                local ui = require("mold.ui")
                local handle = mold.animation.play {
                    loops = "forever",
                    { node = ui.Item {}, property = "x", to = 1, duration = 100 },
                }
                assert(handle:active())
            "#,
        )
        .unwrap();
}
