use std::process::ExitCode;

mod config;
mod lock;
mod pacing;
mod paint;
mod services;
mod supervisor;
mod surface_actions;
mod surface_events;
mod surface_layers;
mod surface_popups;
mod surface_run;
mod surface_touch;
mod surfaces;
mod workers;

use config::*;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("morf: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
