use crate::pacing::FramePacer;
use std::time::Duration;

const REFRESH: Duration = Duration::from_micros(16_667);

#[test]
fn a_surface_that_keeps_up_paints_on_every_callback() {
    let mut pacer = FramePacer::new();
    // Nothing measured yet: take every callback until there is evidence.
    assert_eq!(pacer.interval(REFRESH), 1);
    assert!(pacer.due(REFRESH));

    pacer.observed(Duration::from_micros(4_000));
    assert_eq!(pacer.interval(REFRESH), 1);
    for _ in 0..10 {
        assert!(pacer.due(REFRESH), "a cheap frame never waits");
    }
}

#[test]
fn a_surface_that_cannot_keep_up_halves_its_rate_rather_than_missing_deadlines() {
    // The point of pacing: a frame that needs a refresh and a half will miss
    // every other deadline anyway. Choosing to paint every second callback
    // gives up the same frames and gets an even rhythm for them.
    let mut pacer = FramePacer::new();
    for _ in 0..12 {
        pacer.observed(Duration::from_micros(25_000));
    }
    assert_eq!(pacer.interval(REFRESH), 2);

    let painted: Vec<bool> = (0..8).map(|_| pacer.due(REFRESH)).collect();
    assert_eq!(
        painted,
        vec![true, false, true, false, true, false, true, false],
        "the first frame is taken, then an even cadence — not a burst and a gap"
    );
}

#[test]
fn the_cadence_follows_the_cost_and_is_bounded_at_both_ends() {
    let mut pacer = FramePacer::new();
    for _ in 0..20 {
        pacer.observed(Duration::from_micros(50_000));
    }
    assert_eq!(pacer.interval(REFRESH), 3, "three refreshes of work");

    // However slow it gets, it keeps painting sometimes.
    for _ in 0..20 {
        pacer.observed(Duration::from_secs(2));
    }
    assert_eq!(pacer.interval(REFRESH), 4, "capped rather than stopping");

    // And it recovers when the work does.
    for _ in 0..40 {
        pacer.observed(Duration::from_micros(2_000));
    }
    assert_eq!(pacer.interval(REFRESH), 1);
}

#[test]
fn a_frame_that_only_just_fits_does_not_halve_the_rate() {
    // Measurement is noisy and a refresh is not a hard wall. A frame a hair
    // over budget should keep trying for every callback rather than dropping
    // to half rate on the strength of a rounding error.
    let mut pacer = FramePacer::new();
    for _ in 0..20 {
        pacer.observed(REFRESH.mul_f64(1.05));
    }
    assert_eq!(pacer.interval(REFRESH), 1);

    // Clearly over, though, and it gives up the frame.
    for _ in 0..20 {
        pacer.observed(REFRESH.mul_f64(1.6));
    }
    assert_eq!(pacer.interval(REFRESH), 2);
}

#[test]
fn resting_clears_the_cadence_so_motion_restarts_on_the_next_callback() {
    // Between animations the surface is idle; when something moves again it
    // should paint at once rather than sit out callbacks it owed from before.
    let mut pacer = FramePacer::new();
    for _ in 0..12 {
        pacer.observed(Duration::from_micros(25_000));
    }
    assert!(pacer.due(REFRESH), "the first frame of motion is taken");
    assert!(!pacer.due(REFRESH), "mid-cadence");
    pacer.rest();
    assert!(
        pacer.due(REFRESH),
        "the first frame of new motion is not skipped"
    );
}
