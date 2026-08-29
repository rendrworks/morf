use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use mold_lua::Runtime;

fn usage() -> &'static str {
    "mold - reactive Wayland shell runtime\n\nusage: mold <shell.lua>\n       mold --help\n       mold --version"
}

fn run() -> Result<(), String> {
    let mut args = env::args_os();
    let _program = args.next();
    let Some(argument) = args.next() else {
        return Err(usage().to_owned());
    };
    if args.next().is_some() {
        return Err("mold accepts exactly one configuration path".to_owned());
    }

    if argument == "-h" || argument == "--help" {
        println!("{}", usage());
        return Ok(());
    }
    if argument == "-V" || argument == "--version" {
        println!("mold {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let path = PathBuf::from(argument);
    let source =
        fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    Runtime::default()
        .execute(&path.to_string_lossy(), &source)
        .map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mold: {error}");
            ExitCode::FAILURE
        }
    }
}
