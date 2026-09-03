//! One flush per handler.
//!
//! A handler is any Lua the host runs in answer to something: a click, a
//! key, a timer, an IPC verb, a D-Bus call, a capture arriving. What the
//! handler writes -- signals, node properties -- is applied as it goes, and
//! the reactive graph is flushed once when the outermost handler returns.
//! Three signal writes are one flush; a bare `node.text = "x"` reaches the
//! bindings that read it before the next frame rather than at the next
//! unrelated write.

use luna::Context;

use crate::{reactive_bindings::*, types::*};

impl Runtime {
    /// Runs `body` in Lua as a handler, and flushes afterwards if it wrote.
    pub(crate) fn run_handler<T>(
        &mut self,
        body: impl for<'gc> FnOnce(Context<'gc>, Limits) -> T,
    ) -> T {
        self.reactive.borrow_mut().handler_depth += 1;
        let limits = self.limits;
        let result = self.lua.enter(|ctx| body(ctx, limits));
        let flush = {
            let mut state = self.reactive.borrow_mut();
            state.handler_depth = state.handler_depth.saturating_sub(1);
            state.handler_depth == 0 && std::mem::take(&mut state.flush_pending)
        };
        if flush
            && let Err(message) = self
                .lua
                .enter(|ctx| flush_reactive(&self.reactive, ctx, self.limits))
        {
            self.reactive
                .borrow_mut()
                .log(LogLevel::Warn, format!("after handler: {message}"));
        }
        result
    }
}
