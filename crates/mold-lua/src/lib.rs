//! Sandboxed execution of mold configuration code.

use std::error::Error as StdError;
use std::fmt;

use luna::{Closure, Executor, ExecutorMode, Fuel, Lua};

/// Execution limits applied independently to each loaded chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum VM fuel a chunk may consume.
    pub fuel: u64,
    /// Maximum bytes owned by the Lua state.
    pub memory: usize,
    /// VM fuel granted before the host regains control.
    pub slice_fuel: i32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 10_000_000,
            memory: 64 * 1024 * 1024,
            slice_fuel: 4_096,
        }
    }
}

/// A configuration execution failure.
#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    /// The source could not be compiled.
    Load(String),
    /// Execution stopped with a Lua error.
    Runtime(String),
    /// Execution exceeded its instruction budget.
    FuelExhausted { budget: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(message) => write!(f, "could not load Lua: {message}"),
            Self::Runtime(message) => write!(f, "Lua error: {message}"),
            Self::FuelExhausted { budget } => {
                write!(f, "Lua fuel exhausted after {budget} instructions")
            }
        }
    }
}

impl StdError for Error {}

/// The Luna VM owned behind mold's stable runtime boundary.
pub struct Runtime {
    lua: Lua,
    limits: Limits,
}

impl Runtime {
    /// Creates a sandboxed runtime with the supplied limits.
    pub fn new(limits: Limits) -> Self {
        let mut lua = Lua::core();
        lua.set_memory_limit(Some(limits.memory));
        Self { lua, limits }
    }

    /// Compiles and executes a Lua chunk.
    pub fn execute(&mut self, name: &str, source: &[u8]) -> Result<(), Error> {
        let executor = self
            .lua
            .try_enter(|ctx| {
                let closure = Closure::load(ctx, Some(name), source)?;
                Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
            })
            .map_err(|error| Error::Load(format!("{name}: {error}")))?;

        let slice_fuel = self.limits.slice_fuel.max(1);
        let mut remaining = self.limits.fuel;

        loop {
            if remaining == 0 {
                self.lua.enter(|ctx| ctx.fetch(&executor).stop(&ctx));
                return Err(Error::FuelExhausted {
                    budget: self.limits.fuel,
                });
            }

            let allowance = remaining.min(slice_fuel as u64) as i32;
            let mut fuel = Fuel::with(allowance);
            let finished = self
                .lua
                .enter(|ctx| ctx.fetch(&executor).step(ctx, &mut fuel))
                .map_err(|error| Error::Runtime(error.to_string()))?;
            let consumed = allowance.saturating_sub(fuel.remaining()).max(0) as u64;
            remaining = remaining.saturating_sub(consumed.max(1));

            if finished {
                break;
            }
        }

        let mode = self.lua.enter(|ctx| ctx.fetch(&executor).mode());
        if mode != ExecutorMode::Result {
            return Err(Error::Runtime(format!(
                "execution stopped in {mode:?} mode"
            )));
        }

        self.lua
            .execute::<()>(&executor)
            .map_err(|error| Error::Runtime(error.to_string()))
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_a_chunk() {
        let mut runtime = Runtime::default();
        runtime
            .execute("test.lua", b"local answer = 40 + 2")
            .unwrap();
    }

    #[test]
    fn reports_syntax_errors_with_the_source_name() {
        let mut runtime = Runtime::default();
        let error = runtime.execute("broken.lua", b"local =").unwrap_err();
        assert!(matches!(error, Error::Load(_)));
        assert!(error.to_string().contains("broken.lua"));
    }

    #[test]
    fn stops_an_infinite_loop_on_fuel_exhaustion() {
        let limits = Limits {
            fuel: 2_000,
            slice_fuel: 128,
            ..Limits::default()
        };
        let mut runtime = Runtime::new(limits);
        let error = runtime
            .execute("loop.lua", b"while true do end")
            .unwrap_err();
        assert_eq!(error, Error::FuelExhausted { budget: 2_000 });
    }
}
