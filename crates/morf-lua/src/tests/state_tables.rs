use crate::*;

#[test]
fn a_state_table_is_read_by_bindings_and_written_field_by_field() {
    // Signals hold scalars. `morf.state` keeps the shape: each named field
    // is a signal a binding tracks, a nested table is nested, and a whole
    // update is one flush when the handler that made it returns.
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "state.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local model = morf.state {
                    count = 1,
                    who = { name = "a" },
                    items = { "x", "y" },
                }
                local runs = 0
                ui.Text { text = function()
                    runs = runs + 1
                    return tostring(model.count) .. model.who.name
                end }
                morf.ipc.bump = function()
                    model.count = model.count + 1
                    model.who.name = "b"
                end
                morf.ipc.runs = function() return runs end
                morf.ipc.first = function() return model.items:get(1) end
                morf.ipc.bad = function() return model.nope end
            "#,
        )
        .unwrap();
    let label = runtime.scene().roots()[0];
    assert_eq!(runtime.scene().string_value(label, "text").unwrap(), "1a");
    assert_eq!(
        runtime.call_ipc("first", &[]).unwrap(),
        [IpcValue::String("x".into())]
    );

    runtime.call_ipc("bump", &[]).unwrap();

    assert_eq!(runtime.scene().string_value(label, "text").unwrap(), "2b");
    // One initial run, one after the handler: two writes, one flush.
    assert_eq!(
        runtime.call_ipc("runs", &[]).unwrap(),
        [IpcValue::Integer(2)]
    );
    let error = runtime.call_ipc("bad", &[]).unwrap_err().to_string();
    assert!(error.contains("unknown state field `nope`"), "{error}");
}

#[test]
fn ui_each_follows_a_state_list() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "each.lua",
            br#"
                local morf = require("morf")
                local ui = require("morf.ui")
                local model = morf.state { rows = { { id = "a", label = "A" }, { id = "b", label = "B" } } }
                ui.each(model.rows, function(row)
                    return ui.Text { text = row.label }
                end, { as = "column" })
                morf.ipc.shuffle = function()
                    model.rows = { { id = "b", label = "B" }, { id = "c", label = "C" } }
                end
                morf.ipc.rename = function()
                    model.rows:replace({ { id = "b", label = "B2" }, { id = "c", label = "C" } }, "id")
                end
            "#,
        )
        .unwrap();
    let root = runtime.scene().roots()[0];
    assert_eq!(
        runtime.scene().element(root).unwrap(),
        morf_scene::Element::Column
    );
    let before = runtime.scene().children(root).unwrap().to_vec();
    assert_eq!(before.len(), 2);

    runtime.call_ipc("shuffle", &[]).unwrap();
    runtime.poll_services();
    let scene = runtime.scene();
    let after = scene.children(root).unwrap();
    let texts = after
        .iter()
        .map(|node| scene.string_value(*node, "text").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["B", "C"]);
    // Matched by value: the "b" row is the same node it was.
    assert_eq!(after[0], before[1]);
    drop(scene);

    runtime.call_ipc("rename", &[]).unwrap();
    runtime.poll_services();
    let scene = runtime.scene();
    let after = scene.children(root).unwrap();
    assert_eq!(scene.string_value(after[0], "text").unwrap(), "B2");
}

#[test]
fn a_reloadable_state_table_keeps_its_scalars_across_a_reload() {
    let source = br#"
        local morf = require("morf")
        local model = morf.state({ count = 1, who = { name = "a" }, rows = { "x" } }, { reloadable = "app" })
        morf.ipc.bump = function() model.count = model.count + 1; model.who.name = "b" end
        morf.ipc.read = function() return model.count, model.who.name end
    "#;
    let mut first = Runtime::default();
    first.execute("state-reload.lua", source).unwrap();
    first.call_ipc("bump", &[]).unwrap();
    let carried = first.reloadable_state();
    assert_eq!(carried.get("app.count"), Some(&IpcValue::Integer(2)));

    let mut second = Runtime::default();
    second.restore_reloadable_state(carried);
    second.execute("state-reload.lua", source).unwrap();
    assert_eq!(
        second.call_ipc("read", &[]).unwrap(),
        [IpcValue::Integer(2), IpcValue::String("b".into())]
    );
}
