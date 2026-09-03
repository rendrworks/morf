//! What `morf` does when it is not running a shell: reading the log, listing
//! instances, printing what a bug report needs, and going to the background.
//!
//! Split from `config` at the line gate, and a fair seam: that file decides
//! what was asked; this one does the asking.

use std::env;
use std::fs;
use std::path::PathBuf;

use morf_io::{IpcRequest, IpcValue as WireValue, ipc_call};
use morf_lua::{LogEntry, LogLevel};

use crate::config::{config_root, socket_dir, socket_path};

/// Prints the shell's log, once or until interrupted.
///
/// Following is a poll rather than a stream, and deliberately: the IPC is one
/// request and one reply by design, and a socket that pushes would be a second
/// protocol to get right. It works because reading the log *drains* it, so each
/// pass returns only what arrived since the last -- which is exactly what
/// following means.
pub(crate) fn follow_logs(follow: bool, level: LogLevel) -> Result<(), String> {
    loop {
        let reply = ipc_call(socket_path()?, &IpcRequest::Log).map_err(|e| e.to_string())?;
        if let Some(error) = &reply.error {
            return Err(error.clone());
        }
        for value in &reply.result {
            let WireValue::String(line) = value else {
                continue;
            };
            let entry = LogEntry::from_wire(line.as_str());
            if entry.level < level {
                continue;
            }
            println!("{:<5} {}", entry.level.name(), entry.message);
        }
        if !follow {
            return Ok(());
        }
        // Slow enough that an idle shell costs nothing, quick enough that a
        // person watching a reload does not wonder whether it is working.
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Prints every instance on this machine, by asking each one who it is.
///
/// Dead sockets -- left by an instance that was killed rather than stopped --
/// are skipped unless asked for, because the ordinary question is "what is
/// running", and the answer to that should not include what is not.
pub(crate) fn list_instances(json: bool, show_dead: bool) -> Result<(), String> {
    let dir = socket_dir()?;
    let mut entries = match fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "sock")
            })
            .collect::<Vec<_>>(),
        // No directory is no instances, not an error: nothing has run yet.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("could not read {}: {error}", dir.display())),
    };
    entries.sort();
    let mut rows = Vec::new();
    for socket in entries {
        let display = socket
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        match ipc_call(&socket, &IpcRequest::Info) {
            Ok(reply) if reply.ok => {
                let pid = match reply.result.first() {
                    Some(WireValue::Integer(pid)) => *pid,
                    _ => 0,
                };
                let config = match reply.result.get(1) {
                    Some(WireValue::String(config)) => config.clone(),
                    _ => String::new(),
                };
                let uptime = match reply.result.get(2) {
                    Some(WireValue::Integer(seconds)) => *seconds,
                    _ => 0,
                };
                rows.push((display, Some((pid, config, uptime))));
            }
            _ if show_dead => rows.push((display, None)),
            _ => {}
        }
    }
    if json {
        let items = rows
            .iter()
            .map(|(display, alive)| match alive {
                Some((pid, config, uptime)) => serde_json::json!({
                    "display": display, "pid": pid, "config": config, "uptime_s": uptime,
                }),
                None => serde_json::json!({ "display": display, "dead": true }),
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string(&items).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    for (display, alive) in rows {
        match alive {
            Some((pid, config, uptime)) => println!("{display}\t{pid}\t{uptime}s\t{config}"),
            None => println!("{display}\t-\t-\t(dead socket)"),
        }
    }
    Ok(())
}

/// Prints what a bug report needs: the machine, the display, the instance.
///
/// The half about this process is always printable; the half about the
/// running instance -- which GPU, which protocols -- is asked of it, and a
/// display with no instance says so rather than failing.
pub(crate) fn print_info() -> Result<(), String> {
    println!("morf {}", env!("CARGO_PKG_VERSION"));
    for name in [
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CURRENT_DESKTOP",
        "MORF_CONFIG",
        "MORF_PAM_LIBRARY",
    ] {
        println!(
            "{name}={}",
            env::var(name).unwrap_or_else(|_| "(unset)".to_owned())
        );
    }
    println!(
        "config_root={}",
        config_root()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    let socket = socket_path()?;
    println!("socket={}", socket.display());
    let Ok(reply) = ipc_call(&socket, &IpcRequest::Info) else {
        println!("instance=none (nothing is listening on that socket)");
        return Ok(());
    };
    if let [
        WireValue::Integer(pid),
        WireValue::String(config),
        WireValue::Integer(uptime),
    ] = reply.result.as_slice()
    {
        println!("instance_pid={pid}\ninstance_config={config}\ninstance_uptime_s={uptime}");
    }
    if let Ok(reply) = ipc_call(&socket, &IpcRequest::Capabilities) {
        for value in &reply.result {
            if let WireValue::String(line) = value {
                println!("{line}");
            }
        }
    }
    Ok(())
}

/// Puts the process in the background.
///
/// The classic double step: fork, and let the parent exit so the shell that
/// started us gets its prompt back; `setsid` so a hangup on that terminal is
/// not ours; stdin from `/dev/null` so nothing later blocks reading it.
///
/// Stdout and stderr go to a file, not the void. The first cut sent them to
/// `/dev/null`, and a daemon that died on its first line died silently --
/// `morf log` reads a running instance, and there was none to read. Anything
/// printed before the socket exists, and every panic after, lands in
/// `$XDG_STATE_HOME/morf/daemon.log` instead.
pub(crate) fn detach() -> Result<(), String> {
    // SAFETY: called before any thread exists, which is the one condition
    // under which forking a Rust process is sound.
    match unsafe { libc::fork() } {
        -1 => {
            return Err(format!(
                "could not fork: {}",
                std::io::Error::last_os_error()
            ));
        }
        0 => {}
        _ => std::process::exit(0),
    }
    let log = daemon_log_path().and_then(|path| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok()?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    // SAFETY: plain syscalls with no memory arguments; the file descriptors
    // are ours and stay open for the life of the process.
    unsafe {
        libc::setsid();
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 0);
            if log.is_none() {
                libc::dup2(null, 1);
                libc::dup2(null, 2);
            }
            if null > 2 {
                libc::close(null);
            }
        }
        if let Some(log) = log {
            use std::os::unix::io::IntoRawFd;
            let fd = log.into_raw_fd();
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
    Ok(())
}

/// Where a daemonised shell's stdout and stderr go.
fn daemon_log_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("morf").join("daemon.log"))
}
