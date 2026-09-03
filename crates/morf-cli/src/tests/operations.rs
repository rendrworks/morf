//! How the binary is operated: its options, its instances, its log, and
//! what it leaves behind when it dies.
//!
//! Split from `tests/mod.rs` at the line gate.

use super::*;

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
    assert!(report.contains("operations.rs"), "and where: {report}");
    assert!(
        report.contains("a_panic_leaves_a_report_behind"),
        "and carries a real backtrace naming this frame: {report}"
    );
    let _ = std::fs::remove_dir_all(&directory);
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
fn an_automatic_exclusive_zone_is_the_surfaces_own_extent_on_its_edge() {
    // quickshell's ExclusionMode.Auto. A bar that grows should push windows
    // with it, without the configuration keeping a number in step by hand.
    use crate::surfaces::auto_exclusive_zone;
    use morf_lua::LayerSurfaceConfig;
    let mut surface = LayerSurfaceConfig {
        height: 40,
        width: 300,
        margin_top: 6,
        ..LayerSurfaceConfig::default()
    };
    surface.anchors.top = true;
    surface.anchors.left = true;
    surface.anchors.right = true;
    assert_eq!(
        auto_exclusive_zone(&surface),
        46,
        "a top bar: its height and its gap"
    );
    surface.anchors.top = false;
    surface.anchors.right = false;
    assert_eq!(auto_exclusive_zone(&surface), 300, "a left dock: its width");
    surface.anchors.right = true;
    surface.anchors.top = true;
    surface.anchors.bottom = true;
    assert_eq!(
        auto_exclusive_zone(&surface),
        0,
        "spanning both edges reserves nothing"
    );
}
