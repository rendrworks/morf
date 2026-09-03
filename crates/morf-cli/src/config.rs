use morf_io::{IpcRequest, IpcValue as WireValue, ipc_call};
use morf_lua::{LogEntry, LogLevel};
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::{lock::*, supervisor::*};

pub(crate) fn usage() -> &'static str {
    "morf - reactive Wayland shell runtime\n\nusage: morf [--no-plugin | --clean] [-d | --daemonize] [shell.lua]\n       morf -i <display> <client command>\n       morf list [-j|--json] [--show-dead]\n       morf [--no-plugin | --clean] -c <name>\n       morf lock [lock.lua]\n       morf ipc call <target> [args...]\n       morf ipc verbs\n       morf log [-f|--follow] [--level <debug|info|warn|error>]\n       morf log --bindings\n       morf kill\n       morf --help\n       morf --version"
}

pub(crate) fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match parse_command(&args)? {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("morf {}", env!("CARGO_PKG_VERSION")),
        Command::Run(path, policy, arguments, daemonize) => {
            if daemonize {
                // Before anything else: a fork after threads exist takes only
                // the forking thread with it, and the supervisor is about to
                // start several.
                detach()?;
            }
            // Before anything is loaded, so the very first line of the very
            // first configuration can already ask what it was started with.
            morf_lua::arguments::install(arguments);
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            supervise(path, source, policy)?;
        }
        Command::Lock(path) => {
            let source = fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            run_lock(&path, &source)?;
        }
        Command::Log { follow, level } => follow_logs(follow, level)?,
        Command::List { json, show_dead } => list_instances(json, show_dead)?,
        Command::Client(request) => {
            let reply = ipc_call(socket_path()?, &request).map_err(|error| error.to_string())?;
            println!(
                "{}",
                String::from_utf8_lossy(&reply.to_wire().map_err(|e| e.to_string())?)
            );
        }
    }
    Ok(())
}

/// Prints the shell's log, once or until interrupted.
///
/// Following is a poll rather than a stream, and deliberately: the IPC is one
/// request and one reply by design, and a socket that pushes would be a second
/// protocol to get right. It works because reading the log *drains* it, so each
/// pass returns only what arrived since the last -- which is exactly what
/// following means.
fn follow_logs(follow: bool, level: LogLevel) -> Result<(), String> {
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

/// Reads the options `morf log` takes.
///
/// Its own function because it is the only subcommand with more than one, and
/// folding it into the match above would put four cases of flag parsing in the
/// middle of a table of shapes.
fn parse_log(rest: &[&str]) -> Result<Command, String> {
    let mut follow = false;
    let mut level = LogLevel::Debug;
    let mut rest = rest.iter();
    while let Some(argument) = rest.next() {
        match *argument {
            "-f" | "--follow" => follow = true,
            "--level" => {
                let Some(name) = rest.next() else {
                    return Err("--level wants a name".to_owned());
                };
                level =
                    LogLevel::parse(name).ok_or_else(|| format!("unknown log level `{name}`"))?;
            }
            other => return Err(format!("unknown option `{other}` for `morf log`")),
        }
    }
    Ok(Command::Log { follow, level })
}

#[derive(Debug)]
pub(crate) enum Command {
    Help,
    Version,
    /// The configuration to run, how much of the world to load, and the
    /// arguments that are the configuration's own rather than morf's.
    Run(PathBuf, LoadPolicy, Vec<String>, bool),
    Lock(PathBuf),
    /// Reading the shell's log, once or until interrupted.
    Log {
        follow: bool,
        level: LogLevel,
    },
    /// Every running instance on this machine.
    List {
        json: bool,
        show_dead: bool,
    },
    Client(IpcRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoadPolicy {
    pub(crate) plugins: bool,
    pub(crate) external_roots: bool,
}

impl Default for LoadPolicy {
    fn default() -> Self {
        Self {
            plugins: true,
            external_roots: true,
        }
    }
}

pub(crate) fn parse_command(args: &[std::ffi::OsString]) -> Result<Command, String> {
    let strings = args
        .iter()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "arguments must be UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Leading options, in any order and any combination. Each is about how
    // morf runs rather than what it runs, so they come before the command and
    // the command sees none of them.
    let mut policy = LoadPolicy::default();
    let mut daemonize = false;
    let mut strings = strings.as_slice();
    loop {
        match strings {
            ["--no-plugin", rest @ ..] => {
                policy.plugins = false;
                strings = rest;
            }
            ["--clean", rest @ ..] => {
                policy.plugins = false;
                policy.external_roots = false;
                strings = rest;
            }
            ["-d" | "--daemonize", rest @ ..] => {
                daemonize = true;
                strings = rest;
            }
            ["-i" | "--instance", display, rest @ ..] => {
                select_instance(display)?;
                strings = rest;
            }
            _ => break,
        }
    }
    match strings {
        ["-h" | "--help"] => Ok(Command::Help),
        ["-V" | "--version"] => Ok(Command::Version),
        ["-c", name, rest @ ..] => Ok(Command::Run(
            named_config_path(name)?,
            policy,
            own(rest),
            daemonize,
        )),
        ["lock"] => Ok(Command::Lock(config_root()?.join("lock.lua"))),
        ["lock", path] => Ok(Command::Lock(PathBuf::from(path))),
        ["ipc", "verbs"] => Ok(Command::Client(IpcRequest::Verbs)),
        ["ipc", "call", target, args @ ..] => Ok(Command::Client(IpcRequest::Call {
            target: (*target).to_owned(),
            args: args
                .iter()
                .map(|value| WireValue::String((*value).to_owned()))
                .collect(),
        })),
        ["log", "--bindings"] => Ok(Command::Client(IpcRequest::Bindings)),
        ["log", rest @ ..] => parse_log(rest),
        ["kill"] => Ok(Command::Client(IpcRequest::Kill)),
        ["list", rest @ ..] => parse_list(rest),
        // With nothing named, the environment may name it: `MORF_CONFIG` is
        // how a session file or a display manager points a shell at its
        // configuration without a command line to put it on.
        [] => Ok(Command::Run(
            match env::var_os("MORF_CONFIG") {
                Some(path) => PathBuf::from(path),
                None => config_root()?.join("shell.lua"),
            },
            policy,
            Vec::new(),
            daemonize,
        )),
        // A configuration whose own name begins with a dash, said explicitly.
        ["--", path, rest @ ..] => Ok(Command::Run(
            PathBuf::from(path),
            policy,
            own(rest),
            daemonize,
        )),
        // Everything after the configuration is the configuration's, but the
        // configuration itself still has to look like a path. Without this an
        // unknown flag becomes a filename, and `morf --colour` fails by saying
        // it could not read a file called `--colour` rather than that there is
        // no such option.
        [path, rest @ ..] if !path.starts_with('-') => Ok(Command::Run(
            PathBuf::from(path),
            policy,
            own(rest),
            daemonize,
        )),
        _ => Err(usage().to_owned()),
    }
}

/// Everything after the configuration belongs to the configuration.
///
/// morf takes the few arguments it needs to find the file at all and stops
/// looking. What the rest mean is not something a shell runtime can know: they
/// are addressed to whatever was written in Lua, and they are handed over
/// exactly as typed. A leading `--` is dropped, so `morf shell.lua -- --help`
/// asks the *shell* for help rather than morf.
fn own(rest: &[&str]) -> Vec<String> {
    let rest = match rest {
        ["--", after @ ..] => after,
        all => all,
    };
    rest.iter().map(|word| (*word).to_owned()).collect()
}

pub(crate) fn config_root() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("morf"));
    }
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/morf"))
        .ok_or_else(|| "HOME and XDG_CONFIG_HOME are unset".to_owned())
}

pub(crate) fn named_config_path(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err("config name must be one path component".to_owned());
    }
    Ok(config_root()?.join(name).join("shell.lua"))
}

/// The instance `-i` named, if any.
///
/// A process-wide choice rather than a parameter threaded through every
/// client command, the same way the configuration's own arguments are held:
/// it is decided once, before anything runs, and read from one place.
static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn select_instance(display: &str) -> Result<(), String> {
    if display.is_empty() || display.contains('/') {
        return Err("an instance is named by its WAYLAND_DISPLAY, one path component".to_owned());
    }
    INSTANCE
        .set(display.to_owned())
        .map_err(|_| "-i given twice".to_owned())
}

/// Where every instance keeps its socket: one file per `WAYLAND_DISPLAY`.
///
/// The directory is the registry. There is no separate list to keep in step
/// with reality; an instance is running exactly when its socket answers.
pub(crate) fn socket_dir() -> Result<PathBuf, String> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(|runtime| PathBuf::from(runtime).join("morf"))
        .ok_or_else(|| "XDG_RUNTIME_DIR is unset".to_owned())
}

pub(crate) fn socket_path() -> Result<PathBuf, String> {
    socket_path_for(INSTANCE.get().map(String::as_str))
}

/// The socket for a named instance, or for this display when none is named.
pub(crate) fn socket_path_for(display: Option<&str>) -> Result<PathBuf, String> {
    let display = match display {
        Some(display) => display.to_owned(),
        None => env::var("WAYLAND_DISPLAY").map_err(|_| "WAYLAND_DISPLAY is unset".to_owned())?,
    };
    if display.is_empty() || display.contains('/') {
        return Err("WAYLAND_DISPLAY must be one path component".to_owned());
    }
    Ok(socket_dir()?.join(format!("{display}.sock")))
}

/// Reads the options `morf list` takes.
fn parse_list(rest: &[&str]) -> Result<Command, String> {
    let mut json = false;
    let mut show_dead = false;
    for argument in rest {
        match *argument {
            "-j" | "--json" => json = true,
            "--show-dead" => show_dead = true,
            other => return Err(format!("unknown option `{other}` for `morf list`")),
        }
    }
    Ok(Command::List { json, show_dead })
}

/// Prints every instance on this machine, by asking each one who it is.
///
/// Dead sockets -- left by an instance that was killed rather than stopped --
/// are skipped unless asked for, because the ordinary question is "what is
/// running", and the answer to that should not include what is not.
fn list_instances(json: bool, show_dead: bool) -> Result<(), String> {
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

/// Puts the process in the background.
///
/// The classic double step: fork, and let the parent exit so the shell that
/// started us gets its prompt back; `setsid` so a hangup on that terminal is
/// not ours; stdio to `/dev/null` so nothing later blocks on a closed pipe.
/// Whatever the shell logs still reaches `morf log`, which is the point of
/// having that.
fn detach() -> Result<(), String> {
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
    // SAFETY: plain syscalls with no memory arguments.
    unsafe {
        libc::setsid();
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 0);
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            if null > 2 {
                libc::close(null);
            }
        }
    }
    Ok(())
}
