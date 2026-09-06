use morf_lua::Runtime;
use morf_wayland::{LayerClient, PRIMARY_LAYER};
use std::time::Duration;

use crate::{surface_layers::*, surfaces::*};

/// How a surface decides which frame callbacks it can afford to paint on.
///
/// A worker paints when the compositor says it may, and on a machine that can
/// keep up that is exactly right. When it cannot — three fullscreen overlays on
/// one GPU, say — every worker still tries for every callback, they contend,
/// and the loser does not degrade to a slower steady rate: it misses deadlines
/// irregularly. A steady thirty reads as smooth; thirty that arrives in bursts
/// of sixty and gaps reads as stutter, which is worse than either.
///
/// So a surface that cannot paint inside one refresh deliberately paints on
/// every second callback, or every third, and keeps that cadence. It gives up
/// frames it was going to lose anyway, and gets an even rhythm in exchange.
#[derive(Debug)]
pub(crate) struct FramePacer {
    /// Smoothed cost of producing one frame.
    pub(crate) cost: Option<Duration>,
    /// Callbacks seen since the last paint, or `None` when the surface is at
    /// rest and the next callback should paint whatever the cadence was.
    pub(crate) waited: Option<u32>,
}

/// Weight given to the newest measurement, out of one.
///
/// Low enough that one slow frame — a first paint, a resize, a shader compiled
/// on demand — does not halve the cadence on its own, high enough to follow a
/// real change within a few frames.
pub(crate) const COST_SMOOTHING: f64 = 0.25;

impl FramePacer {
    pub(crate) fn new() -> Self {
        Self {
            cost: None,
            waited: None,
        }
    }

    /// Records what the last paint cost.
    pub(crate) fn observed(&mut self, cost: Duration) {
        self.cost = Some(match self.cost {
            None => cost,
            Some(previous) => previous.mul_f64(1.0 - COST_SMOOTHING) + cost.mul_f64(COST_SMOOTHING),
        });
    }

    /// Callbacks this surface lets pass between paints.
    ///
    /// One while it fits inside a refresh, two when it needs up to two, and so
    /// on. Capped, because a surface that has become very slow should keep
    /// painting occasionally rather than stop.
    pub(crate) fn interval(&self, refresh: Duration) -> u32 {
        const SLOWEST: u32 = 4;
        let Some(cost) = self.cost else {
            return 1;
        };
        if refresh.is_zero() {
            return 1;
        }
        let needed = cost.as_secs_f64() / refresh.as_secs_f64();
        // A frame that only just fits is not worth halving the rate for.
        (needed * 0.9).ceil().clamp(1.0, f64::from(SLOWEST)) as u32
    }

    /// Whether this callback is one the surface paints on.
    pub(crate) fn due(&mut self, refresh: Duration) -> bool {
        // A surface with no cadence yet — new, or just woken — paints at once.
        // Making the first frame of motion wait is the one delay nobody can
        // afford, because it is the one the eye is waiting for.
        let Some(waited) = self.waited else {
            self.waited = Some(0);
            return true;
        };
        if waited + 1 >= self.interval(refresh) {
            self.waited = Some(0);
            return true;
        }
        self.waited = Some(waited + 1);
        false
    }

    /// Forgets where in the cadence the surface was, for when it stops moving.
    pub(crate) fn rest(&mut self) {
        self.waited = None;
    }
}

/// One frame callback for the shell's own surface: motion advances, and the
/// pacer decides whether this callback is painted on.
pub(crate) fn primary_frame(
    runtime: &mut Runtime,
    client: &mut LayerClient,
    state: &mut SurfaceEventState,
    time_ms: u32,
) -> Result<bool, String> {
    let mut repaint = false;
    let delta = animation_delta(state.last_frame, time_ms);
    let frame = runtime
        .tick_animations(delta)
        .map_err(|error| error.to_string())?;
    // Carried forward only while motion continues, so the next run of
    // animation starts from a clean timebase rather than inheriting
    // however long the shell was idle.
    state.last_frame = frame.active.then_some(time_ms);
    // The callbacks themselves are the clock: whatever rate the
    // compositor offers this output is the rate to pace against.
    if !delta.is_zero() {
        state.refresh = delta;
    }
    // A shader reading the clock is motion like any other: it makes
    // the frame *advance*, and the pacer still decides which callbacks
    // are painted on. Forcing a repaint outside this path would spin as
    // fast as the event loop turns rather than at the output's rate.
    let advanced = frame.active || frame.changed > 0 || state.animating_shaders;
    if advanced {
        // A surface that cannot paint inside one refresh paints on
        // every second callback instead, and keeps that cadence rather
        // than missing deadlines at random.
        if state.pacer.due(state.refresh) {
            repaint = true;
        } else {
            // The next callback is asked for by painting, so a skipped
            // frame has to ask for itself — otherwise the compositor
            // has nothing outstanding, never calls back, and the
            // surface stops dead on the first frame it gives up.
            client.request_layer_frame(PRIMARY_LAYER);
            client.commit_layer(PRIMARY_LAYER);
        }
    } else {
        state.pacer.rest();
    }
    // Configured layer surfaces have no clock of their own; the shell's
    // tick is what tells them a repaint is due, and a surface that is
    // already idle needs a frame callback to come back on.
    if advanced {
        for surface in state.layer_surfaces.values_mut() {
            if surface.updates_enabled && !surface.needs_paint {
                surface.needs_paint = true;
                client.request_layer_frame(window_layer_id(surface.id));
            }
        }
    }

    Ok(repaint)
}
