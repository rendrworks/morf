use crate::paint::CachedLayout;

use mold_layout::Layout;

/// Builds a cached layout with known provenance.
fn cached_at(revision: u64, size: (u32, u32), scale_120: u32) -> CachedLayout {
    CachedLayout {
        layout: Layout::default(),
        revision,
        size,
        scale_120,
        input: Vec::new(),
    }
}

#[test]
fn a_layout_is_reused_only_while_everything_it_was_built_from_holds() {
    // Layout is the most expensive thing a frame does, and most frames change
    // nothing it reads. Reuse is therefore worth having — and worth being
    // exact about, because a layout reused after the geometry moved draws the
    // whole surface at stale positions.
    let cached = cached_at(7, (800, 600), 120);

    assert!(cached.still_valid(7, (800, 600), 120));

    // The scene moved something layout reads.
    assert!(!cached.still_valid(8, (800, 600), 120));
    // The surface resized; nothing in the scene had to change for that.
    assert!(!cached.still_valid(7, (801, 600), 120));
    assert!(!cached.still_valid(7, (800, 601), 120));
    // The compositor presents it at a new scale, so every length is different.
    assert!(!cached.still_valid(7, (800, 600), 180));
}
