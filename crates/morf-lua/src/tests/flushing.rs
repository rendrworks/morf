use crate::*;

#[test]
fn a_handler_that_writes_three_signals_flushes_once() {
    // Each `set` outside an effect used to flush the whole graph on the spot:
    // three writes in a click handler were three passes over every dirty
    // effect. Inside a handler they are now one flush, when it returns.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "flush.lua",
            br#"
                local morf = require("morf")
                local a = morf.signal("a", 0)
                local b = morf.signal("b", 0)
                local runs = 0
                morf.effect("count", function()
                    a:get()
                    b:get()
                    runs = runs + 1
                end)
                morf.ipc.poke = function()
                    a:set(1)
                    b:set(2)
                    a:set(3)
                end
                morf.ipc.runs = function() return runs end
                morf.ipc.sum = function() return a:get() + b:get() end
            "#,
        )
        .unwrap();
    assert_eq!(
        runtime.call_ipc("runs", &[]).unwrap(),
        [IpcValue::Integer(1)]
    );

    runtime.call_ipc("poke", &[]).unwrap();

    assert_eq!(
        runtime.call_ipc("runs", &[]).unwrap(),
        [IpcValue::Integer(2)]
    );
    assert_eq!(
        runtime.call_ipc("sum", &[]).unwrap(),
        [IpcValue::Integer(5)]
    );
}

#[test]
fn a_bare_property_write_in_a_handler_reaches_the_bindings_that_read_it() {
    // `node.text = "b"` marked the bindings reading `node.text` dirty and
    // then nothing flushed them: they waited for the next signal write
    // anywhere, or the once-a-second clock. A handler's return is a flush.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "write.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local source = ui.Text { text = "a" }
                local label = ui.Text { text = function() return source.text .. "!" end }
                morf.ipc.rename = function() source.text = "b" end
            "#,
        )
        .unwrap();
    let label = runtime.scene().roots()[1];
    assert_eq!(runtime.scene().string_value(label, "text").unwrap(), "a!");

    runtime.call_ipc("rename", &[]).unwrap();

    assert_eq!(runtime.scene().string_value(label, "text").unwrap(), "b!");
}
