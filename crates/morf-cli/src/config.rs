use morf_io::{IpcRequest, IpcValue as WireValue, ipc_call};
use morf_lua::{LogEntry, LogLevel};
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::{lock::*, supervisor::*};

pub(crate) fn usage() -> &'static str {
    "morf - reactive Wayland shell runtime\n\nusage: morf [--no-plugin | --clean] [shell.lua]\n       morf [--no-plugin | --clean] -c <name>\n       morf lock [lock.lua]\n       morf ipc call <target> [args...]\n       morf ipc verbs\n       morf log [-f|--follow] [--level <debug|info|warn|error>]\n       morf log --bindings\n       morf kill\n       morf --help\n       morf --version"
}

pub(crate) fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match parse_command(&args)? {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("morf {}", env!("CARGO_PKG_VERSION")),
        Command::Run(path, policy, arguments) => {
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

pub(crate) enum Command {
    Help,
    Version,
    /// The configuration to run, how much of the world to load, and the
    /// arguments that are the configuration's own rather than morf's.
    Run(PathBuf, LoadPolicy, Vec<String>),
    Lock(PathBuf),
    /// Reading the shell's log, once or until interrupted.
    Log {
        follow: bool,
        level: LogLevel,
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
    let (policy, strings) = match strings.as_slice() {
        ["--no-plugin", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: true,
            },
            rest,
        ),
        ["--clean", rest @ ..] => (
            LoadPolicy {
                plugins: false,
                external_roots: false,
            },
            rest,
        ),
        strings => (LoadPolicy::default(), strings),
    };
    match strings {
        ["-h" | "--help"] => Ok(Command::Help),
        ["-V" | "--version"] => Ok(Command::Version),
        ["-c", name, rest @ ..] => Ok(Command::Run(named_config_path(name)?, policy, own(rest))),
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
        [] => Ok(Command::Run(
            config_root()?.join("shell.lua"),
            policy,
            Vec::new(),
        )),
        // A configuration whose own name begins with a dash, said explicitly.
        ["--", path, rest @ ..] => Ok(Command::Run(PathBuf::from(path), policy, own(rest))),
        // Everything after the configuration is the configuration's, but the
        // configuration itself still has to look like a path. Without this an
        // unknown flag becomes a filename, and `morf --colour` fails by saying
        // it could not read a file called `--colour` rather than that there is
        // no such option.
        [path, rest @ ..] if !path.starts_with('-') => {
            Ok(Command::Run(PathBuf::from(path), policy, own(rest)))
        }
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

pub(crate) fn socket_path() -> Result<PathBuf, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "XDG_RUNTIME_DIR is unset".to_owned())?;
    let display = env::var("WAYLAND_DISPLAY").map_err(|_| "WAYLAND_DISPLAY is unset".to_owned())?;
    if display.is_empty() || display.contains('/') {
        return Err("WAYLAND_DISPLAY must be one path component".to_owned());
    }
    Ok(runtime.join("morf").join(format!("{display}.sock")))
}
