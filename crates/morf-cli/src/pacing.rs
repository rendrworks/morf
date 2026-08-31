use std::time::Duration;

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
