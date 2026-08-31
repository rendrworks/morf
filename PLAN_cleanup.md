# Cleanup plan

Every place in `mold` where the same idea is implemented twice, where something is
built in a way that will bite, or where code exists that nothing reaches.

**Method.** Eight agents swept the tree in parallel — one per subsystem, one
cross-cutting — each required to prove every claim by opening the cited line, and
each gated behind an adversarial verifier told to refute by default and to reject
any finding whose citations did not check out. A completeness critic then went
looking for what the eight had missed. Alongside that, a mechanical pass hashed
every 8-line window of normalised production code to find copy-paste the readers
would not notice.

**Result.** 80 findings survived verification; 1 was refuted; 2 corrections were
made to the previous version of this plan. 20 are high severity. Every `file:line`
below was verified against the tree at the time of writing.

**How to read it.** Divergences first, because those are bugs. Then bad
implementations, parallel implementations, dead code, repetition. Within each,
highest severity first. The last two sections are the ones no per-file review
could see: the gaps outside `crates/*/src` entirely, and the single structural
decision that produced most of the rest.

---

## Implementation status

**76 of the 80 findings are implemented.** Every high-severity finding is done.
Gates after each batch: **332 CPU tests** (from 319 — thirteen added, each one
checked to fail against the unfixed code where that was possible), **19 GPU
tests** (from 13), and `boundary-check`, `rust-loc-check`, `fmt-check` and
`clippy -D warnings` green. All 16 shipped configurations load and paint.

### What is not done, and why

Four findings are deliberately not implemented. Each is named here rather than
left to be inferred from the list above.

**`glyph.wgsl`'s `mode.z` branch — the finding's premise no longer holds.** It
said no producer could set `mode.z` without also setting `mode.w`. That was true
when the sweep ran; the SDF-text work in this same session changed it.
`gpu/glyph_batch.rs:150` now emits `z = 1, w = 0` for mask glyphs, which is
exactly the combination said to be impossible, so the branch is reachable and
deleting it would break coverage-atlas glyphs. Left in place.

**Padding for image distance fields (half of the two-generators finding).** The
glyph generator pads its source so an outline has room outside the shape; the
image one does not. Padding it would change the image's dimensions, and the
texture quad is the node's own rectangle — the glyph path can pad only because
it sizes its quad from the field. Making the image path do the same is a design
change to texture placement, not a cleanup. The divergence and its consequence
are now documented at `mold-image/src/distance_field.rs`; the shared
normalisation half is not extracted because `mold-image` and `mold-text` are
both dependency-free leaves with no crate below them, the same obstacle that
kept `physical_size` at two implementations.

**The two big design merges** — one shape vocabulary, one field pipeline.
Unchanged from the reasoning below: a pipeline merge is the change most likely
to regress the whole shell's appearance, and wants GPU tests written before it
rather than after.

**`include!` → `mod` is done.** All 114 sites across the nine crates are real
modules now, each with its own imports and its own visibility. Section "The
structural cause" below describes why this was the mechanism rather than a
symptom; what it actually cost is recorded there. The conversion is what made
the next paragraph's numbers possible: the flat namespace could not report an
unused import, so **413 of them** had been accumulating invisibly, and every one
is now gone. Two trait imports (`std::io::{Read, Write}`) and one type import
looked unused to the compiler's own diagnostic and were not — method resolution
needs the trait in scope — so every removal was re-checked against a build.

**Findings deliberately out of scope.** The PipeWire round-trip that blocks the
shell thread, and the D-Bus per-frame drain, are not implemented: the user
scoped audio and the service bus out of this branch explicitly. They are real
findings and stay listed below; they are not open questions.

**Two medium findings remain open** and are listed above unchanged:
`mold.flickable` as a second momentum mechanism, and the reserve layers
recreated rather than moved. Each is a behaviour change rather than a cleanup,
and each deserves its own decision.

### The `-Dwarnings` gap, and why no gate was added

`unused_crate_dependencies` is the lint that would have caught `svgtypes`. It
reports per-target, so under CI's `--all-targets` it produces about 105 warnings
from examples that legitimately do not use every dependency — it would red the
gate. A `make deps-check` recipe was written to run it over library targets
only, and then **deleted**: `RUSTFLAGS` does not activate the lint the way the
manifest `[lints]` table does, so the recipe passed while checking nothing. That
is precisely the failure repaired in `boundary-check` this same session (see
below), and shipping a second instance of it would have been worse than shipping
no gate. The two real dependency defects the lint found while it was briefly
active — `svgtypes`, and three `mold-wayland` dependencies used only by its
examples — are fixed.

### Bugs a configuration could trigger

| what | pinned by |
|---|---|
| Corner radius clamped to the half-extent everywhere, so `radius = 9999` is a capsule through all three paths | `an_oversized_corner_radius_makes_a_capsule_through_every_path` |
| A fling points `target` at where it stopped, so a later write home is not silently dropped | `a_property_can_be_written_back_to_where_it_was_before_it_was_flung` |
| `animate_from` invalidates the layout it moved | `a_zero_duration_animation_announces_the_geometry_it_moved` |
| Damage keyed by node *occurrence*, so a `ClipRect`'s fill reaches the differ | `a_clip_rect_repaints_when_only_its_fill_changes` — verified to fail without the fix |
| `next_event` honours its timeout after the pipes close instead of spinning and losing the exit | `a_finished_process_reports_its_exit_rather_than_an_empty_poll` |
| D-Bus property reads accept `o` and `g` | `a_property_holding_an_object_path_is_readable` — verified to fail without the fix |
| `field_area` accounts for rotation, so a rotated bar is not sliced flat by its own quad | `a_rotated_layer_is_not_sliced_flat_by_its_own_quad` — verified to fail without the fix |
| A group refuses both alternating forms instead of silently running once | `a_group_refuses_both_ways_of_asking_it_to_alternate` |
| `mold-image` widens before dividing, so a huge request is refused rather than resized | `a_huge_request_is_refused_rather_than_silently_resized` |
| One budget across a whole `mold.variants` call, not one per entry | `a_runaway_variant_factory_is_cut_off_once_not_once_per_entry` |
| `ui.spring`/`ui.smoothed` copy rather than mutate the caller's table | `a_shared_behavior_table_can_be_both_a_spring_and_a_smoothing` |
| An easing curve interpolates between an integer and a float | `an_easing_curve_interpolates_between_an_integer_and_a_float` |
| Replacing one kind of motion with another always says so | `replacing_a_behavior_with_physics_says_the_animation_was_canceled` |
| Pooled layer targets do not carry the previous frame into this one | `a_reused_layer_target_does_not_carry_the_last_frame_into_this_one` |

`next_event` was not on the list. It surfaced as a test failing about one run in
three, and turned out to be a product bug rather than a flaky test: a
disconnected channel makes `recv_timeout` return instantly, so a configuration
polling `process:next(1000)` after output ended spun a core and could miss the
exit entirely. Twelve consecutive suite runs are now clean.

Also fixed without a dedicated test: the lock screen now shares the real key
handler, so it has Tab traversal and focus memory — on the one surface whose
purpose is accepting a password; a scale change on a configured layer surface
repaints instead of resizing a swapchain and leaving the old picture in it; and
field layer colours take the inherited tint that the field's own fill already
took, so tinting a subtree no longer changes half a field.

### Leaks and unbounded growth

- **One node-destruction signal now crosses the crate boundary.** `Scene` records
  what it destroys, `mold-cli` drains it once a frame and hands it to the render
  backend. Closes the `TextSystem::buffers` leak — a full shaped cosmic-text
  buffer per Text node *ever measured* — and the `TransformTracker` one, and
  gives `retain_scene` its first caller.
  `a_destroyed_node_is_reported_once_to_whoever_holds_state_for_it`.
- **The two size-keyed caches are bounded**, so an animated icon width no longer
  mints a decode and a GPU texture per pixel step.
  `decoding_many_sizes_does_not_grow_the_cache_without_end`.

### Duplication removed

`sd_box` 3 → 1 (a `shape.wgsl` prelude concatenated at pipeline creation, since
WGSL has no include) · the full-screen triangle 3 → 1 · the fuel-metered
executor loop 10 → 1, closing both drifts · logical-to-physical 3 → 1 shared
plus one deliberate leaf · `paint_popup_surface`/`paint_floating_surface` 52
duplicated lines → 1 · the two D-Bus encoders → 1 · the field reach rectangle
2 → 1 (which is how the rotation fix reached damage as well) · the icon
resolve-or-cache 3 → 1 · the window-surface registration 3 → 1 · the
service-request list 6 → 1, closing two subsets that disagreed · the event-name
table 2 → 1 · the surface-cleanup path 3 → 1 · `InputRect` → `mold_region::Rect`
· `ScriptValue` → `IpcValue` · the whole-scene keyboard lookup deleted in favour
of the scoped one.

### Config API made consistent

One name for surface visibility · one size rule for both axes and both doors,
with zero meaning compositor-sized · one validator for reloadable IDs, applied
where the map is written so no door can skip it · one stream framer · one
quantizer · `thickness`/`softness`/`outline_width`/`outline_color` spelled the
same on Text, Image and Icon, with `thickness` meaning one thing rather than two.

### Performance

Layer render targets pooled instead of a full-screen GPU texture per layer per
frame · the `DrawList` swapped rather than deep-cloned every frame · glyph field
bitmaps shared rather than copied per visible glyph per frame · `merge_damage`
no longer shifts its tail on every merge · `StreamCollector`'s O(n²) second
buffer removed · the udev read buffer allocated once instead of zeroing 64 KiB
per call · configured layer surfaces painted once per tick rather than twice ·
the configured input mask deduped like its sibling branch · the primary surface
root resolved once rather than re-derived on every repaint and key press · the
auxiliary layout cache kept unless what it depends on changed · `PropertyClass`
deleted, having been computed per property per frame for a field nobody read.

### Build, CI and dead code

The release workflow builds `--bin mold`, which exists, rather than
`--example main`, which never did — verified by building it; macOS targets
dropped from a Wayland shell. `svgtypes` removed and three `mold-wayland`
dependencies moved to dev-dependencies. Ten dead public functions deleted, plus
`Graph::effect` and the internal effect half it was the only producer for — and
`mold-reactive`'s tests, which exercised only that dead API, now drive the
external path production actually uses, which surfaced `Graph::batch` being
unable to evaluate an external effect at all. Two produced-and-discarded layer
events, the write-only reposition record, three unused `PRIMARY_LAYER` wrappers,
the dead `"path"` match arm, the one-byte `mold-cli/src/lib.rs` that published an
empty lib target, and the unreferenced root `tests/` tree.

**`boundary-check` was repaired.** Removing `tests/` made its grep fail on a
missing path — and because the gate asserts on the grep *failing*, it began
passing for the wrong reason. It is now proven to still catch a real leak.

**`frame_bench` can open the flagship config.** It built a screenless runtime, so
any configuration using the documented `mold.variants(mold.screens, …)` idiom
produced no root and it panicked. `.make.lua`'s `run` target also pointed at a
fixture that no longer existed.

---

## Corrections to the previous plan

The earlier version of this document was written from a single-pass read. Two of
its claims were wrong, and the first one matters:

**A1 was mis-ranked.** It said the divergence between the two `rounded_distance`
copies was the missing `radius = max(radius, 0.0)` in `glyph.wgsl`, and called it
unreachable. That part is correct but irrelevant: `rect_radii`
(`crates/mold-render/src/effects.rs:198`) already clamps the radius to `>= 0`
before it reaches either shader, so the line is a no-op in one and harmlessly
absent in the other.

The divergence that *is* reachable was missed entirely. Neither `sdf.wgsl` nor
`glyph.wgsl` clamps the corner radius to the box half-extent, while
`field.wgsl:105` (`r = min(r, min(half.x, half.y))`) and
`mold-region/src/lib.rs:343` (`.min(width / 2.0).min(height / 2.0)`) both do. So
the ordinary pill idiom — `radius = 9999` on a wide short box — paints a **square**
through the `Rect` path and a **capsule** through the field and input-region paths.
Same numbers, three different shapes. It is config-reachable today and it is now
finding 1 below.

**A1's fix was understated.** Adopting `field.wgsl`'s `sd_box` as the shared
implementation is not a pure refactor, as claimed — it is the only one of the three
that clamps to the half-extent, so unifying on it *changes how `Rect` and
`ClipRect` render* for radii above half the box. That is the correct behaviour and
it matches what the other two paths already do, but it needs a test at
`radius > half-extent` and a note in the commit.

Everything else in the previous plan survived: A1's core duplication claim, B1–B6,
C1–C3, D1, E1–E2, F1–F2 are all confirmed and are folded into the sections below.

---
## Divergence — the same idea implemented twice, and the copies disagree

These are latent or live bugs, not tidiness. Each one is a place where two code paths that must agree do not.

_19 findings — 7 high, 8 medium, 4 low._

#### 1. Four copies of the lay-out-render-commit routine; the lock screen's copy recomputes layout every frame because it is the one that never got the CachedLayout optimization

*high · certain · cross-cutting*

`crates/mold-cli/src/lock.rs:216`, `crates/mold-cli/src/lock.rs:277`, `crates/mold-cli/src/lock.rs:206-212`

**Costs.** Four copies of one routine means every optimization and every fix has to be applied four times, and demonstrably has not been. The lock screen is the worst-affected: it is the copy that runs a 1Hz clock and animations over a fullscreen surface, and it is the only one that recomputes the layout — described three files over as "the most…

**Fix.** Collapse `paint_popup_surface` and `paint_floating_surface` into one function taking the surface kind (they already share `AuxiliarySurface`; the four differences are one enum match or two closures). Make both call `CachedLayout::still_valid` rather than re-inlining it. Then give `paint_lock` a `CachedLayout` and route it through the…

#### 2. The lock screen's key handler is a divergent copy of the surface key handler: no Tab focus traversal and no focus persistence, on the one surface that exists to accept a password

*high · certain · cross-cutting*

`crates/mold-cli/src/lock.rs:135`, `crates/mold-cli/src/surface_events.rs:320`, `crates/mold-lua/src/runtime_events.rs:242`, `crates/mold-lua/src/runtime_events.rs:299`

**Costs.** This is the divergence with the worst blast radius, because the affected surface is a session lock. A lock config with a user field and a password field, or with a Cancel button, is unreachable by keyboard past the first key handler — and a session lock is exactly where a keyboard-only user cannot fall back to the pointer. The two…

**Fix.** Extract the surface_events.rs:320-351 body into one function taking `(root, focused_slot, keysym, text)` and call it from both event loops. `runtime.first_key_target_in(root)` / `next_key_target_in(root, current)` already accept the lock root, so the lock loop only needs to carry a `focused: Option<NodeHandle>` local instead of a HashMap.

#### 3. Corner radius larger than half the box: sdf.wgsl renders a plain square, field.wgsl and mold-region render a circle

*high · certain · geometry*

`crates/mold-render/src/sdf.wgsl:92`, `crates/mold-render/src/sdf.wgsl:94`, `crates/mold-render/src/glyph.wgsl:69`, `crates/mold-render/src/field.wgsl:105`, `crates/mold-region/src/lib.rs:343` (+1 more)

**Costs.** `radius = 9999` is the standard idiom for "make this a pill" in every CSS/QML-shaped language, and in mold it silently produces the opposite: a hard-cornered rectangle. Worse, the same numbers describe a circle to the Sdf pipeline and to the input-region rasteriser, so a rounded Rect and the input mask meant to match it disagree about…

**Fix.** Add `radius = min(radius, min(size.x, size.y) * 0.5);` to sdf.wgsl:92 and glyph.wgsl (after the per-corner select), or — better, and what A1 already proposes — collapse all three onto field.wgsl's `sd_box` via a shared WGSL prelude, since field.wgsl's version is the only one that already gets both clamps right. Keep mold-region's…

#### 4. A fling leaves `target` stale forever, and `assign`'s target-equality short-circuit then silently drops an assignment back to the pre-fling value

*high · certain · scene*

`crates/mold-scene/src/scene.rs:184`, `crates/mold-scene/src/scene.rs:298`, `crates/mold-scene/src/scene.rs:352`, `crates/mold-scene/src/fling.rs:58`, `crates/mold-scene/src/scene.rs:341`

**Costs.** Config-reachable silent no-op on a property write. `mold.animation.fling(node, "y", 800, {...})` coasts `y` from 0 to, say, 73; the target signal still holds 0. Any later `node.y = 0` — a reset button, a state machine re-asserting its declared value, a binding re-evaluating to the same number — returns `Ok(())` and changes nothing. The…

**Fix.** Pin the target when a decay settles, mirroring the timed branch: in the `physics_finished` loop (scene.rs:352) read `slot.current` and write it to `slot.target` before removing the motion, exactly as scene.rs:298-310 does. Better still, factor the two finish loops into one `settle(key)` helper so they cannot drift again. As a…

#### 5. `animate_from` writes `current` without calling `touch_layout`, so a zero-duration animation moves geometry that paint never re-lays-out

*high · certain · scene*

`crates/mold-scene/src/scene_behavior.rs:78`, `crates/mold-scene/src/scene_behavior.rs:84`, `crates/mold-scene/src/scene.rs:246`, `crates/mold-scene/src/scene.rs:338`, `crates/mold-cli/src/paint.rs:78`

**Costs.** This is the exact failure the comment at scene.rs:334-337 warns about ("Without it a paint reuses the layout it already had and the scene animates behind a still picture"), reintroduced on a different write path. With `duration = 0` it is permanent, not a one-frame glitch: the repaint fires (mold-lua bumps `scene_revision`) but paints…

**Fix.** Add `self.touch_layout(name);` at the end of `animate_from`, before `Ok(())` — the same conservative bump `assign` does at scene.rs:246. `name` is already the interned `&'static str` resolved at scene_behavior.rs:52-59.

#### 6. D-Bus property reads silently reject `o` and `g` scalars that every other decode path accepts

*high · certain · services*

`crates/mold-io/src/dbus_decode.rs:87`, `crates/mold-io/src/dbus_decode.rs:116`, `crates/mold-io/src/dbus_decode.rs:124`, `crates/mold-io/src/dbus_decode.rs:12`, `crates/mold-lua/src/api_system.rs:8`

**Costs.** Object-path-typed properties are everywhere in the D-Bus APIs a shell talks to (NetworkManager PrimaryConnection/ActiveConnections members, logind session paths, UPower device paths). A config author hits an opaque "not a supported scalar" error for a value the engine already knows how to convert, and the failure depends on whether the…

**Fix.** Delete the whole probe chain. `basic_value` reduces to `fn basic_value(value: &OwnedValue) -> Result<DbusValue, String> { dynamic_value(value) }` — `OwnedValue` derefs to `Value`, and `dynamic_value` already covers every scalar variant plus the compounds, with a better error for `Fd`. That removes ~30 lines and closes the hole in one…

#### 7. A corner radius larger than half the box renders as a different shape in sdf.wgsl than in field.wgsl — the same Rect changes shape depending on whether an Sdf ancestor absorbed it

*high · certain · shaders*

`crates/mold-render/src/field.wgsl:105`, `crates/mold-render/src/sdf.wgsl:94`, `crates/mold-render/src/glyph.wgsl:69`, `crates/mold-render/src/sdf.rs:84`, `crates/mold-render/src/effects.rs:197`

**Costs.** `radius = 9999` is the standard idiom for "pill"; it silently produces a square in the quad path. Worse, the same Rect node gives two different shapes depending on whether an `Element::Sdf` ancestor absorbed it into a field (paint_fields.rs `rect_layer` routes it to `sd_box`), so wrapping a row of cards in a field to fuse them changes…

**Fix.** Delete the two copies. Have sdf.wgsl and glyph.wgsl use one `rounded_distance` that matches field.wgsl's `sd_box` semantics — clamp per-corner radius to `min(half.x, half.y)` — and drop the now-redundant `max(radius, 0.0)`. Since WGSL has no include, either concatenate a shared prelude string at `create_shader_module` time or fold the…

#### 8. A scale change on a configured layer surface resizes its swapchain but never repaints it, unlike the configure and primary-scale paths

*medium · likely · cli-wayland*

`crates/mold-cli/src/surface_layers.rs:233`, `crates/mold-cli/src/surface_layers.rs:226`, `crates/mold-cli/src/surface_events.rs:31`, `crates/mold-cli/src/surface_events.rs:26`

**Costs.** The wgpu surface is reconfigured to a new physical size with no frame rendered into it, so a configured layer surface is left blank or stale after a scale change on an otherwise idle shell — and its cached layout still holds the old `scale_120`, so even the cache would have forced a relayout had a paint happened.

**Fix.** Mirror `layer_surface_configure`: after the resize, set `surface.needs_paint = true` and `paint_layer_surface(...)` (or request a frame callback), and let `layer_surface_scale` return `Result<(), String>` like its sibling.

#### 9. Logical-to-physical pixel conversion is written three times in three crates and the three copies disagree on every edge case

*medium · certain · cross-cutting*

`crates/mold-wayland/src/helpers.rs:1`, `crates/mold-image/src/image_cache.rs:238`, `crates/mold-cli/src/surfaces.rs:380`

**Costs.** PLAN_cleanup.md B6 lists two of these; there are three, and they differ from each other in four separate ways rather than being harmless copies. The version guarding the GPU surface size is the least defensive of the three, so the invariant 'a surface is never zero-sized' is held only by a clamp that lives two crates away in…

**Fix.** One function, in the lowest crate that all three can depend on (mold-layout or a small shared geometry module): take u64 internally, clamp scale to >= 1, div_ceil, clamp the result to >= 1, and return u32. Delete all three copies. If mold-image genuinely wants to reject a zero-size request, that check belongs at its own call site, not…

#### 10. `DistanceFieldStyle::weight` means two different things in two different units depending on which producer filled it, and the one `Default` is right for only one of them

*medium · certain · geometry*

`crates/mold-render/src/paint.rs:361`, `crates/mold-render/src/paint.rs:383`, `crates/mold-render/src/gpu/textures.rs:313`, `crates/mold-render/src/gpu/textures.rs:323`, `crates/mold-render/src/commands.rs:144` (+1 more)

**Costs.** A shared type whose single field carries two incompatible unit systems is a correctness assumption held purely by convention — the compiler cannot tell you that you paired `text_field_style` with `distance_field_uniform`, and the doc comment and `Default` both assert the wrong one of the two for text. The immediate cost is that the…

**Fix.** Split the field: `weight_units: FieldWeight` as an enum, or — simpler and truer to the code — two named fields, `edge: f32` (absolute, neutral 0.5, the image path) and `thickness_px: f32` (signed logical pixels, neutral 0.0, the text path), with `Default` giving `edge: 0.5, thickness_px: 0.0` so it is correct in both. Then…

#### 11. Layer-surface width and height are validated by three different rules across two doors onto the same struct fields

*medium · certain · lua*

`crates/mold-lua/src/layer_parse.rs:49`, `crates/mold-lua/src/layer_parse.rs:53`, `crates/mold-lua/src/layer_parse.rs:58`, `crates/mold-lua/src/window_methods.rs:39`, `crates/mold-lua/src/window_methods.rs:53` (+3 more)

**Costs.** The same idea — hand the anchored axis to the compositor — is expressible for width and refused for height, with no comment saying why, so a vertical dock anchored top+bottom cannot ask for it. And no `window.layer` surface can ever be resized back to width 0 once created, because the method door refuses what the table door accepts. The…

**Fix.** One validator. Decide the rule once — 0 means compositor-sized on both axes, otherwise 1..=16384 — and have `window_size_method` call `apply_layer_setting` for the `Layer` arm instead of writing the fields directly.

#### 12. Four doors into one reloadable-ID namespace apply three different name validations

*medium · certain · lua*

`crates/mold-lua/src/api_signal.rs:207`, `crates/mold-lua/src/api_signal.rs:223`, `crates/mold-lua/src/api_signal.rs:193`

**Costs.** One namespace, three rules. A `mold.reloadable` ID can be megabytes long or contain `..`, both of which the scoped sibling rejects; and because `persistent` synthesises `"{name}.{key}"`, `mold.reloadable("panel.width", 0)` and `mold.persistent("panel", { width = 0 })` collide, the loser getting an order-dependent "already registered"…

**Fix.** Run every ID through one validator before it reaches `register_reloadable_value` — put the `validate_scope_part` check inside `register_reloadable_value` itself so no door can skip it, and delete the three per-door checks.

#### 13. A ClipRect's content layer records untransformed bounds while every other Layer records transformed bounds — the same rectangle is transformed two lines later for the clip

*medium · likely · render-cpu*

`crates/mold-render/src/paint.rs:333`

**Costs.** For a rotated or scaled ClipRect with a border, the content layer is scissored to the pre-transform rectangle while its contents were rasterized post-transform, so part of the clipped content is cut away or a stale region is left behind. A rotated ClipRect is a case the code goes out of its way to support — `rotation != 0.0` is one of…

**Fix.** Set `bounds: transform.bounds(inner)` at paint.rs:266 and paint.rs:300, matching what paint.rs:274 already does with the same rectangle — or better, hoist `let inner_surface = transform.bounds(inner);` once and use it for both the layer bounds and the child clip.

#### 14. Field layer colours skip the inherited colour overlay that the field's own fill applies, and rect_layer's `unwrap_or(defaults.color)` fallback can never fire

*medium · certain · render-cpu*

`crates/mold-scene/src/schema.rs:188`

**Costs.** Tinting a subtree with `color_overlay` recolours an Sdf's default fill but leaves every layer that names its own colour untinted, so a themed or hover-tinted fused composition comes out half-tinted. And the two colour-resolution rules for the two absorbable element kinds differ in a way one of them documents (`layer_color`: "Transparent…

**Fix.** Route both `rect_layer` and `shape_layer` through one resolver that applies `apply_overlay(with_opacity(own, opacity), overlay)` to a layer's own colour just as `defaults.color` gets, and pass the overlay into `FieldDefaults`. Then either drop the dead `.unwrap_or` at paint_fields.rs:137 or make Rect's absorbed colour use the same…

#### 15. Three per-frame event sources in the same loop use three different drain policies; the D-Bus one is unbounded end to end

*medium · certain · services*

`crates/mold-lua/src/runtime_services.rs:200`, `crates/mold-lua/src/runtime_services.rs:206`, `crates/mold-services/src/status_notifier.rs:120`, `crates/mold-io/src/dbus_types.rs:188`

**Costs.** A chatty signal source — an MPRIS player emitting PropertiesChanged per position tick, or any remote peer that decides to spam — grows the channel without bound while the shell is busy, then discharges the entire backlog into Lua callbacks inside a single frame. The two neighbouring subscription types are already protected against…

**Fix.** Pick one policy for all three. Bound the subscribe-side channel (`mpsc::sync_channel(N)` with the producer thread dropping on full, matching Timer), and give the D-Bus drain the same `for _ in 0..N` cap the other two have.

#### 16. `close_layer` prunes stale touch points; `close_popup` and `close_floating` do not

*low · certain · cli-wayland*

`crates/mold-wayland/src/client_layer.rs:188`, `crates/mold-wayland/src/client_surface.rs:231`, `crates/mold-wayland/src/client_floating.rs:55`, `crates/mold-wayland/src/input_handlers.rs:169`

**Costs.** A finger down on a popup that Lua then hides (sync_window_surfaces -> `client.close_popup(id)`, surfaces.rs:260) leaves a `touch_points` entry keyed to a destroyed surface; subsequent motion/up for that touch id are delivered as events for `SurfaceRole::Popup(id)`, which mold-cli then dispatches to whatever node its own stale…

**Fix.** Add the same `touch_points.retain(...)` line to `close_popup` and `close_floating`, or better, factor the three-step cleanup into one `fn forget_surface(&mut self, role: SurfaceRole)` that all three call.

#### 17. `open_layer` seeds the layer record's width with a literal 1 while seeding height from the config

*low · likely · cli-wayland*

`crates/mold-wayland/src/client_layer.rs:120`, `crates/mold-wayland/src/client_layer.rs:121`, `crates/mold-wayland/src/surface_handlers.rs:110`

**Costs.** Dormant today because a compositor echoes a non-zero requested size, but it is the same expression written twice with one copy wrong — the failure mode is a surface laid out one pixel wide with no error anywhere.

**Fix.** `width: config.width.max(1),` — same as the height line directly beneath it.

#### 18. A ClipRect absorbed by an enclosing Sdf still emits its border quad and still allocates a content layer, though its fill is suppressed; border_width is read five times per node

*low · likely · render-cpu*

`crates/mold-render/src/paint.rs:104`, `crates/mold-render/src/paint.rs:114`, `crates/mold-render/src/paint.rs:240`, `crates/mold-render/src/paint.rs:302`, `crates/mold-render/src/paint_fields.rs:59`

**Costs.** The border case makes the field-absorption rule inconsistent with itself: paint_fields.rs:56-58 says a rect that became a layer "must not also be drawn as a rect, or the composition is painted twice — once fused and once with every seam back", yet the ClipRect variant is drawn twice anyway for its border, and pays for an extra offscreen…

**Fix.** Gate both paint.rs:240 and paint.rs:302 on `painted`, and hoist `border_width` into a single `let border_width = if matches!(element, Element::ClipRect) { scene.number(node, "border_width")?.max(0.0) } else { 0.0 };` beside the existing `radii`/`clips` hoist.

#### 19. `set_behavior(Some)` and `set_physics(Some)` tear down in-flight motion without the Canceled event and without clearing `paused_physics`, unlike their `None` counterparts

*low · certain · scene*

`crates/mold-scene/src/scene_behavior.rs:28`, `crates/mold-scene/src/scene_behavior.rs:121`, `crates/mold-scene/src/playback.rs:43`, `crates/mold-scene/src/playback.rs:50`

**Costs.** Two bugs from one asymmetry. A Lua handler registered on animation end never fires when the motion is torn down by installing the other kind of motion, so a waiter hangs. And `is_animation_paused` can report `true` indefinitely for a property that is running, which is exactly the kind of state a UI uses to decide whether to show a resume…

**Fix.** Give all four arms the same teardown: a single `fn cancel_motion(&mut self, key: PropertyKey)` that removes from `animations`, `physics` and `paused_physics` and pushes `AnimationEnd::Canceled` if anything was actually removed, called from both branches of `set_behavior` and `set_physics`.

## Bad implementation — built in a way that bites

Unbounded caches, per-frame waste, quadratic loops on hot paths, and error handling that loses information.

_26 findings — 11 high, 15 medium, 0 low._

#### 20. Popup and floating surfaces repaint forever: each paint asks for a frame callback and the callback handler repaints unconditionally

*high · certain · cli-wayland*

`crates/mold-render/src/damage.rs:128`

**Costs.** One visible menu popup keeps its worker thread, the compositor's callback machinery and a full draw-list rebuild + damage diff running at display refresh rate for as long as the popup exists, on a shell whose whole architecture is otherwise built to go quiet when nothing moves (FramePacer, needs_paint, `pacer.rest()`). It also defeats…

**Fix.** Give popups and floating surfaces the same gate the other two kinds have. Either reuse `AuxiliarySurface::needs_paint` (set it in the primary `Frame` arm alongside the layer-surface loop at surface_events.rs:67-74, filter and clear it in the PopupFrame/FloatingFrame arms exactly as `layer_surface_frame` does), or make…

#### 21. TransformTracker's per-node geometry cache grows forever: its only eviction method, retain_scene, has zero callers

*high · certain · cross-cutting*

`crates/mold-lua/src/runtime_helpers.rs:53 (remove_scene_subtree starts at 53, not 75; the pruning block spans 66-116)`

**Costs.** A shell process runs for the machine's uptime. Every scene node a config ever creates and destroys — list rows re-rendered, views swapped, popups opened and closed, anything a reactive `for` rebuilds — leaves a permanent NodeHandle+Geometry entry in this map. NodeHandle is slotmap-versioned, so a reused slot never overwrites the stale…

**Fix.** Call `TransformTracker::retain_scene` from `remove_scene_subtree` in crates/mold-lua/src/runtime_helpers.rs alongside the other twelve retains — or better, since `removed` is already computed there as a HashSet, add a `remove_nodes(&mut self, removed: &HashSet<NodeHandle>)` to TransformTracker and drop the O(cache) scene-walk of…

#### 22. DamageTracker deep-clones the entire DrawList every frame, defeating the capacity recycling the surrounding code explicitly does

*high · certain · cross-cutting*

`crates/mold-render/src/damage.rs:25`, `crates/mold-render/src/damage.rs:42`, `crates/mold-render/src/damage.rs:75`, `crates/mold-render/src/damage.rs:45`, `crates/mold-render/src/damage.rs:52` (+2 more)

**Costs.** This is the per-frame hot path of the renderer, and it allocates proportionally to the whole scene on every frame even when nothing changed — which is the case the caching in `RenderEngine::render` and `CachedLayout` was built for. A bar with a clock repaints at 1Hz minimum and 60Hz whenever anything animates; every one of those frames…

**Fix.** `self.previous` is dead the instant `diff` returns, and `RenderEngine::render` already owns `list` by value. Swap instead of cloning: have `diff` take the list and return the old one (`std::mem::replace(&mut self.previous, next)`), and let `RenderEngine::render` put the recovered previous list back into `self.list` for its capacity. That…

#### 23. WgpuBackend::render creates a full-screen GPU texture per layer per frame, plus up to eight more textures and eight uniform buffers when blur or layer shadow is on

*high · certain · cross-cutting*

`crates/mold-render/src/gpu/backend_render.rs:77`, `crates/mold-render/src/gpu/targets.rs:1`, `crates/mold-render/src/gpu/targets.rs:26`, `crates/mold-render/src/gpu/targets.rs:41`, `crates/mold-render/src/gpu/targets.rs:81` (+1 more)

**Costs.** Any config that sets opacity below 1, a rotation, a rounded clip, a blur, or a layer shadow creates a layer (see crates/mold-render/src/paint.rs:41-47, `creates_layer`), and each such layer costs one full-screen RGBA texture allocation per frame — nine textures and eight uniform buffers if both blur and layer shadow are on. At 60Hz that…

**Fix.** Cache the per-layer targets and blur chains on WgpuBackend keyed by (index, size), the way `instance_buffer`/`instance_capacity` already work: keep a Vec of targets grown to `list.layers.len()` and invalidated only in `resize`. The blur chains additionally depend only on `offset`, which can go into the uniform buffer as a per-frame write…

#### 24. With `surface.mask` set, the whole input region is rasterised and re-sent to the compositor on every single paint; the MouseArea path next to it dedupes

*high · certain · geometry*

`crates/mold-cli/src/paint.rs:93`, `crates/mold-cli/src/paint.rs:116`, `crates/mold-region/src/lib.rs:112`, `crates/mold-region/src/lib.rs:238`, `crates/mold-wayland/src/client_layer.rs:285`

**Costs.** Per-frame O(surface area) work plus per-frame heap allocation plus redundant compositor IPC, in the paint path, to recompute a value that cannot have changed. The identical concern is already handled correctly ten lines below, which makes this an asymmetry someone will assume is deliberate.

**Fix.** Cache the built `Vec<InputRect>` on the surface alongside the existing `cached.input` and gate `set_layer_composed_input_region` on the same `is_none_or(|cached| cached.input != input)` test the MouseArea branch uses — ideally by having the mask branch also produce an `input: Vec<InputRect>` so both branches share one dedupe, instead of…

#### 25. ImageCache is keyed on animatable pixel sizes, never evicts, and `clear()` has no callers — an animated icon size leaks a decode, a distance field, a theme-index parse and a GPU texture per pixel step

*high · certain · geometry*

`crates/mold-image/src/image_cache.rs:230`, `crates/mold-image/src/image_cache.rs:48`, `crates/mold-image/src/image_cache.rs:154`, `crates/mold-render/src/gpu/textures.rs:113`, `crates/mold-render/src/gpu/textures.rs:221` (+1 more)

**Costs.** A single animated icon or image size — a hover grow, a launcher zoom — does a synchronous SVG rasterisation and a full distance transform per frame during the animation, and permanently retains every intermediate: two host RGBA buffers and one GPU texture per pixel step, plus a filesystem theme-index re-parse per step. Nothing ever gives…

**Fix.** Two independent fixes. (a) Give the caches a bound — an LRU keyed the way the glyph atlas already is (`last_used` clock plus eviction, gpu/glyphs.rs:292-345 is the working pattern in-tree), covering `images`, `distance_fields` and `image_textures` together. (b) Stop keying on the animated size: an SVG or a distance field should be…

#### 26. Every scene node allocates its own metatable and two closures, against the crate's own shared-metatable convention

*high · certain · lua*

`crates/mold-lua/src/scene_bindings.rs:120`, `crates/mold-lua/src/scene_bindings.rs:126`, `crates/mold-lua/src/scene_bindings.rs:143`, `crates/mold-lua/src/scene_bindings.rs:186`, `crates/mold-lua/src/scene_bindings.rs:193` (+6 more)

**Costs.** Three GC allocations and two `Rc` clones for every node the config creates, where one shared metatable would do — and repeated on every `loader.item` read, which a binding can do per frame. It also means two userdata for the same node are never `==` in Lua, and each carries its own function identities.

**Fix.** Build the `__index`/`__newindex` pair and the metatable once where the UI constructors are installed, `ctx.stash` it, and have `node_userdata` take the stashed handle and only do `UserData::new_static` + `set_metatable(ctx, Some(ctx.fetch(&node_metatable)))`. This matches the six other userdata types in the crate. Additionally cache the…

#### 27. DamageTracker keys commands by NodeHandle, but a ClipRect emits two commands with the same node — the fill quad is invisible to the differ

*high · certain · render-cpu*

`crates/mold-render/src/damage.rs:46`, `crates/mold-render/src/damage.rs:53`, `crates/mold-render/src/paint.rs:114`, `crates/mold-render/src/paint.rs:302`, `crates/mold-render/src/tests/tree.rs:56` (+1 more)

**Costs.** Animating or assigning the background colour of any bordered ClipRect produces an empty damage list; RenderEngine::render then skips the backend entirely (damage.rs:138 `if !damage.is_empty()`), so the pixels never update. The bug is silent and only shows on the one element type that emits two commands, which is exactly the container…

**Fix.** Key the diff by command position, not by node: zip `previous.commands` and `next.commands` by index and fall back to whole-list damage when the lengths differ, or key by `(node, command_slot)` where the slot distinguishes a ClipRect's background from its border. Alternatively give DrawCommand a stable per-command id assigned during…

#### 28. udev: a filtered-out event is indistinguishable from "no event", so the per-frame drain loop aborts early

*high · certain · services*

`crates/mold-services/src/udev.rs:113`, `crates/mold-services/src/udev.rs:118`, `crates/mold-services/src/udev.rs:86`, `crates/mold-lua/src/runtime_services.rs:211`

**Costs.** A subsystem-filtered udev subscription — the normal way to use this API — delivers at most one matching event per non-matching packet per frame. During exactly the bursts that matter (device hotplug floods hundreds of uevents), matching events are delayed by a frame each or dropped entirely once the 64KB socket buffer overflows. The bug…

**Fix.** Make the filter loop inside `next_event` rather than return: after a filter miss, `continue` back to the poll instead of `return Ok(None)`, so `Ok(None)` means only "socket empty". Alternatively return a three-state (`Event`/`Filtered`/`Empty`) and have the caller keep draining on `Filtered`.

#### 29. Every offscreen layer allocates a full-surface texture (plus up to 8 more for blur/shadow) every single frame

*high · certain · shaders*

`crates/mold-render/src/gpu/backend_render.rs:76`, `crates/mold-render/src/gpu/backend_render.rs:77`, `crates/mold-render/src/gpu/backend_render.rs:78`, `crates/mold-render/src/gpu/backend_render.rs:89`, `crates/mold-render/src/gpu/targets.rs:6` (+2 more)

**Costs.** At 1920x1080 one full-size Rgba8 target is ~8.3 MB. A single blurred, faded layer churns roughly 8.3 + (2.1 + 0.5 + 2.1 + 8.3) MB of GPU memory allocated and freed per frame; two or three such nodes during a fade-in is tens of MB of allocator traffic at 60 Hz. wgpu does not pool textures, so this hits the driver allocator directly and is…

**Fix.** Cache layer targets and blur chains on `WgpuBackend`, keyed by layer index (or a small free-list of full-size targets), invalidated only in `resize`. The blur uniform buffers can be one buffer with dynamic offsets written per frame rather than four `create_buffer_init` calls. Size the target to `layer.bounds` rather than the whole…

#### 30. field_area ignores per-layer rotation, so a rotated non-square SDF layer is clipped flat by its own quad

*high · certain · shaders*

`crates/mold-render/src/field.rs:168`, `crates/mold-render/src/field.rs:185`, `crates/mold-render/src/field.wgsl:275`, `crates/mold-render/src/field.wgsl:57`, `crates/mold-render/src/paint_fields.rs:105`

**Costs.** Any rotated non-square SDF layer is silently truncated, and animating `rotation` makes the shape grow and shrink as the clip bites. It is the same class of bug the function was written to fix, just not extended to the one transform the shader applies per-layer.

**Fix.** Rotate the layer's four corners about `layer.bounds` centre by `layer.rotation` before folding into left/top/right/bottom (or, cheaper and safe, expand each layer's half-extents to the circumscribed radius `hypot(w/2, h/2)` whenever `rotation != 0`). Note the same loop also walks all `layers` while `from_command` only uploads the first…

#### 31. Configured layer surfaces are painted twice per animation frame — the main repaint block ignores `needs_paint` and never clears it

*medium · certain · cli-wayland*

`crates/mold-cli/src/surface_run.rs:268`, `crates/mold-cli/src/surface_events.rs:67`, `crates/mold-cli/src/surface_layers.rs:257`, `crates/mold-cli/src/paint.rs:121`

**Costs.** Doubles the per-frame CPU cost of every configured layer surface for no visible effect, and makes the `needs_paint` accounting a lie — the field is set by one path and consumed by another that the first path has already pre-empted.

**Fix.** Pick one driver. Either drop the `layer_surfaces` loop from the main repaint block and let the frame callback own configured layer surfaces (which is what `needs_paint` was written for), or have `paint_layer_surface` clear `needs_paint` so the callback path becomes a no-op after a main-block paint.

#### 32. `primary_surface_root` re-validates the whole config and allocates three collections on every repaint and every key press

*medium · certain · cli-wayland*

`crates/mold-cli/src/surfaces.rs:421`, `crates/mold-cli/src/paint.rs:7`, `crates/mold-cli/src/surface_events.rs:329`, `crates/mold-lua/src/runtime_config.rs:102`

**Costs.** Four heap allocations and a full-scene scan per frame, per surface worker, forever, in the function whose only job is to answer a question whose answer almost never changes — the same class of per-frame allocation already called out for `tick_animations` (E1).

**Fix.** Resolve the primary root once in `run_surface` (it is already called there at line 29), cache it in `SurfaceEventState`, and recompute it only where `take_window_surface_change()` / a reload already forces a resync.

#### 33. Every auxiliary surface's layout cache is dropped on every window-surface sync, and an anchored popup makes that every frame

*medium · likely · cli-wayland*

`crates/mold-lua/src/runtime_services.rs:52`

**Costs.** `CachedLayout` exists because "Layout is the most expensive thing a frame does" (paint.rs:26-28); this hands that saving back for every auxiliary surface as soon as one anchored popup follows a moving node. It is a cache that invalidates itself for reasons unrelated to the surface being invalidated.

**Fix.** Clear `current.layout` only when something the layout actually depends on changed — `current.root != surface.root` for all three, plus a size change for floating. The `revision`/`size`/`scale_120` check in `CachedLayout::still_valid` already covers the rest.

#### 34. merge_damage is O(n^2) comparisons with an O(n) Vec::remove inside and a full inner-loop restart on every merge, run on every frame

*medium · certain · cross-cutting*

`crates/mold-render/src/damage.rs:27`, `crates/mold-render/src/damage.rs:44`, `crates/mold-render/src/damage.rs:77`

**Costs.** Any frame that changes the layer set or the output scale hands this function two full command lists' worth of rectangles. A scene of a few hundred nodes therefore does tens of thousands of `touches` calls with memmoves between them, on the frame that is already the most expensive one. `touches` uses `<=` on the exclusive edges, so merely…

**Fix.** At minimum, do not restart: after a merge, continue from the current `other` (the element that shifted into it) rather than resetting to `index + 1`, and swap_remove into a scratch vector instead of `Vec::remove`. Better, sort by y then x and do a single-pass sweep merge, which is O(n log n) and is what damage accumulation normally uses.

#### 35. Every visible glyph's full field bitmap is heap-cloned on every frame, for data the atlas only ever reads on a miss

*medium · certain · geometry*

`crates/mold-text/src/raster_glyph.rs:64`, `crates/mold-text/src/raster_glyph.rs:76`, `crates/mold-text/src/lib.rs:215`, `crates/mold-render/src/gpu/glyphs.rs:262`, `crates/mold-text/src/lib.rs:81`

**Costs.** The whole point of the distance-field glyph path, stated at glyph_fields.rs:2-10, is that the letter is stored once and never re-rendered. It is then copied out of that store, in full, once per glyph per frame — reintroducing per-frame bandwidth proportional to the glyph *bitmaps* on screen when the design had reduced it to a handful of…

**Fix.** Make `RasterGlyph::data` an `Rc<Vec<u8>>` (or a `Cow`/`Option`) rather than an owned `Vec<u8>` — `self.fields` already holds `Rc<FieldImage>`, so the hit path becomes a refcount bump and `glyph_pixels` (glyphs.rs:351) reads through the Rc unchanged. Better still, have `rasterize` return the key and geometry and let the atlas ask for the…

#### 36. `ui.spring` / `ui.smoothed` mutate the caller's table, so a shared options table silently takes the last kind

*medium · certain · lua*

`crates/mold-lua/src/configure.rs:302`, `crates/mold-lua/src/configure.rs:308`, `crates/mold-lua/src/configure.rs:322`

**Costs.** A function that reads like a constructor but is an in-place mutation. The failure is silent and produces the wrong motion, and the aliasing is invisible at the call site.

**Fix.** Copy: build a fresh `Table`, shallow-copy the caller's entries into it, set `kind`, and return the copy. Reject a table that already carries a conflicting `kind` rather than overwriting it.

#### 37. DamageTracker deep-clones the entire DrawList every frame, defeating the buffer reuse the crate is built around

*medium · certain · render-cpu*

`crates/mold-render/src/damage.rs:25`, `crates/mold-render/src/damage.rs:42`, `crates/mold-render/src/damage.rs:75`, `crates/mold-render/src/damage.rs:128`, `crates/mold-render/src/commands.rs:344`

**Costs.** The optimization RenderEngine::render is built around is cancelled by the tracker sitting behind it: the allocator traffic saved on the draw list is spent again, in full, cloning it into `previous`. On the thousands-of-commands surface the comment describes this is hundreds of kilobytes of allocate-copy-free per frame.

**Fix.** Swap instead of clone: keep two `DrawList` buffers in the engine and `std::mem::swap` the freshly built list into `previous` at the end of the frame, rebuilding next frame into the old `previous` (whose String and Vec capacities are still warm). `RenderEngine` already owns the list, so it can hand the tracker the retired buffer rather…

#### 38. merge_damage is O(n^2) with a Vec::remove per merge, on the per-frame damage path

*medium · certain · render-cpu*

`crates/mold-render/src/effects.rs:365`, `crates/mold-render/src/damage.rs:27`, `crates/mold-render/src/damage.rs:77`

**Costs.** Every scale change, every frame where the layer set changes (a node gaining or losing opacity/blur/rotation), and every large scene edit runs a quadratic pass with an O(n) memmove per merge, in the frame's critical path — before anything is presented, since `declare(&damage)` at damage.rs:137 must happen before the commit.

**Fix.** Cap the work: sort the rects by y then x and sweep, or fall back to a single bounding rect (or a fixed small number of buckets, e.g. a 4x4 grid of the surface) once the input exceeds a threshold. A compositor gains nothing from more damage rects than it can scissor anyway.

#### 39. `Repeat::PingPongTimes` on an animation group is accepted and then silently downgraded to a single pass, while `Repeat::PingPong` is rejected with an error

*medium · certain · scene*

`crates/mold-scene/src/groups.rs:155`, `crates/mold-scene/src/groups.rs:270`, `crates/mold-lua/src/configure.rs:396`, `crates/mold-lua/src/api_group.rs:59`

**Costs.** `mold.animation.play{ ping_pong = true, loops = 3, ... }` runs once and stops, with no error and no event distinguishing it from a correct run — the one form of the option that fails does so silently, while the adjacent form fails loudly. The mismatch with `passes()` also means `group.total` is `N *` too long, so even the single pass it…

**Fix.** Either implement it — add `Repeat::PingPongTimes(count) => group.passes < count.max(1)` to groups.rs:270 alongside per-pass direction reversal — or reject it at groups.rs:155 by widening the guard to `matches!(repeat, Repeat::PingPong | Repeat::PingPongTimes(_))`, so a config author gets the same clear error either way.

#### 40. StreamCollector copies its entire accumulated buffer on every pushed chunk (O(n^2) on the process-output path)

*medium · certain · services*

`crates/mold-io/src/streams.rs:137`, `crates/mold-io/src/streams.rs:104`

**Costs.** Streaming a large command's output (a `journalctl` dump, a big `curl`) through a collector burns quadratic time and doubles peak memory, on the shell's own thread, for a value the collector already holds in `pending`.

**Fix.** Drop the `data` field. In non-`wait_for_end` mode `data()` and `text()` should read `pending` directly; in `wait_for_end` mode gate them on `self.finished`. That makes `push` O(chunk) and removes the duplicate 16 MiB buffer.

#### 41. IpcServer's accept thread exits permanently and silently on any transient accept error

*medium · certain · services*

`crates/mold-io/src/ipc.rs:129`, `crates/mold-io/src/files.rs:305`

**Costs.** The shell's entire external control surface (`mold call`, `verbs`, `bindings`, `kill` — ipc.rs:18-24) goes dead from one recoverable errno, with no log line and no way to notice short of the CLI timing out. Recovering requires restarting the shell.

**Fix.** Retry the recoverable errnos (`ConnectionAborted`, `Interrupted`, and the fd-exhaustion kinds) with the same 10 ms backoff, and on a genuinely fatal error record it somewhere the shell can surface — the same `state.logs` channel that already carries `udev:` and `status notifier:` errors (crates/mold-lua/src/runtime_services.rs:218, :233).

#### 42. udev poll zeroes a 64 KiB stack buffer on every call, up to 32 times per frame

*medium · certain · services*

`crates/mold-services/src/udev.rs:89`, `crates/mold-services/src/udev.rs:7`, `crates/mold-lua/src/runtime_services.rs:206`

**Costs.** Per-frame work proportional to a worst-case constant rather than to the data actually received, on the shell's main thread, in the path that runs during exactly the hotplug bursts where frames are already under pressure. It also puts a 64 KiB frame on the stack of the render loop.

**Fix.** Hold the buffer in `UdevMonitor` as a `Box<[MaybeUninit<u8>; MAX_EVENT_BYTES]>` (or a `Vec<u8>` allocated once in `new`) and reuse it across calls; `next_event` would need `&mut self`, which the caller can supply.

#### 43. File error classification round-trips through error message strings

*medium · certain · services*

`crates/mold-io/src/files.rs:99`, `crates/mold-io/src/files.rs:103`, `crates/mold-io/src/files.rs:38`, `crates/mold-io/src/files.rs:31`

**Costs.** Editing either message — a typo fix, a reworded diagnostic — silently reclassifies the error to `FileViewError::Unknown`, which is what the Lua config sees as `error()` (files.rs:84-92 maps it to the wire string `"unknown"`). A correctness invariant held only by two matching string literals, with no test or type forcing them to agree.

**Fix.** Have `read_bounded` return `Result<Vec<u8>, FileViewError>` directly, or introduce a private enum error that `FileView` returns and `FileDocument` maps — either way the classification stops depending on message text.

#### 44. PipeWire round-trips block the shell thread in an untimed wait loop reachable from Lua

*medium · likely · services*

`crates/mold-services/src/pipewire/runtime.rs:205`, `crates/mold-services/src/pipewire/runtime.rs:212`, `crates/mold-services/src/pipewire/runtime.rs:106`, `crates/mold-services/src/pipewire/runtime.rs:143`

**Costs.** If the PipeWire daemon dies or stalls between the `core_sync` and the `done` event without delivering a `core_error`, `thread_loop_wait` never returns and the caller's thread is wedged with the loop lock held. Reached from a config calling the volume API, that is a hard hang of the whole shell with no recovery path.

**Fix.** Bound the loop: use the timed variant of the thread-loop wait (pw_thread_loop_timed_wait) or track a deadline across iterations and return a `PipeWireError` on expiry, matching the timeout discipline the rest of mold-services already follows.

#### 45. image_textures is an unbounded GPU-texture cache keyed on pixel size, and the one clear() that exists has no callers

*medium · certain · shaders*

`crates/mold-render/src/gpu/backend_types.rs:56`, `crates/mold-render/src/gpu/textures.rs:221`, `crates/mold-render/src/gpu/textures.rs:115`, `crates/mold-image/src/image_cache.rs:230`

**Costs.** A shell runs for weeks. A config that animates an icon's size, or a set of album-art thumbnails at varying sizes, leaks GPU memory monotonically with no ceiling and no way for the host to reclaim it — `ImageCache::clear` is the intended escape hatch and nothing wires it up.

**Fix.** Give `image_textures` (and `ImageCache`) the same last-used clock + eviction the glyph atlas has: stamp each entry on use in `create_texture_batch`, and drop entries not touched for N frames or once a byte budget is exceeded. Then either call `ImageCache::clear` from the eviction path or delete it as dead API.

#### 80. The sandbox has two limit systems: five named and tunable, twenty-one hard-coded inline

*medium · certain · cross-cutting*

`crates/mold-lua/src/types.rs:3`, `crates/mold-lua/src/api_host.rs:14`, `crates/mold-lua/src/api_transform.rs:69`, `crates/mold-lua/src/api_system.rs:192`, `crates/mold-lua/src/reactive_bindings.rs:159` (+16 more)

**Costs.** `Limits` (`types.rs:3`) names five bounds — `fuel`, `memory`, `slice_fuel`, `effect_fuel`, `frame_fuel` — and a host embedding mold can tune all five. Twenty-one further resource caps are written inline as magic numbers, seventeen of them in `api_host.rs` alone, ranging from `>= 4` (screencopy callbacks, status notifiers) to `>= 1_024` (transform watchers) with no stated reason for any value. A host cannot tune them, and there is no single place to answer "what does this sandbox actually bound?" — which is the question that matters, because these are the caps standing between a config and resource exhaustion. Three of them are the *same* guard with three different answers: maximum table-nesting depth is 16 in `reactive_bindings.rs:159`, 32 in `table_menu.rs:33`, and 64 in `window_parse.rs:236`.

**Fix.** Move the twenty-one into `Limits` as named fields with the current values as defaults, so the sandbox's bounds are declared in one struct that can be read, tuned and tested. Collapse the three nesting depths to one unless there is a reason for three, in which case state it.

## Parallel implementations — two ways to say one thing

The standing rule on this project is one implementation, no legacy path. Each of these is two.

_12 findings — 1 high, 10 medium, 1 low._

#### 46. Two Lua easing APIs cover the same ground, and the curve-object one rejects a mixed integer/float pair

*high · certain · lua*

`crates/mold-lua/src/api_animation.rs:139`, `crates/mold-lua/src/api_animation.rs:148`, `crates/mold-lua/src/api_animation.rs:159`, `crates/mold-lua/src/api_animation.rs:176`, `crates/mold-lua/src/api_animation.rs:197` (+8 more)

**Costs.** A config author has to learn two spellings of the same operation and discover by trial which one accepts their arguments. The mixed integer/float rejection is a live papercut reachable from ordinary config (`curve:interpolate(t, 0, 1.5)`), and the Nil-axis disagreement means the same point table behaves differently depending on which…

**Fix.** Keep one. The curve userdata is the cheaper form (parse once, evaluate many), so keep `mold.easing_curve` and give it `value_at`, `number`, `point`, `rect`, `color` methods that reuse `read_axes` and `Easing::interpolate_point/rect/color`; delete the `mold.easing` table. Whichever survives, fix the number arm to accept any mix of…

#### 47. Reserve layers are destroyed and recreated for a thickness change, though the only field that changed is one `set_layer_geometry` updates in place

*medium · certain · cli-wayland*

`crates/mold-cli/src/surface_layers.rs:49`, `crates/mold-cli/src/surface_layers.rs:60`, `crates/mold-cli/src/surface_run.rs:202`, `crates/mold-cli/src/surface_layers.rs:96`, `crates/mold-wayland/src/client_layer.rs:142`

**Costs.** Two mechanisms for the same protocol operation, and the reserver picked the destructive one: every thickness change unmaps and remaps four layer surfaces, so the compositor recomputes the output's usable area twice and tiled windows visibly jump — the exact failure `layer_update` was written to avoid.

**Fix.** Route reservers through the same decision the window surfaces use: keep the last `reserve_bar_config` per edge, `set_layer_geometry` when only the exclusive zone moved, `open_layer` only when the surface is not open, `close_layer` when the thickness reaches zero.

#### 48. Keyboard-target lookup exists in three duplicated pairs (whole-scene and per-root); the whole-scene half is now vestigial and next_key_target has no production caller

*medium · certain · cross-cutting*

`crates/mold-lua/src/runtime_helpers.rs:7`, `crates/mold-lua/src/runtime_helpers.rs:36`, `crates/mold-lua/src/runtime_events.rs:242`, `crates/mold-lua/src/runtime_events.rs:253`, `crates/mold-lua/src/runtime_events.rs:286` (+1 more)

**Costs.** Six functions and two tree walks where three functions and one tree walk do the job. The duplication is already load-bearing: the lock screen calls the vestigial half (`first_key_target`) and consequently misses the Tab traversal that only exists on the `_in` half. Keeping both guarantees the next focus-behaviour change lands on one of…

**Fix.** Delete `key_targets`, `first_key_target`, and `next_key_target`. Point lock.rs at `first_key_target_in`/`next_key_target_in` with the lock root (which it already has via `primary_surface_root`), and update tests/scene.rs:315 and :325 to the `_in` forms.

#### 49. ScriptValue is a byte-for-byte duplicate of mold-io's IpcValue, with duplicated to_lua/from_lua and two shim functions to convert between them

*medium · certain · cross-cutting*

`crates/mold-lua/src/runtime_helpers.rs:180`, `crates/mold-io/src/ipc.rs:8`, `crates/mold-lua/src/runtime_helpers.rs:189`, `crates/mold-lua/src/surface_types.rs:271`, `crates/mold-lua/src/runtime_helpers.rs:203` (+3 more)

**Costs.** Two identical value vocabularies mean a third variant (a bytes value, an f32, a nil-vs-absent distinction) has to be added in two enums, four conversion functions, and two shims, and any one of those six can be missed. The shims are pure overhead — a String clone per value on the config-reload path — bought by the duplication and nothing…

**Fix.** Delete `ScriptValue` and its `from_lua`/`to_lua`/`to_scene`, use `mold_io::IpcValue` throughout the reactive graph (state.rs:323/327/328/341), and delete `script_ipc_value` and `ipc_script_value` along with their two call sites at runtime_config.rs:194 and :203. Keep whichever of the two `from_lua` error messages is better and use it…

#### 50. Two distance-field generators over the same library, and the image one has no padding, so an image field has no room outside the shape for an outline

*medium · certain · geometry*

`crates/mold-image/src/distance_field.rs:5`, `crates/mold-text/src/glyph_fields.rs:38`, `crates/mold-image/src/distance_field.rs:16`, `crates/mold-text/src/glyph_fields.rs:39`, `crates/mold-render/src/gpu/textures.rs:161`

**Costs.** One idea, two implementations, and the copies have already drifted on the one parameter that decides whether the feature works. `distance_field = true` on an Icon that fills its bounds gives a clipped or absent outline, and the config author has no way to see why; the glyph path with the same style properties works. It is also the exact…

**Fix.** One `alpha_distance_field(alpha, width, height, spread) -> (Vec<u8>, u32, u32)` in a shared place, padding by `ceil(spread)` on every side and returning the padded extents; mold-text wraps it into a `FieldImage` and mold-image expands the single channel into RGBA (or, better, stops doing so — the texture is sampled `.r` only, per…

#### 51. `Layout::chain_transform` and `TransformTracker::node_to_surface_transform` are the same root-to-node transform walk written twice, diverging on the missing-geometry case

*medium · certain · geometry*

`crates/mold-layout/src/hit.rs:62`, `crates/mold-layout/src/transform.rs:50`, `crates/mold-layout/src/transform.rs:159`

**Costs.** The same walk in two places is where the next transform feature (a node that opts out of its parent's transform, a cached chain, a clip-aware fold) gets added to one copy and not the other — the exact drift PLAN A1 documents for the shaders. The Err-vs-Ok(None) split already means the same broken scene produces a hard error through one…

**Fix.** Delete `chain_transform` and give `node_to_surface_transform` a generic geometry source — `fn chain_transform(scene: &Scene, node: NodeHandle, geometry: impl Fn(NodeHandle) -> Option<Geometry>) -> Result<Option<Transform2D>, LayoutError>` as a free function next to `ancestor_chain`, called by both. Pick one missing-geometry answer;…

#### 52. `mold.color_quantize` and `mold.color_quantizer` are the same call under two names, both exported to `mold.core`

*medium · certain · lua*

`crates/mold-lua/src/api_time.rs:226`, `crates/mold-lua/src/api_time.rs:235`, `crates/mold-lua/src/api_image.rs:92`, `crates/mold-lua/src/api_image.rs:114`, `crates/mold-lua/src/api_image.rs:79` (+2 more)

**Costs.** Two API names one word apart that do the same thing, offered on the same table. This is the exact shape of parallel implementation the project has said it does not keep, and it doubles the surface that has to stay correct when quantizer options change.

**Fix.** Delete `color_quantize` (api_time.rs:226-235; it is also mis-filed — it lives in the time API next to the clock). Keep `mold.color_quantizer`, whose userdata is a strict superset, and update the one caller pattern to `mold.color_quantizer(opts):colors()`.

#### 53. `mold.window` offers three ways to say "show this surface", all writing the same bool

*medium · certain · lua*

`crates/mold-lua/src/api_module.rs:97`, `crates/mold-lua/src/api_module.rs:113`, `crates/mold-lua/src/api_module.rs:129`

**Costs.** Three spellings of one state change is three things to document, three to keep in step, and an open question for a config author about whether `open()` does something `set_visible(true)` does not. It does not.

**Fix.** Keep `visible()` as the getter and one setter. Delete `open` and `close` (or delete `set_visible` and keep the verb pair) — not both spellings.

#### 54. `mold.flickable` is a second momentum mechanism, clocked by Lua, with a different decay law than `mold.animation.fling`

*medium · likely · lua*

`crates/mold-scene/src/model.rs:379`, `crates/mold-scene/src/motion.rs:249`

**Costs.** Two decay laws for one gesture, one of which requires the configuration to drive the clock — the thing the fling API was written to avoid — and which the engine cannot pause, seek, or hand to a spring. Its docstring already describes consumers that do not exist, which is how a facility quietly becomes dead.

**Fix.** Delete `mold.flickable` and `FlickState`, and drive `Flickable.content_x/content_y` with `mold.animation.fling` plus its `min`/`max` bounds, which already provides clamping and bounce. If bounded coasting on a non-scene value is genuinely wanted, express it as a fling on a property rather than a second integrator.

#### 55. Two config-facing stream framers: `mold.line_parser()` is `mold.split_parser("\n")` plus a CR strip

*medium · certain · services*

`crates/mold-io/src/streams.rs:3`, `crates/mold-io/src/streams.rs:31`, `crates/mold-lua/src/api_socket.rs:255`, `crates/mold-lua/src/api_socket.rs:318`

**Costs.** Two ways for a config author to say "split this stream on newlines", with an undocumented and easy-to-miss semantic difference (CRLF handling) as the only thing separating them. This is the pattern the owner has said to delete rather than keep.

**Fix.** Delete `LineParser` and its bindings. Give `SplitParser` a `trim_cr: bool` (or make `\r\n` handling unconditional for the `\n` delimiter) and expose `mold.split_parser("\n")` as the single answer.

#### 56. DistanceFieldStyle.weight means two incompatible things, and Image/Icon vs Text spell the same four properties two different ways

*medium · certain · shaders*

`crates/mold-render/examples/gpu_smoke.rs:98`, `crates/mold-render/src/gpu/text_field_tests.rs:59`, `crates/mold-render/src/gpu/text_field_tests.rs:91`

**Costs.** A `DistanceFieldStyle` value is not portable between the two paths, and nothing in the type says so: any future code that builds one for text from a default, or reuses `distance_field_style()` for a Text node, silently renders at the wrong weight with a size-dependent error. The duplicated property vocabulary is the same 'two ways to say…

**Fix.** Pick one unit for `weight` — logical-pixel edge offset, neutral 0.0, is the one that actually holds meaning across sizes — convert the image path to it, fix `Default` to 0.0, and delete the `distance_field_*` property names in favour of the shorter `thickness / softness / outline_width / outline_color` on Image and Icon too. Then…

#### 57. mold_region::Rect and mold_wayland::InputRect are the same struct under two names, bridged by a per-update allocating field-by-field copy

*low · certain · cross-cutting*

`crates/mold-wayland/src/client_layer.rs:277 (set_layer_composed_input_region, the enclosing function)`

**Costs.** Six spellings of a rectangle across six crates, two pairs of which are structurally identical, means every path that crosses a crate boundary re-copies rather than passes. The input-region path in particular converts twice on one call: layout `Geometry` (f64) is floored/ceiled into `InputRect` (i32) at paint.rs:101-113, while the…

**Fix.** Delete `InputRect` and re-export `mold_region::Rect` from mold-wayland (or move the i32 rect into mold-region as the single owner and have mold-wayland use it directly). That removes the map+collect at client_layer.rs:284-293 entirely. Same treatment applies to ImageRect/DamageRect if the u32 pair is ever worth unifying.

## Dead code — nothing produces it, or nothing consumes it

Every entry here was proved dead by grepping for producers *and* consumers, not just definitions.

_11 findings — 1 high, 6 medium, 4 low._

#### 58. PaintContext::opacity is provably always 1.0 — the whole colour-folding opacity path (with_opacity, DrawCommand::Texture::opacity) is dead, superseded by offscreen layers

*high · certain · render-cpu*

`crates/mold-render/src/paint.rs:43`, `crates/mold-render/src/paint.rs:68`, `crates/mold-render/src/commands.rs:361`, `crates/mold-render/src/effects.rs:304`, `crates/mold-render/src/commands.rs:97` (+1 more)

**Costs.** Two mechanisms exist on paper for node opacity — fold it into every colour, or composite an offscreen layer — and only the second one runs. The dead one costs a f64 in PaintContext, an extra parameter on five functions, an always-1.0 field on every Texture command, and 13 multiply sites that mislead anyone reading the paint pass. Worse,…

**Fix.** Delete `PaintContext::opacity`, `with_opacity`, the `opacity` parameters, and `DrawCommand::Texture::opacity`; make layers the single opacity mechanism. Or, if the cheap path is wanted back, restrict `creates_layer` to the cases that genuinely need an offscreen pass (blur, shadow, rounded clip, rotation, explicit `layer.enabled`, and a…

#### 59. `LayerEvent::Modifiers` and `LayerEvent::OutputPower` are produced every time and discarded by every consumer

*medium · certain · cli-wayland*

`crates/mold-wayland/src/input_handlers.rs:299`, `crates/mold-wayland/src/protocol_handlers.rs:140`, `crates/mold-cli/src/surface_events.rs:438`, `crates/mold-cli/src/surface_events.rs:84`, `crates/mold-cli/src/lock.rs:180` (+1 more)

**Costs.** A Lua config can never see modifier state or learn that an output changed power state, yet the client pays a VecDeque push (and a String-free but non-trivial event) on every modifier keystroke. Either the events are a missing feature or they are two variants, four discard arms and a producer each to delete.

**Fix.** Delete `LayerEvent::Modifiers` and `LayerEvent::OutputPower`, their producers, and the six arms that mention them — or wire them to `runtime.dispatch_*` the way `Idle` and `Clipboard` are. Do not leave them produced-and-dropped.

#### 60. Unused PRIMARY_LAYER wrapper API and the write-only half of the popup reposition-token bookkeeping

*medium · certain · cli-wayland*

`crates/mold-wayland/src/client_surface.rs:44`, `crates/mold-wayland/src/client_surface.rs:54`, `crates/mold-wayland/src/client_surface.rs:87`, `crates/mold-wayland/src/client_surface.rs:92`, `crates/mold-wayland/src/client_layer.rs:201` (+2 more)

**Costs.** Two ways to address the shell's own layer surface, with the shorter one unused, is exactly the parallel path the project bans; and the ack bookkeeping makes the popup path look like it correlates configures when it does not, which is a trap for the next person debugging popup repositioning.

**Fix.** Delete the five unused wrappers (keeping `request_frame`/`surface`, still used by examples/layer_smoke.rs, or port that example to the `_layer` forms and delete those too), and reduce `PopupReposition` to the `sent` counter, dropping `acknowledged`, `record_reposition_ack` and `popup_reposition_token`.

#### 61. Eleven public functions across six crates have zero references anywhere in the workspace — not in production code, tests, or examples

*medium · certain · cross-cutting*

`crates/mold-desktop/src/lib.rs:126`, `crates/mold-image/src/icons.rs:41`, `crates/mold-image/src/image_cache.rs:98`, `crates/mold-layout/src/transform.rs:68`, `crates/mold-reactive/src/lib.rs:193` (+6 more)

**Costs.** Two of these are not merely unused, they are actively misleading. `retain_scene` is the missing eviction for an unbounded cache (see the TransformTracker finding) — its existence makes the cache look bounded on a skim. `remove_effect` is the reactive graph's teardown path, whose absence means effects are only ever added; whether that…

**Fix.** Delete all eleven, except `retain_scene` and `remove_effect`, which should instead be WIRED UP — `retain_scene` into `remove_scene_subtree` (crates/mold-lua/src/runtime_helpers.rs) and `remove_effect` into whatever tears down a Lua effect — or deleted with a deliberate note that the corresponding resource is never reclaimed.

#### 62. Flickable's `content_width` / `content_height` schema properties have no reader anywhere in the engine

*medium · certain · scene*

`crates/mold-scene/src/schema.rs:65`, `crates/mold-scene/src/schema.rs:66`, `crates/mold-layout/src/layout.rs:394`

**Costs.** A config author can write `Flickable{ content_width = 2000 }`, have it accepted and coerced, and get no behaviour — the property is silently inert. Every Flickable also pays two extra signal allocations per node in `Scene::create` (scene.rs:22-51 builds a `current` and a `target` signal, each with a formatted name string, for every…

**Fix.** Either delete both from schema.rs:65-66, or make them load-bearing by clamping `content_x`/`content_y` to `0..=(content_extent - viewport_extent)` in layout.rs before the offsets at layout.rs:395-396.

#### 63. `PropertyClass` and the `node`/`property`/`class` fields of `AnimatedChange` are computed every frame and read by nothing outside tests

*medium · certain · scene*

`crates/mold-scene/src/animation.rs:3`, `crates/mold-scene/src/animation.rs:292`, `crates/mold-scene/src/motion.rs:389`, `crates/mold-scene/src/scene.rs:288`, `crates/mold-scene/src/scene.rs:343` (+2 more)

**Costs.** A whole public vocabulary (`PropertyClass::{Transform,Layout,Paint}`) that describes damage granularity is maintained, kept in sync with the schema by hand, and consumed by no renderer — it is a second classification of properties alongside `affects_layout` (motion.rs:448), and only `affects_layout` actually drives anything. Every frame…

**Fix.** Either wire the classification into the renderer (a `Transform`-only change should skip the draw-list rebuild, which is what the enum was clearly meant for), or delete `PropertyClass`, `property_class`, and `AnimatedChange`'s fields and replace `frame.changes` with a `changed: usize` or `bool` — the two consumers only need `!is_empty()`.…

#### 64. glyph.wgsl's single-channel coverage branch (mode.z) is unreachable — no producer can set mode.z without also setting mode.w

*medium · likely · shaders*

`crates/mold-render/src/glyph.wgsl:121`, `crates/mold-render/src/glyph.wgsl:126`, `crates/mold-render/src/gpu/glyph_batch.rs:153`, `crates/mold-render/src/gpu/textures.rs:351`, `crates/mold-render/src/gpu/glyphs.rs:354` (+1 more)

**Costs.** Three coupled leftovers from the coverage-atlas text path that the distance-field path replaced: a shader branch, a redundant uniform lane, and a subpixel unpack for a format the font stack never emits. Each one has to be reasoned about on every future change to glyph.wgsl's mode encoding, and the textures.rs:351 write actively misleads…

**Fix.** Drop `mode.z` from the encoding: delete glyphs.rs:353-361, delete glyph.wgsl:121 and :126's `select(...)` (colour glyphs sample `.a` and multiply RGB, which is the only surviving case), and set textures.rs:348-353's mode to `[0.0, 0.0, 0.0, f32::from(style.distance_field)]`. If `RasterContent::Mask` really is now unproducible, collapse…

#### 65. Dead `"none"` match arm in scene_gradient, and a permanently-zero SdfFieldLayer slot documented as a corner radius that the shader never reads

*low · certain · render-cpu*

`crates/mold-render/src/effects.rs:150`, `crates/mold-render/src/field.rs:16`, `crates/mold-render/src/field.rs:91`

**Costs.** Both are small, but each is a false signal in a hot, deliberately dense file: the dead arm implies `scene_gradient` still handles "none" late (so a reader may add a second early-out or move the check), and the `params` doc names a producer/consumer pair that does not exist, which is exactly the kind of stale claim that costs a debugging…

**Fix.** Delete the `"none" => Gradient::None,` arm at effects.rs:150 (the `kind =>` catch-all already errors on unknown names). Rename the `params.x` slot to `_pad` and fix the doc comment to `[unused, points, inner radius, thickness]`, or fold `points`/`inner_radius`/`thickness` into three slots and drop the fourth.

#### 66. `SplitParser::new` returns an `io::Result` that can never be `Err`

*low · certain · services*

`crates/mold-io/src/streams.rs:38`, `crates/mold-lua/src/api_socket.rs:306`

**Costs.** A dead error path that misleads readers into thinking the delimiter is validated, sitting directly on top of a real unvalidated input whose only guard is a check in a different function. Anyone adding a second call site for `find_bytes` reintroduces a config-reachable panic.

**Fix.** Either make `new` return `Self` and drop the `map_err` at the call site, or make the `Result` real by rejecting an empty delimiter in `new` and in `set_delimiter` — which then also lets `push` and `find_bytes` drop their empty-delimiter special cases.

#### 67. mold-scene uses mold-reactive as a plain signal arena — no effect is ever registered, so its per-frame flush and error check are dead

*low · certain · services*

`crates/mold-scene/src/scene.rs:361`, `crates/mold-scene/src/types.rs:204`, `crates/mold-reactive/src/lib.rs:255`, `crates/mold-reactive/src/lib.rs:310`

**Costs.** It reads as if scene properties participate in the reactive graph — they don't. Anyone debugging a scene-property update chases invalidation machinery that is inert here, and the unreachable `SceneError::Reactive` arm invites the assumption that scene effects exist and can fail.

**Fix.** Either register the scene's property dependencies as real effects (if that was the intent), or state the actual relationship: Scene needs a generational arena with change detection, not a reactive graph. Splitting a `Signals<T>` (signal/read/write) out of `Graph<T>` and having Scene hold that would delete the dead flush, the dead error…

#### 68. SdfFieldLayer.params[0] is always written 0.0 and never read, and the Rust doc claims it holds a corner radius

*low · certain · shaders*

`crates/mold-render/src/field.rs:16`, `crates/mold-render/src/field.rs:91`, `crates/mold-render/src/field.wgsl:20`, `crates/mold-render/src/field.wgsl:198`

**Costs.** Sixteen wasted bytes per layer in the storage buffer is nothing; the wrong doc is the cost. Anyone reading field.rs will believe `params[0]` carries a radius and that `radii` is redundant, or will 'fix' the zero by writing a radius there that the shader ignores.

**Fix.** Shrink `params` to `[f32; 3]`-worth of meaning by repacking (`params: [points, inner_radius, thickness, unused]`) in both field.rs and field.wgsl together, or at minimum change field.rs:16's doc to match field.wgsl:20's `unused`. `extra[3]` (field.rs:100, field.wgsl:23) is the same situation.

## Repetition — the same block, copy-pasted

Ranked by how much a divergence between the copies would cost.

_11 findings — 0 high, 3 medium, 8 low._

#### 69. `paint_popup_surface` and `paint_floating_surface` are the same 52 lines with four identifiers renamed

*medium · certain · cli-wayland*

`crates/mold-cli/src/paint.rs:181`, `crates/mold-cli/src/paint.rs:234`, `crates/mold-cli/src/paint.rs:51`, `crates/mold-cli/src/paint.rs:193`, `crates/mold-cli/src/paint.rs:246`

**Costs.** Two copies of the frame path for two surface kinds that are, at this level, the same surface kind — any fix to one (the missing repaint gate in finding 1, partial damage instead of the full-surface `damage_buffer(0, 0, w, h)`) has to be remembered twice, and the `still_valid` inlining means the cache predicate now exists in three places.

**Fix.** One `paint_auxiliary_surface(runtime, client, surface, kind)` where `kind` supplies the request-frame call, the surface lookup and the error string — or a small trait with those three members. Use `CachedLayout::still_valid` in it instead of re-inlining the comparison.

#### 70. The fuel-metered executor loop is copy-pasted ten times, and two copies differ in ways that matter

*medium · certain · lua*

`crates/mold-lua/src/reactive_execute.rs:97`, `crates/mold-lua/src/reactive_execute.rs:103`

**Costs.** Ten copies of the sandbox's fuel accounting is ten places a metering fix has to land, and the two existing divergences show the drift is real, not hypothetical. The frame cap that `execute_effect` maintains is a rule about how much Lua may run per frame, and it currently lives in one of ten copies.

**Fix.** One `fn drive(ctx, executor, budget, what: &str) -> Result<(), String>` holding the loop, taking the budget and the label for the exhaustion message; each caller then differs only in how it starts the executor and how it takes the result. Decide explicitly whether the frame budget applies to handlers, delegates and variants, and put the…

#### 71. `dbus_argument_value` and `inferred_dbus_value` are the same 12-line match written twice, differing only in two error strings

*medium · certain · services*

`crates/mold-io/src/dbus_encode.rs:1`, `crates/mold-io/src/dbus_encode.rs:137`

**Costs.** Two copies of the DbusValue->zvariant scalar mapping means every future variant added to `DbusValue` has to be added twice, and a fix applied to one copy silently misses the other — exactly the shape of the divergence already proven above on the decode side.

**Fix.** Keep one function taking the context string: `fn dbus_scalar_value(value: &DbusValue, context: &str) -> Result<Value<'_>, String>` producing "nil cannot be a {context}" / "compound values need an explicit signature", called with "positional D-Bus argument" and "D-Bus variant".

#### 72. The apply-pending-service-requests block is written six times in three different subsets, and output power is missing from the post-event copies

*low · certain · cli-wayland*

`crates/mold-cli/src/surface_run.rs:119`, `crates/mold-cli/src/surface_run.rs:221`, `crates/mold-cli/src/surface_run.rs:244`, `crates/mold-cli/src/lock.rs:65`, `crates/mold-cli/src/lock.rs:83` (+1 more)

**Costs.** Six copies of one list is six chances to add a seventh service to five of them; the existing omission is already a real, if small, latency asymmetry between one service and the other five.

**Fix.** One `fn apply_service_requests(runtime: &mut Runtime, client: &mut LayerClient)` containing all six calls, invoked at each of the three points in both loops.

#### 73. A third copy of the logical-to-physical size formula, this one without the clamps

*low · certain · cli-wayland*

`crates/mold-cli/src/surfaces.rs:380`, `crates/mold-wayland/src/helpers.rs:1`, `crates/mold-image/src/image_cache.rs:238`

**Costs.** Three copies of one conversion, and the copy that sizes GPU swapchains is the one without the floor — it is protected only by the convention that `scale_120` is never zero (`scale.max(1)` at protocol_handlers.rs:87 and surface_handlers.rs:19), which is a guarantee held two crates away.

**Fix.** Export mold-wayland's `physical_size` and delete `auxiliary_physical_size`, adapting the two call shapes.

#### 74. The icon resolve-or-cache block is copy-pasted three times, two copies byte-identical, and `ImageCache::load_icon` has no callers

*low · certain · geometry*

`crates/mold-image/src/image_cache.rs:117`, `crates/mold-image/src/image_cache.rs:170`, `crates/mold-image/src/image_cache.rs:216`, `crates/mold-image/src/image_cache.rs:98`

**Costs.** Three copies of the icon lookup mean the next change to icon caching (a size bucket, an eviction, a resolver reused instead of rebuilt per miss) has to be made three times, and the third copy has already drifted into a bare `120`. `load_icon` is a public entry point nobody uses that will nonetheless be kept working. The grid duplication…

**Fix.** Extract `fn resolve_icon(&mut self, name: &str, theme: &str, physical: u32) -> Result<PathBuf, ImageError>` and call it from all three; have `icon_intrinsic_size` pass `physical_size(preferred_size, 120)?` explicitly or, better, take a `scale_120` like its siblings. Delete `load_icon`. In mold-layout, have `resolve_children` call a…

#### 75. The Lua-event-name table is written out twice, in opposite directions, with nothing keeping the two in step

*low · certain · lua*

`crates/mold-lua/src/configure.rs:123`, `crates/mold-lua/src/configure.rs:142`, `crates/mold-lua/src/events.rs:37`, `crates/mold-lua/src/events.rs:55`, `crates/mold-lua/src/runtime_events.rs:332`

**Costs.** Adding an event means editing two `match`es in two files; the compiler catches a missing arm in `property()` (exhaustive on the enum) but not a missing or misspelled arm in `handler_event`, which fails as "unknown property" at config time. A disagreement between them mislabels every error log for that event.

**Fix.** One `const` slice of `(UiEvent, &'static str)` pairs, with `handler_event` and `property` both derived from it — the same shape already used for `SurfaceReserve::edges()` (surface_types.rs:40-48).

#### 76. The window-surface registration block is pasted three times in `api_module.rs`

*low · certain · lua*

`crates/mold-lua/src/api_module.rs:311`, `crates/mold-lua/src/api_module.rs:317`, `crates/mold-lua/src/api_module.rs:376`, `crates/mold-lua/src/api_module.rs:382`, `crates/mold-lua/src/api_module.rs:426` (+1 more)

**Costs.** Adding a fourth surface kind, or a field to `WindowSurfaceConfig`, means three near-identical edits, and a miss in one of them is a surface kind that silently skips a validation or forgets to raise `window_surfaces_changed`.

**Fix.** One `fn register_window_surface(ctx, state, root, visible, updates_enabled, kind) -> Result<UserData, HostError>` doing the element check, the id allocation, the insert, the change flag and the userdata; the three constructors then reduce to parse + call.

#### 77. The Field command's reachable-area rectangle is computed twice, in two different spaces, by two hand-written copies of the same min/max-plus-spread loop

*low · certain · render-cpu*

`crates/mold-render/src/commands.rs:260`

**Costs.** These two must stay identical or the frame breaks: the first is what gets scissored as damage, the second is what the shader actually rasterizes. Any future change to how a field's reach is computed — a new operator that spreads differently, a per-layer stroke — has to be made in both, and the failure mode when it is not is a field drawn…

**Fix.** Extract one `fn field_reach(bounds, stroke_width, softness, layers) -> Geometry` returning the node-local reach rectangle, and have `DrawCommand::bounds()` offset+transform it while `field_area` scales it into `[l,t,r,b]`.

#### 78. The `span.is_zero()` special case in `keyframe_steps` builds a step identical to the one the fallthrough builds

*low · certain · scene*

`crates/mold-scene/src/keyframes.rs:69`, `crates/mold-scene/src/keyframes.rs:81`

**Costs.** Twelve lines whose comment claims a behavioural distinction ("a deliberate jump") that the code does not make. A reader trusting the comment will assume coincident stops are handled specially and will not look for the actual zero-duration path — which is `Behavior::intercepts()` returning false in `animate_from` (scene_behavior.rs:83),…

**Fix.** Delete the `if span.is_zero() { ... continue; }` block at keyframes.rs:69-80 and keep the single push, moving the explanatory comment onto it. If a coincident-stop jump is genuinely meant to differ from a zero-length tween, make it differ — right now it does not.

#### 79. The full-screen-triangle vertex shader is written out three times, byte-identical in blur.wgsl and composite.wgsl

*low · certain · shaders*

`crates/mold-render/src/gpu/quad_pipeline.rs:8`, `crates/mold-render/src/gpu/quad_pipeline.rs:43`, `crates/mold-render/src/gpu/quad_pipeline.rs:54`

**Costs.** Three copies of the same twelve lines means a fix to the UV flip or the oversized-triangle trick has to land in three files. The stray `Viewport` uniform and the `instances: bool` switch are residue from when clear shared sdf.wgsl's vertex stage — they make the clear pass look like it depends on the viewport when it does not.

**Fix.** Build the blur, composite and clear shader sources by concatenating one `fullscreen.wgsl` prelude string at `create_shader_module` time (`include_str!("../fullscreen.wgsl")` + the pass body), delete clear.wgsl's `Viewport` struct and binding, and give the clear pass its own two-line pipeline constructor instead of the `instances: bool`…
---

## Outside `crates/*/src` — what a code sweep structurally cannot see

### Build and CI

**The release workflow cannot ever have succeeded.**
`.github/workflows/release.yml:31-33` runs `cargo build --release --example main`
and packages `target/release/examples/main`. There is no example named `main` in
the workspace — `cargo metadata` lists ten, none of them `main` — and the shipped
binary is `mold` (`crates/mold-cli/Cargo.toml:10-12`). The same file also builds
this Wayland shell on `macos-15-intel` and `macos-14`.

**`svgtypes` is a dependency of nothing.** Declared at
`crates/mold-render/Cargo.toml:16` and `Cargo.toml:28`; zero hits in any `.rs`
file. It was the other half of the deleted path renderer — `lyon_tessellation` went
with `path.rs`, this did not. The same removal left `"path"` as a live match arm in
`property_class` (`crates/mold-scene/src/motion.rs:424`) for a property no schema
defines any more.

**`crates/mold-cli/src/lib.rs` is one byte** — a single newline. With no `[lib]`
stanza suppressing it, Cargo auto-discovers it and publishes an empty `mold_cli`
lib target that is compiled and linted on every CI run.

**Why `-Dwarnings` never caught the eleven dead public functions.**
`tests.yml:29` runs clippy with `-Dwarnings` and there is no `[lints]` table
anywhere. `dead_code` does not fire for `pub` items in a lib crate — and per the
next section, every crate here is one flat module where everything is `pub`. The
gate cannot see any of it. Fixing that mechanism is what makes this cleanup stick
rather than recur.

**The root `tests/` directory is dead** — one fixture referenced by nothing, in a
directory Cargo ignores at a workspace root regardless.

### Test coverage

**16 of 21 GPU tests never run in CI.** All are `#[ignore = "requires a GPU
adapter"]` and `tests.yml:30` has no second `--ignored` pass. They are precisely
the tests covering what this sweep flagged hardest — the corner-radius shape
divergence, the `field_area` rotation bug, the `DistanceFieldStyle.weight` unit
collision. `--all-targets` also silently disables doctests.

**10 of 17 shipped Lua examples are executed by nothing** — including all eight
`sdf-*.lua`, i.e. the entire demo surface for the pipeline that was just rewritten.
They all load when run by hand, so this is latent rather than broken.

**`frame_bench` panics on the flagship example.** `frame_bench examples/board/init.lua`
→ index out of bounds at `crates/mold-cli/examples/frame_bench.rs:208`. It builds
`Runtime::default()`, which has no screens, and `board/init.lua:1298` uses
`mold.variants(own_screen and { own_screen } or {}, …)` — the *documented*
multi-monitor idiom — so no root node is created. The project's own benchmarking
tool cannot profile a config written the recommended way. It needs
`Runtime::for_screen`, which the unit tests already use.

**Three of the smoke gates were red and nobody knew**, because none of them runs
in `test`. Running the whole recipe list after the conversion found all three, and
all three are now fixed:

- `gpu-smoke` panicked with `range end index 4 out of range for slice of length 3`.
  Its `Layer` claimed `commands: 0..4` while the list held three — the fourth was
  the `DrawCommand::Shape` deleted with lyon, and the range was never narrowed. The
  gate that exists precisely to catch a bad `DrawList` was itself feeding one in.
- `io-smoke` failed `NotFound`. `SocketServer`'s `Drop` unlinks the socket, and the
  example then unlinked it a second time by hand — so the example passed only in
  the window before the server was dropped, and joining the thread closes it.
- `pam-smoke` could not load libpam. The loader tried `libpam.so.0` by soname and
  the two Debian multiarch directories and nothing else, so on Arch, Fedora or SUSE
  — where it sits in a flat `/usr/lib` — **the lock screen cannot authenticate at
  all**. Now widened to `/usr/lib`, `/lib` and `/usr/lib64`. It still fails on this
  particular machine, one step further along: the nix-provided glibc has no
  `ld.so.cache`, so libpam's own `libaudit.so.1` will not resolve by soname. That
  part is the environment, not mold.

`popup-smoke` blocks waiting for a real click, by design, so it cannot be part of an
unattended run. That is worth knowing before anyone wires the recipe list into CI.

`crates/mold-lua/examples/vrfy_perframe_tx.rs` is a committed debugging scratch
that asserts nothing and that CI compiles on every push.

---

## The structural cause: `include!` instead of `mod` — resolved

**Done.** Every table row below is now zero, and the numbers are kept as the
record of what it was. What the conversion actually surfaced, once each file
became a module with a boundary: **413 unused imports** the flat namespace could
not report, several files over the 500-line gate that had been hiding inside a
larger include chain (`mold-lua/src/state.rs` split, `mold-render/src/commands/`
nested), and a long tail of items that had been public only because there was no
smaller scope to be private in — those are `pub(crate)` now. The one hazard worth
recording for anyone doing this again: the compiler's unused-import diagnostic
reports a *trait* import as unused when it is needed only for method resolution,
so a blind prune breaks the build in a way that reads as "no method named X"
rather than "missing import". Prune, then build, then re-check.

Most of the repetition and all of the dead code above share one root. The file
split across this workspace is cosmetic:

| crate | `include!` | real `mod` |
|---|---|---|
| mold-lua | 46 | 1 |
| mold-wayland | 16 | 1 |
| mold-scene | 14 | 2 |
| mold-cli (`main.rs`) | 13 | 0 |
| mold-io | 9 | 1 |
| mold-render | 7 | 2 |
| mold-layout | 5 | 1 |
| mold-image | 4 | 1 |

Only `mold-services` and `mold-text` use real modules. Everywhere else the crate is
**one flat namespace with no privacy boundaries** — no `pub(super)`, no per-file
unused-import or unused-private-function diagnostics, and no way for the compiler
to notice that `api_time.rs` and `api_animation.rs` grew two easing APIs, or that
`color_quantize` and `color_quantizer` are the same call.

This is why `-Dwarnings` is silent on eleven dead public functions, and it is why
`mold-lua` is a 12,109-line crate that duplicates itself. **Any "split mold-lua up"
task that does not convert `include!` to `mod` changes nothing** — the compilation
unit is already one module regardless of how many files it is spread across.

Converting is mechanical but not free: every cross-file reference needs a `use`,
and items currently visible by accident will need deliberate visibility. Doing it
crate by crate, smallest first, is the way in.

---

## One bug wearing five costumes: nothing tells anyone a node died

The sweep reported four unbounded caches independently. The critic found a fifth
and showed they are one problem.

`grep -rn "retain_scene\|prune\|evict" crates` returns exactly **one** production
hit in the entire workspace: `crates/mold-layout/src/transform.rs:68` — the
function with no callers. There is no node-liveness notification of any kind that
crosses a crate boundary.

The only cleanup that exists is `remove_scene_subtree`
(`crates/mold-lua/src/runtime_helpers.rs:53-90`), an exhaustive twelve-map sweep of
`ReactiveState`. It is thorough, and it lives inside `mold-lua`, so it structurally
cannot reach anything downstream — the caches live in `WgpuBackend` and
`TextSystem`, constructed over in `mold-cli`. Every cache on the far side of that
boundary has an eviction API and no wiring:

| cache | eviction API | callers |
|---|---|---|
| `TransformTracker` geometry | `retain_scene` | 0 |
| `WgpuBackend::image_textures` | `clear()` | 0 |
| `ImageCache` | `clear()` | 0 |
| `TextSystem::buffers` | `TextSystem::remove` | 0 |
| mold-lua transform geometry | — | — |

`TextSystem::buffers` is the worst of them: one `CachedBuffer` per Text node ever
measured, each holding a full cosmic-text `Buffer` with its shaped runs and
per-glyph vectors plus owned `String`s. Every view switch, Loader swap, VirtualList
recycle and soft reload leaks one per text node, forever. Its only external door,
`WgpuBackend::text_mut`, is itself in the dead-function list — so the eviction is
unreachable twice over.

**Fix.** One node-destruction signal that crosses crate boundaries — the scene
already tracks removals and already bumps a revision, so it can accumulate a
removed-node list that `mold-cli` drains once per frame and fans out to every
cache. Five leaks close at once, and the five orphaned eviction APIs get their
first caller.

---

## Suggested order

**First — the bugs.** Findings 1–20 are the high-severity set. Within them, start
with the ones a configuration can trigger today: the corner-radius shape
divergence, the stale fling target that silently drops a later write, the
`animate_from` missing `touch_layout`, the ClipRect invisible to the damage
differ, and the popup/floating surfaces that repaint forever.

**Second — the node-liveness signal.** One change, five leaks closed, and it is a
prerequisite for trusting any of the caches.

**Third — the mechanism, not the symptoms.** `include!` → `mod`, crate by crate,
then a `[lints]` table. Without this the dead code and the duplicate APIs grow
back, because nothing in the toolchain can see them.

**Fourth — CI honesty.** Fix the release workflow, add the `--ignored` GPU pass,
and fix `frame_bench` so the benchmark tool can open the flagship config. Right
now the gates are green on things that do not work.

**Then** the parallel implementations and the repetition, in the order they get in
the way. The `Scale` newtype (three disagreeing copies of one formula) and the
fuel-loop consolidation (ten copies, two of which differ in ways that matter) are
the two with the best ratio of risk removed to effort.

**Last** the two big design merges — one shape vocabulary, and one field pipeline.
Each deserves its own branch, with GPU tests written before the merge rather than
after.
