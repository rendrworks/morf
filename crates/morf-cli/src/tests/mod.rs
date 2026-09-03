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
    let Command::Run(path, policy, _) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert_eq!(path, PathBuf::from("custom.lua"));
    assert_eq!(policy, LoadPolicy::default());

    let args = ["--no-plugin", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy, _) = parse_command(&args).unwrap() else {
        panic!("expected config path");
    };
    assert!(!policy.plugins);
    assert!(policy.external_roots);

    let args = ["--clean", "custom.lua"].map(std::ffi::OsString::from);
    let Command::Run(_, policy, _) = parse_command(&args).unwrap() else {
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
    let Command::Run(path, policy, arguments) = parse_command(&args).unwrap() else {
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
    let Command::Run(_, _, arguments) = parse_command(&args).unwrap() else {
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
