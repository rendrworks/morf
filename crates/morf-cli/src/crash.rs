//! What is left behind when the shell dies.
//!
//! A shell is the thing drawing the screen, so when it faults there is often no
//! terminal watching and nothing to read afterwards: the panel vanishes and the
//! user is left with a bare compositor and no idea why. Until this existed morf
//! installed nothing — no panic hook, no backtrace, no report — and a fault in
//! the renderer left exactly that.
//!
//! Deliberately small. A crash handler that allocates a lot, takes locks, or
//! calls back into the engine is a crash handler that dies inside the crash it
//! is reporting, and then there is nothing at all. This writes one file and
//! prints one line.

use std::backtrace::Backtrace;
use std::fs;
use std::io::Write;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether a report has already been written this run.
///
/// A panic on one worker thread often knocks over the others, and three
/// simultaneous reports of the same fault are worse than one: the first is the
/// interesting one and the rest bury it.
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Installs the panic hook.
///
/// `MORF_DISABLE_CRASH_HANDLER` turns it off, for the case where an outer
/// harness — a debugger, a test runner, a coredump handler — wants the default
/// behaviour instead. `MORF_CRASH_CORE` asks for a core dump as well as the
/// report, subject to the usual `ulimit -c`.
pub(crate) fn install() {
    if std::env::var_os("MORF_DISABLE_CRASH_HANDLER").is_some() {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // The default hook first, so a terminal that *is* watching still sees
        // the ordinary message in the ordinary place.
        previous(info);
        if REPORTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let report = format(info);
        match write(&report) {
            Some(path) => {
                let _ = writeln!(
                    std::io::stderr(),
                    "morf: crash report written to {}",
                    path.display()
                );
                show_screen(&path);
            }
            None => {
                // Nowhere to write is not a reason to lose the backtrace; it is
                // a reason to put it where the default hook does not.
                let _ = writeln!(std::io::stderr(), "{report}");
            }
        }
        // A core, when asked for. A backtrace says where; a core says what
        // every variable held when it got there, and a fault in the renderer
        // is the kind where that is the difference between a fix and a guess.
        // Opt-in, because a core of a process holding a GPU context is large
        // and the kernel's own limits decide whether one appears at all.
        if std::env::var_os("MORF_CRASH_CORE").is_some() {
            // SAFETY: a plain signal to ourselves, with no memory arguments.
            unsafe { libc::abort() };
        }
    }));
}

/// Puts a crash screen up, when the session asked for one.
///
/// `MORF_CRASH_SCREEN` names a configuration -- `examples/crash.lua` is one --
/// and it is started as a new shell with the report's path as its argument.
/// Started through a shell with a one-second delay, because the process this
/// runs in is the one dying: it still holds the IPC socket, and a replacement
/// started this instant would find the display taken. A second later the
/// socket is stale and the replacement reclaims it. Opt-in, because a crash
/// screen on top of a crash is only right when there is a screen to draw on.
fn show_screen(report: &Path) {
    let Some(config) = std::env::var_os("MORF_CRASH_SCREEN") else {
        return;
    };
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let mut command = crash_screen_command(&executable, Path::new(&config), report);
    // Detached from this process entirely: no inherited stdio, no wait.
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = command.spawn();
}

/// The command that starts the crash screen, built where it can be checked.
pub(crate) fn crash_screen_command(
    executable: &Path,
    config: &Path,
    report: &Path,
) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        // `$0` is the executable, `$1` the configuration, `$2` the report:
        // nothing is interpolated into the script, so a path with a space or
        // a quote in it is still one argument.
        .arg("sleep 1; exec \"$0\" -d -- \"$1\" \"$2\"")
        .arg(executable)
        .arg(config)
        .arg(report);
    command
}

/// The report body.
fn format(info: &panic::PanicHookInfo<'_>) -> String {
    let what = info
        .payload()
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic with a payload of an unknown type".to_owned());
    let where_ = info
        .location()
        .map(ToString::to_string)
        .unwrap_or_else(|| "an unknown location".to_owned());
    format!(
        "morf {} panicked\n\
         at:      {where_}\n\
         thread:  {}\n\
         message: {what}\n\
         \n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::thread::current().name().unwrap_or("<unnamed>"),
        // Forced, because a shell is not usually run with `RUST_BACKTRACE` set
        // and a report without a stack is a report that says nothing.
        Backtrace::force_capture(),
    )
}

/// Writes the report, returning where it went.
fn write(report: &str) -> Option<PathBuf> {
    let directory = reports_dir()?;
    fs::create_dir_all(&directory).ok()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let path = directory.join(format!("crash-{stamp}-{}.log", std::process::id()));
    fs::write(&path, report).ok()?;
    Some(path)
}

/// Where reports are kept: `$XDG_STATE_HOME/morf/crashes`.
///
/// Not under the per-shell storage key the rest of the engine uses, because the
/// shell that would name that key is the one that just died — and at the point
/// this runs, reading anything out of it is exactly the sort of work that
/// panics again.
fn reports_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| Path::new(&home).join(".local").join("state"))
        })?;
    Some(base.join("morf").join("crashes"))
}
