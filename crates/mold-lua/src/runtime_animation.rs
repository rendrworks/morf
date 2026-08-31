use std::time::Duration;

use mold_scene::AnimationFrame;

use crate::{api_animation::*, reactive_execute::*, surface_types::*, types::*};

// The animation frame tick and the Lua handlers it reports completions to.

impl Runtime {
    /// Advances animations entirely in Rust and reports the ones that ended.
    ///
    /// The tick itself runs no Lua. Only the `on_finished` handlers declared on
    /// behaviors are invoked afterwards, and a failing one is logged rather than
    /// allowed to abort the frame.
    pub fn tick_animations(&mut self, delta: Duration) -> Result<AnimationFrame, Error> {
        let frame = self
            .reactive
            .borrow_mut()
            .scene
            .tick_animations(delta)
            .map_err(|error| Error::Runtime(error.to_string()))?;
        if frame.events.is_empty() && frame.groups.is_empty() {
            return Ok(frame);
        }
        // A group callback is registered once and fires once, so it is taken
        // out of the map as it is collected rather than left to leak.
        let finished = {
            let mut state = self.reactive.borrow_mut();
            let mut finished = frame
                .events
                .iter()
                .filter_map(|event| {
                    let key = (event.node, event.property.to_owned());
                    let callback = state.animation_callbacks.get(&key)?.clone();
                    Some((callback, event.property.to_owned(), event.end, "behavior"))
                })
                .collect::<Vec<_>>();
            for event in &frame.groups {
                if let Some(callback) = state.group_callbacks.remove(&event.group) {
                    finished.push((callback, String::new(), event.end, "animation group"));
                }
            }
            finished
        };
        for (callback, property, end, source) in finished {
            let mut args = vec![IpcValue::String(animation_end_name(end).to_owned())];
            if !property.is_empty() {
                args.insert(0, IpcValue::String(property));
            }
            if let Err(message) = self
                .lua
                .enter(|ctx| execute_handler_args(ctx, &callback, &args, self.limits))
            {
                self.reactive
                    .borrow_mut()
                    .logs
                    .push(format!("{source} on_finished: {message}"));
            }
        }
        Ok(frame)
    }
}
