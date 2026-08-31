use std::time::Duration;

use morf_layout::{Layout, Size};
use morf_lua::Runtime;
use morf_scene::NodeHandle;

struct NoText;
impl morf_layout::TextMeasurer for NoText {
    fn measure(
        &mut self,
        _node: NodeHandle,
        _text: &str,
        _family: &str,
        _size: f64,
        _options: morf_layout::TextOptions,
    ) -> Size {
        Size::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::default();
    runtime.execute(
        "perframe_tx.lua",
        br#"
            local morf = require("morf")
            local core = require("morf.core")
            local ui = require("morf.ui")

            -- driver: a purely visual transform property, no layout effect
            local carrier = ui.Item {
                translate_x = 0,
                implicit_width = 10,
                implicit_height = 10,
                behavior = { translate_x = { duration = 200, easing = "in_out_quad" } },
            }
            local bubble = ui.Rect { width = 10, height = 10, radius = 0, opacity = 1, visible = false }
            local column = ui.Item { opacity = 0, visible = false }
            local root = ui.Item { carrier, bubble, column }
            local calls = 0
            local flips = 0
            core.transform_watcher {
              a = root,
              b = carrier,
              common_parent = root,
              on_changed = function()
                calls = calls + 1
                local t = carrier.translate_x / 100.0
                local ease = t * t * (3 - 2 * t)          -- smoothstep of an eased value
                bubble.width = 10 + (110 - 10) * (t * t)  -- morph*morph width curve
                bubble.height = 10 + 40 * ease
                bubble.radius = 5 + 15 * ease
                bubble.opacity = 1 - t
                column.opacity = ease
                local vis = t > 0.08
                if vis ~= column.visible then flips = flips + 1 end
                column.visible = vis
              end,
            }
            morf.ipc["probe"] = function()
              return calls, flips, carrier.translate_x, bubble.width, bubble.height,
                     bubble.radius, bubble.opacity, column.opacity, column.visible
            end
            morf.ipc["go"] = function()
              carrier.translate_x = 100
              return morf.animation.active(carrier, "translate_x")
            end
        "#,
    )?;
    let root = runtime.scene().roots()[0];
    let relayout = |runtime: &mut Runtime| {
        let layout = Layout::compute(
            &runtime.scene(),
            root,
            Size {
                width: 400.0,
                height: 200.0,
            },
            &mut NoText,
        )
        .unwrap();
        runtime.observe_layout(&layout);
    };
    relayout(&mut runtime);
    println!("go -> {:?}", runtime.call_ipc("go", &[])?);
    for step in 0..12 {
        runtime.tick_animations(Duration::from_millis(20))?;
        relayout(&mut runtime);
        runtime.poll_services();
        println!("step {step}: {:?}", runtime.call_ipc("probe", &[])?);
    }
    println!("binding diagnostics: {:?}", runtime.binding_dependencies());
    for log in runtime.take_logs() {
        println!("log: {log}");
    }
    Ok(())
}
