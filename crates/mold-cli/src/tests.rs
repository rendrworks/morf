use super::*;

#[test]
fn named_screen_set_tracks_hotplug_identity() {
    let screens = [
        ScreenInfo {
            id: 7,
            name: Some("eDP-1".to_owned()),
            position: Some((0, 0)),
            size: Some((1920, 1080)),
            scale: 1,
            ..ScreenInfo::default()
        },
        ScreenInfo {
            id: 9,
            name: Some("DP-2".to_owned()),
            position: Some((1920, 0)),
            size: Some((2560, 1440)),
            scale: 2,
            ..ScreenInfo::default()
        },
    ];

    let names = named_screens(&screens).unwrap();

    assert_eq!(names.keys().cloned().collect::<Vec<_>>(), ["DP-2", "eDP-1"]);
    assert_eq!(names["DP-2"].id, 9);
}

#[test]
fn primary_root_excludes_registered_window_roots() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "window-roots.lua",
            br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    local primary = ui.Item {}
                    local popup = ui.Item {}
                    window.popup { root = popup, width = 20, height = 10 }
                "#,
        )
        .unwrap();
    let primary = primary_surface_root(&runtime).unwrap();
    assert_eq!(runtime.scene().roots()[0], primary);
    assert_eq!(auxiliary_physical_size(101, 31, 150), (127, 39));
}

#[test]
fn child_window_visibility_follows_parent_chain() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "window-parents.lua",
            br#"
                    local ui = require("mold.ui")
                    local window = require("mold.window")
                    local parent = window.floating {
                      root = ui.Item {}, visible = false,
                    }
                    local child = window.floating {
                      root = ui.Item {}, visible = true, parent = parent,
                    }
                    window.popup {
                      root = ui.Item {}, visible = true, parent = child,
                    }
                "#,
        )
        .unwrap();
    let surfaces = runtime.window_surface_configs();
    let by_id = surfaces
        .iter()
        .map(|surface| (surface.id, surface))
        .collect::<HashMap<_, _>>();

    assert!(!window_surface_effectively_visible(
        2,
        &by_id,
        &mut HashSet::new()
    ));
    runtime.set_window_surface_visible(0, true);
    let surfaces = runtime.window_surface_configs();
    let by_id = surfaces
        .iter()
        .map(|surface| (surface.id, surface))
        .collect::<HashMap<_, _>>();
    assert!(window_surface_effectively_visible(
        2,
        &by_id,
        &mut HashSet::new()
    ));
}

#[test]
fn command_parser_exposes_ipc_and_legacy_config_path() {
    let args = ["ipc", "call", "launcher.toggle", "one", "two"].map(std::ffi::OsString::from);
    let Command::Client(IpcRequest::Call { target, args }) = parse_command(&args).unwrap() else {
        panic!("expected IPC call");
    };
    assert_eq!(target, "launcher.toggle");
    assert_eq!(
        args,
        [
            WireValue::String("one".into()),
            WireValue::String("two".into())
        ]
    );

    let args = [std::ffi::OsString::from("custom.lua")];
    let Command::Run(path, policy) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert_eq!(path, PathBuf::from("custom.lua"));
    assert_eq!(policy, LoadPolicy::default());

    let args = ["--no-plugin", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert!(!policy.plugins);
    assert!(policy.external_roots);

    let args = ["--clean", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert!(!policy.plugins);
    assert!(!policy.external_roots);

    let args = ["lock", "secure.lua"].map(std::ffi::OsString::from);
    let Command::Lock(path) = parse_command(&args).unwrap() else {
        panic!("expected lock config path");
    };
    assert_eq!(path, PathBuf::from("secure.lua"));

    let args = ["log", "--bindings"].map(std::ffi::OsString::from);
    assert!(matches!(
        parse_command(&args).unwrap(),
        Command::Client(IpcRequest::Bindings)
    ));
}

#[test]
fn runtimepath_snapshot_tracks_nested_lua_changes() {
    let root = std::env::temp_dir().join(format!("mold-watch-{}", std::process::id()));
    let module = root.join("lua/plugin/widget.lua");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"return 1").unwrap();
    let before = lua_snapshot(std::slice::from_ref(&root));

    fs::write(&module, b"return 200").unwrap();
    let after = lua_snapshot(std::slice::from_ref(&root));

    assert_ne!(before, after);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_executes_plugins_before_shell_and_after_last() {
    let root = std::env::temp_dir().join(format!("mold-plugins-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::create_dir_all(root.join("after/plugin/nested")).unwrap();
    fs::write(root.join("plugin/first.lua"), b"plugin_value = 40").unwrap();
    fs::write(
        root.join("after/plugin/nested/last.lua"),
        b"assert(shell_value == 42); after_value = 43",
    )
    .unwrap();
    let shell = root.join("shell.lua");
    let source = b"assert(plugin_value == 40); shell_value = 42; mold.ui.Item {}";
    let mut runtime = Runtime::default();

    execute_config(&mut runtime, &shell, source, LoadPolicy::default()).unwrap();

    assert_eq!(runtime.scene().roots().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plugin_failures_do_not_stop_later_plugins() {
    let root = std::env::temp_dir().join(format!("mold-plugin-errors-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::write(root.join("plugin/01-broken.lua"), b"error('broken')").unwrap();
    fs::write(root.join("plugin/02-working.lua"), b"plugin_value = 42").unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    execute_config(
        &mut runtime,
        &shell,
        b"assert(plugin_value == 42); mold.ui.Item {}",
        LoadPolicy::default(),
    )
    .unwrap();

    assert_eq!(runtime.scene().roots().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_plugin_policy_skips_discovered_plugins() {
    let root = std::env::temp_dir().join(format!("mold-no-plugin-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::write(root.join("plugin/entry.lua"), b"plugin_loaded = true").unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    execute_config(
        &mut runtime,
        &shell,
        b"assert(plugin_loaded == nil); mold.ui.Item {}",
        LoadPolicy {
            plugins: false,
            external_roots: true,
        },
    )
    .unwrap();

    assert_eq!(runtime.scene().roots().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn clean_policy_keeps_only_the_config_root() {
    let config = PathBuf::from("/tmp/mold-clean/shell.lua");

    assert_eq!(
        runtimepath_roots(&config, false),
        [PathBuf::from("/tmp/mold-clean")]
    );
}

#[test]
fn plugin_path_preserves_root_order() {
    let base = std::env::temp_dir().join(format!("mold-plugin-order-{}", std::process::id()));
    let first = base.join("z-first/plugin");
    let second = base.join("a-second/plugin");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(first.join("entry.lua"), b"").unwrap();
    fs::write(second.join("entry.lua"), b"").unwrap();

    let scripts = runtime_scripts(&[base.join("z-first"), base.join("a-second")], "plugin");

    assert!(scripts[0].starts_with(base.join("z-first")));
    assert!(scripts[1].starts_with(base.join("a-second")));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn successful_reload_carries_opt_in_state() {
    let screen = Screen {
        name: "test".into(),
        width: None,
        height: None,
        scale: 1,
        ..Screen::default()
    };
    let source = br#"
            local value = mold.reloadable("counter", 0)
            local completed = false
            mold.on_reload_completed(function() completed = true end)
            mold.ipc["counter.set"] = function(next) value:set(next) end
            mold.ipc["counter.get"] = function() return value:get() end
            mold.ipc["reload.completed"] = function() return completed end
            mold.ui.Item {}
        "#;
    let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
    runtime.execute("shell.lua", source).unwrap();
    runtime
        .call_ipc("counter.set", &[IpcValue::Integer(7)])
        .unwrap();
    let (reply, result) = mpsc::sync_channel(1);

    let update = handle_worker_command(
        &mut runtime,
        &screen,
        LoadPolicy::default(),
        WorkerCommand::Reload {
            path: Arc::new(PathBuf::from("shell.lua")),
            source: Arc::from(&source[..]),
            hard: false,
            reply,
        },
    );

    assert!(result.recv().unwrap().is_ok());
    assert!(update.repaint);
    assert!(!update.recreate_surface);
    assert_eq!(
        runtime.call_ipc("counter.get", &[]).unwrap(),
        [IpcValue::Integer(7)]
    );
    assert_eq!(
        runtime.call_ipc("reload.completed", &[]).unwrap(),
        [IpcValue::Boolean(true)]
    );
}

#[test]
fn hard_reload_discards_opt_in_state() {
    let screen = Screen {
        name: "test".into(),
        width: None,
        height: None,
        scale: 1,
        ..Screen::default()
    };
    let source = br#"
            local value = mold.reloadable("counter", 0)
            mold.ipc["counter.set"] = function(next) value:set(next) end
            mold.ipc["counter.get"] = function() return value:get() end
            mold.ui.Item {}
        "#;
    let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
    runtime.execute("shell.lua", source).unwrap();
    runtime
        .call_ipc("counter.set", &[IpcValue::Integer(7)])
        .unwrap();
    let (reply, result) = mpsc::sync_channel(1);

    let update = handle_worker_command(
        &mut runtime,
        &screen,
        LoadPolicy::default(),
        WorkerCommand::Reload {
            path: Arc::new(PathBuf::from("shell.lua")),
            source: Arc::from(&source[..]),
            hard: true,
            reply,
        },
    );

    assert!(result.recv().unwrap().is_ok());
    assert!(update.repaint);
    assert!(update.recreate_surface);
    assert_eq!(
        runtime.call_ipc("counter.get", &[]).unwrap(),
        [IpcValue::Integer(0)]
    );
}

#[test]
fn failed_reload_keeps_the_previous_runtime() {
    let screen = Screen {
        name: "test".into(),
        width: None,
        height: None,
        scale: 1,
        ..Screen::default()
    };
    let mut runtime = Runtime::for_screen(Limits::default(), screen.clone());
    runtime
        .execute(
            "shell.lua",
            br#"
                    local failure = ""
                    mold.on_reload_failed(function(error) failure = error end)
                    mold.ipc.value = function() return 7 end
                    mold.ipc["reload.failure"] = function() return failure end
                    mold.ui.Item {}
                "#,
        )
        .unwrap();
    let (reply, result) = mpsc::sync_channel(1);

    let update = handle_worker_command(
        &mut runtime,
        &screen,
        LoadPolicy::default(),
        WorkerCommand::Reload {
            path: Arc::new(PathBuf::from("shell.lua")),
            source: Arc::from(&b"local ="[..]),
            hard: false,
            reply,
        },
    );

    assert!(result.recv().unwrap().is_err());
    assert!(!update.repaint);
    assert!(!update.recreate_surface);
    assert_eq!(
        runtime.call_ipc("value", &[]).unwrap(),
        [IpcValue::Integer(7)]
    );
    let failure = runtime.call_ipc("reload.failure", &[]).unwrap();
    assert!(matches!(&failure[..], [IpcValue::String(error)] if !error.is_empty()));
}

#[test]
fn supervisor_dispatches_registered_ipc_handler() {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (commands, rx) = mpsc::channel();
    let join = thread::spawn(move || {
        let mut runtime = Runtime::default();
        runtime
            .execute(
                "ipc.lua",
                b"mold.ipc.echo = function(value) return value, 2 end",
            )
            .unwrap();
        while !worker_stop.load(Ordering::Acquire) {
            if let Ok(command) = rx.recv_timeout(Duration::from_millis(10)) {
                handle_worker_command(
                    &mut runtime,
                    &Screen {
                        name: "test".into(),
                        width: None,
                        height: None,
                        scale: 1,
                        ..Screen::default()
                    },
                    LoadPolicy::default(),
                    command,
                );
            }
        }
    });
    let workers = BTreeMap::from([(
        "test".to_owned(),
        Worker {
            stop,
            commands,
            join,
            screen: ScreenInfo {
                name: Some("test".to_owned()),
                ..ScreenInfo::default()
            },
        },
    )]);

    let reply = handle_ipc(
        &workers,
        &mut Vec::new(),
        &IpcRequest::Call {
            target: "echo".into(),
            args: vec![WireValue::String("ready".into())],
        },
    );
    assert_eq!(
        reply,
        IpcReply::success(vec![
            WireValue::String("ready".into()),
            WireValue::Integer(2)
        ])
    );

    let (reload, result) = mpsc::sync_channel(1);
    workers["test"]
        .commands
        .send(WorkerCommand::Reload {
            path: Arc::new(PathBuf::from("shell.lua")),
            source: Arc::from(&b"this is not lua"[..]),
            hard: false,
            reply: reload,
        })
        .unwrap();
    assert!(
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_err()
    );
    let reply = handle_ipc(
        &workers,
        &mut Vec::new(),
        &IpcRequest::Call {
            target: "echo".into(),
            args: vec![WireValue::String("last-good".into())],
        },
    );
    assert_eq!(
        reply,
        IpcReply::success(vec![
            WireValue::String("last-good".into()),
            WireValue::Integer(2)
        ])
    );
    stop_workers(workers);
}
