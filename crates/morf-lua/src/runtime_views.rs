//! Views that follow their models, once a frame.

use crate::{types::*, views::*};

impl Runtime {
    /// Reconciles every view whose model changed since the last frame.
    ///
    /// Returns whether any did, which is a reason to repaint.
    pub(crate) fn sync_pending_views(&mut self) -> bool {
        let mut changed = false;
        // Views follow their models. A model that changed since the last frame
        // reconciles its delegates here, keyed by item identity, so a
        // `Repeater` whose model was replaced adds, drops and moves rows
        // rather than showing the list it was built from. Scrolling views
        // keep their current offset; `morf.sync_view` remains the way to
        // move it.
        let pending_views = self
            .reactive
            .borrow()
            .views
            .iter()
            .filter(|(_, view)| view.model.borrow().has_changes())
            .map(|(node, _)| *node)
            .collect::<Vec<_>>();
        for node in pending_views {
            let Some(mut view) = self.reactive.borrow_mut().views.remove(&node) else {
                continue;
            };
            let offset = view.view.offset();
            let result = self.lua.enter(|ctx| {
                reconcile_lua_view(&self.reactive, ctx, self.limits, node, offset, &mut view)
            });
            self.reactive.borrow_mut().views.insert(node, view);
            match result {
                Ok(_) => changed = true,
                Err(error) => self
                    .reactive
                    .borrow_mut()
                    .log(LogLevel::Warn, format!("view: {error}")),
            }
        }
        changed
    }
}
