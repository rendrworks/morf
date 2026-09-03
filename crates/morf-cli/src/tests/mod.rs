use crate::supervisor::execute_config;
use crate::supervisor::lua_snapshot;
use crate::supervisor::named_screens;
use crate::supervisor::runtimepath_roots;
use crate::surface_popups::window_surface_effectively_visible;
use crate::surfaces::primary_surface_root;
use morf_io::IpcRequest;
use morf_io::IpcValue as WireValue;
use morf_lua::Runtime;
use morf_wayland::ScreenInfo;
use morf_wayland::physical_size;
use std::fs;
use std::path::PathBuf;

use crate::*;

mod supervision;

use std::collections::{HashMap, HashSet};

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
                    local ui = require("morf.ui")
                    local window = require("morf.window")
                    local primary = ui.Item {}
                    local popup = ui.Item {}
                    window.popup { root = popup, width = 20, height = 10 }
                "#,
        )
        .unwrap();
    let primary = primary_surface_root(&runtime).unwrap();
    assert_eq!(runtime.scene().roots()[0], primary);
    assert_eq!(physical_size((101, 31), 150), (127, 39));
}

#[test]
fn child_window_visibility_follows_parent_chain() {
    let mut runtime = Runtime::default();
    runtime
        .execute(
            "window-parents.lua",
            br#"
                    local ui = require("morf.ui")
                    local window = require("morf.window")
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
    let Command::Run(path, policy, _, _) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert_eq!(path, PathBuf::from("custom.lua"));
    assert_eq!(policy, LoadPolicy::default());

    let args = ["--no-plugin", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy, _, _) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert!(!policy.plugins);
    assert!(policy.external_roots);

    let args = ["--clean", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy, _, _) = parse_command(&args).unwrap() else {
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
    let root = std::env::temp_dir().join(format!("morf-watch-{}", std::process::id()));
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
    let root = std::env::temp_dir().join(format!("morf-plugins-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::create_dir_all(root.join("after/plugin/nested")).unwrap();
    fs::write(root.join("plugin/first.lua"), b"plugin_value = 40").unwrap();
    fs::write(
        root.join("after/plugin/nested/last.lua"),
        b"assert(shell_value == 42); after_value = 43",
    )
    .unwrap();
    let shell = root.join("shell.lua");
    let source = b"assert(plugin_value == 40); shell_value = 42; morf.ui.Item {}";
    let mut runtime = Runtime::default();

    execute_config(&mut runtime, &shell, source, LoadPolicy::default()).unwrap();

    assert_eq!(runtime.scene().roots().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn plugin_failures_do_not_stop_later_plugins() {
    let root = std::env::temp_dir().join(format!("morf-plugin-errors-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::write(root.join("plugin/01-broken.lua"), b"error('broken')").unwrap();
    fs::write(root.join("plugin/02-working.lua"), b"plugin_value = 42").unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    execute_config(
        &mut runtime,
        &shell,
        b"assert(plugin_value == 42); morf.ui.Item {}",
        LoadPolicy::default(),
    )
    .unwrap();

    assert_eq!(runtime.scene().roots().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_plugin_policy_skips_discovered_plugins() {
    let root = std::env::temp_dir().join(format!("morf-no-plugin-{}", std::process::id()));
    fs::create_dir_all(root.join("plugin")).unwrap();
    fs::write(root.join("plugin/entry.lua"), b"plugin_loaded = true").unwrap();
    let shell = root.join("shell.lua");
    let mut runtime = Runtime::default();

    execute_config(
        &mut runtime,
        &shell,
        b"assert(plugin_loaded == nil); morf.ui.Item {}",
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
    let config = PathBuf::from("/tmp/morf-clean/shell.lua");

    assert_eq!(
        runtimepath_roots(&config, false),
        [PathBuf::from("/tmp/morf-clean")]
    );
}

/// Everything after the configuration belongs to the configuration. morf takes
/// what it needs to find the file and stops looking.
#[test]
fn arguments_after_the_configuration_are_the_configurations_own() {
    let args = ["--clean", "custom.lua", "--numbers-only", "-n", "5"].map(std::ffi::OsString::from);
    let Command::Run(path, policy, arguments, _) = parse_command(&args).unwrap() else {
        panic!("a configuration to run");
    };
    assert_eq!(path, std::path::PathBuf::from("custom.lua"));
    assert!(!policy.plugins);
    assert_eq!(arguments, ["--numbers-only", "-n", "5"]);
}

/// A leading `--` is morf getting out of the way, so a configuration can be
/// asked for its own help rather than morf answering for it.
#[test]
fn a_separator_hands_the_rest_over_untouched() {
    let args = ["custom.lua", "--", "--help"].map(std::ffi::OsString::from);
    let Command::Run(_, _, arguments, _) = parse_command(&args).unwrap() else {
        panic!("a configuration to run");
    };
    assert_eq!(arguments, ["--help"]);
}

/// An unknown option is an unknown option, not a filename. Passing the rest of
/// the line to the configuration must not turn a typo into "could not read
/// `--colour`".
#[test]
fn an_unknown_leading_option_is_still_refused() {
    let args = ["--colour", "red"].map(std::ffi::OsString::from);
    assert!(parse_command(&args).is_err());
}

#[test]
fn a_panic_leaves_a_report_behind() {
    // A shell is the thing drawing the screen, so when it faults there is
    // usually no terminal watching. Before this, morf installed no hook at all
    // and a renderer fault left the user with a vanished panel and nothing to
    // read.
    let directory = std::env::temp_dir().join(format!("morf-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    // SAFETY: single-threaded at this point in the test, and both variables are
    // read by the hook rather than by anything running concurrently.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", &directory);
        std::env::remove_var("MORF_DISABLE_CRASH_HANDLER");
    }
    crate::crash::install();

    let panicked = std::panic::catch_unwind(|| panic!("a deliberate fault"));
    assert!(panicked.is_err(), "the panic happened");

    let reports = directory.join("morf").join("crashes");
    let written = std::fs::read_dir(&reports)
        .expect("the crash directory was created")
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(written.len(), 1, "one report, not one per thread");
    let report = &written[0];
    assert!(
        report.contains("a deliberate fault"),
        "it says what: {report}"
    );
    assert!(report.contains("mod.rs"), "and where: {report}");
    assert!(
        report.contains("a_panic_leaves_a_report_behind"),
        "and carries a real backtrace naming this frame: {report}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn log_levels_order_and_survive_the_wire() {
    // Logging was a flat list of strings -- no level, no time, no filter -- so
    // a shell that had been running for a day gave you thousands of lines and
    // no way to ask which were serious, or recent.
    use morf_lua::{LogEntry, LogLevel};

    assert!(
        LogLevel::Debug < LogLevel::Warn,
        "levels compare, so a filter is a comparison rather than a set"
    );
    assert_eq!(LogLevel::parse("warning"), Some(LogLevel::Warn));
    assert_eq!(LogLevel::parse("shouty"), None);

    let entry = LogEntry {
        level: LogLevel::Warn,
        at_ms: 1_700_000_000_000,
        message: "a message with a : colon and a - dash".to_owned(),
    };
    let round_tripped = LogEntry::from_wire(&entry.to_wire());
    assert_eq!(round_tripped.level, LogLevel::Warn);
    assert_eq!(round_tripped.at_ms, entry.at_ms);
    assert_eq!(
        round_tripped.message, entry.message,
        "unit separators cannot occur in a message, so punctuation needs no \
         escaping and survives"
    );

    // A line that was never packed came from somewhere else, and losing it
    // would be worse than showing it without a level.
    let bare = LogEntry::from_wire("something printed the old way");
    assert_eq!(bare.level, LogLevel::Info);
    assert_eq!(bare.message, "something printed the old way");
}

#[test]
fn an_auxiliary_surface_is_addressed_by_its_own_kind() {
    // Fractional scale used to be a layer-surface privilege: a popup or a
    // floating window borrowed the primary layer's, which is right only while
    // they are on the same output. On a mixed-DPI desk they usually are not,
    // and the popup was drawn at the bar's scale and stretched.
    //
    // Each has its own now, and this is the join. It matters that the kind
    // travels with the number: identifiers do not share a space, so a layer
    // surface and a popup may both be `1`, and keying scale on the number alone
    // would have a popup's scale change resize a bar.
    use crate::paint::AuxiliaryKind;
    use morf_wayland::SurfaceRole;

    assert_eq!(AuxiliaryKind::Popup.role(1), SurfaceRole::Popup(1));
    assert_eq!(AuxiliaryKind::Floating.role(1), SurfaceRole::Floating(1));
    assert_ne!(
        AuxiliaryKind::Popup.role(1),
        SurfaceRole::Layer(1),
        "the same number, a different surface"
    );
}

#[test]
fn leading_options_combine_and_the_command_sees_none_of_them() {
    // `--no-plugin`, `--clean`, `-d` and `-i` are about how morf runs rather
    // than what it runs. They stack in any order, and by the time the command
    // is parsed they are gone.
    let args = ["-d", "--no-plugin", "shell.lua", "--numbers-only"].map(std::ffi::OsString::from);
    let Command::Run(path, policy, rest, daemonize) = parse_command(&args).unwrap() else {
        panic!("a run");
    };
    assert_eq!(path, std::path::PathBuf::from("shell.lua"));
    assert!(!policy.plugins);
    assert_eq!(rest, ["--numbers-only"]);
    assert!(daemonize, "and the shell was asked to go to the background");

    let args = ["--daemonize", "-c", "bar"].map(std::ffi::OsString::from);
    assert!(matches!(
        parse_command(&args),
        Ok(Command::Run(_, _, _, true))
    ));
}

#[test]
fn list_takes_its_two_flags_and_nothing_else() {
    let args = ["list"].map(std::ffi::OsString::from);
    assert!(matches!(
        parse_command(&args),
        Ok(Command::List {
            json: false,
            show_dead: false
        })
    ));
    let args = ["list", "--json", "--show-dead"].map(std::ffi::OsString::from);
    assert!(matches!(
        parse_command(&args),
        Ok(Command::List {
            json: true,
            show_dead: true
        })
    ));
    let args = ["list", "--verbose"].map(std::ffi::OsString::from);
    assert_eq!(
        parse_command(&args).unwrap_err(),
        "unknown option `--verbose` for `morf list`"
    );
}

#[test]
fn an_instance_is_named_by_its_display() {
    // `-i` picks which socket a client command talks to. The socket directory
    // is the registry -- one file per WAYLAND_DISPLAY -- so naming an instance
    // is naming a display.
    // SAFETY: this test alone touches XDG_RUNTIME_DIR, and reads it back
    // through the function under test rather than concurrently.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/morf-test") };
    assert_eq!(
        socket_path_for(Some("wayland-7")).unwrap(),
        std::path::PathBuf::from("/run/morf-test/morf/wayland-7.sock")
    );
    assert_eq!(
        socket_path_for(Some("../escape")).unwrap_err(),
        "WAYLAND_DISPLAY must be one path component"
    );
    let args = ["-i", "wayland-7", "kill"].map(std::ffi::OsString::from);
    assert!(matches!(
        parse_command(&args),
        Ok(Command::Client(IpcRequest::Kill))
    ));
    let args = ["-i", "wayland-7", "-i", "wayland-8", "kill"].map(std::ffi::OsString::from);
    assert_eq!(parse_command(&args).unwrap_err(), "-i given twice");
}

#[test]
fn the_crash_screen_is_started_as_a_shell_with_the_report_as_its_argument() {
    // A crash screen on top of a crash: the report exists, and now something
    // draws it. Started through `sh` with a delay because the dying process
    // still holds the socket, and with every path as its own argument so a
    // space in one cannot split it.
    let command = crate::crash::crash_screen_command(
        std::path::Path::new("/opt/morf bin/morf"),
        std::path::Path::new("/home/me/crash screen.lua"),
        std::path::Path::new("/tmp/report 1.log"),
    );
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(command.get_program(), "sh");
    assert_eq!(args[0], "-c");
    assert!(
        args[1].contains("sleep 1; exec \"$0\" -d -- \"$1\" \"$2\""),
        "{}",
        args[1]
    );
    assert_eq!(
        &args[2..],
        [
            "/opt/morf bin/morf",
            "/home/me/crash screen.lua",
            "/tmp/report 1.log"
        ]
    );
}
