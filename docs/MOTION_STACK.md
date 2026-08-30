# Native motion and geometry stack

Mold uses three focused Rust libraries together. None of them turns Lua into a
runtime, and none introduces widgets.

| Library | Mold responsibility | Integration boundary |
|---|---|---|
| [Animato 1.7.2](https://github.com/AarambhDevHub/animato) | Tween clocks with delay, time scaling, and looping, plus spring state updates | `mold-scene`; Mold supplies frame deltas and retains the compositor clock |
| [Polymorpher 0.1.4](https://docs.rs/polymorpher/0.1.4/polymorpher/) | Built-in and parametric rounded polygons, topology matching, and cubic morph curves | `mold-render`; generated paths use the existing Lyon tessellation path |
| [`signed-distance-field` 0.6.3](https://crates.io/crates/signed-distance-field) | Binary alpha-mask distance transforms | `mold-image`; results are cached by decoded source and spread, then sampled with a live edge on the GPU |

Mold drives an Animato `Tween` as the clock behind every timed property, which
is where `delay`, `time_scale`, `Loop`, pause, resume, seek, and reset come
from. Animato's own timeline and orchestration crates are not pulled in: the
scene keeps ownership of scheduling so Mold's retargeting rules, damage
classification, and Rust frame clock stay in one place.

A spring settles when both its distance from the target and its velocity fall
under its `epsilon`, and that threshold is in the animated property's own units.
The same number therefore means very different things across properties: `0.001`
is a sensible resting threshold for `scale` or `opacity`, which live in nought to
one, but on a width measured in hundreds of pixels it asks the spring to come
within a thousandth of a pixel — hundreds of frames of motion no one can see,
with the compositor painting every one of them. Scale the threshold to the
quantity: a twentieth of a pixel settles promptly and is already invisible.

A behavior therefore reads its repetition from Animato and reports its own end.
An alternating repetition that finishes on a backward pass settles on the value
it started from, and the settled target is corrected to match, so the property
level and the target level never disagree once motion stops.

Polymorpher currently accepts these built-in shape names through
`morph_from`/`morph_to`:

- `circle`, `square`, `slanted`, `arch`, `fan`, `arrow`, `semi_circle`, `oval`,
  `pill`, `triangle`, `diamond`, `clam_shell`, `pentagon`, and `gem`;
- `sunny`, `very_sunny`, `cookie4`, `cookie6`, `cookie7`, `cookie9`, `cookie12`,
  `ghostish`, `clover4`, `clover8`, `burst`, `soft_burst`, `boom`, and
  `soft_boom`;
- `flower`, `puffy`, `puffy_diamond`, `pixel_circle`, `pixel_triangle`, `bun`,
  and `heart`.

Six shape families also accept colon-separated numeric parameters, which is how
a shell reaches outlines the fixed table does not enumerate:

| Name | Parameters |
|---|---|
| `polygon:6:0.25:0.4` | vertex count, corner radius, corner smoothing |
| `star:9:0.45:0.2:0.3` | point count, inner radius ratio, outer rounding, inner rounding |
| `circle:24` | segment count |
| `rectangle:0.3:0.5` | corner radius, corner smoothing |
| `pill:0.4:2:1` | endcap smoothing, width, height |
| `pill_star:8:0.5:0.5` | point count, inner radius ratio, vertex spacing |

Trailing parameters may be omitted and fall back to the family's own default; a
bare `polygon`, `rectangle`, or `pill_star` builds entirely from defaults. Every
result is normalized into the same unit box the built-in names produce, so a
parametric shape morphs against a built-in one without special handling. A name
that is not a known family, or that carries a non-numeric parameter, is rejected
rather than silently treated as a default.

`morph_progress` is a normal animated numeric scene property. Lua declares the
two shape names and changes the target progress; Rust builds, caches,
tessellates, and renders the intermediate cubic path.

Tessellated geometry is antialiased by a one-pixel coverage band skirting the
outline, built from the triangle mesh rather than from the path: the outline is
the set of edges belonging to a single triangle, so the band does not depend on
how contours were wound and it follows the boundary of a hole as readily as an
outer edge. The band is extruded strictly outwards, so it never blends a second
time over the shape it is smoothing and a translucent fill stays exactly as
translucent as it was asked to be. It is generated once per cached tessellation,
not per frame, and its width tracks the scale the geometry was tessellated for.

Rounded rectangles take their coverage from the distance field directly. The
width of that edge comes from the screen-space derivative of the distance, so it
stays one pixel wide however the node is scaled, skewed, or rotated on its way
to the screen.

`distance_field = true` converts an Image or Icon alpha mask once per cache key.
`distance_field_spread` controls the source-pixel distance range encoded around
the edge. The CPU transform is not run for every animation frame and is not used
to implement live polygon morphing.

Because the cached texture holds a distance rather than a coverage mask, where
the edge falls is decided when the fragment shader samples it, not when the
field is built. Three further properties shape that edge and are ordinary
animatable scene properties:

| Property | Meaning |
|---|---|
| `distance_field_weight` | Normalized distance treated as the edge; below the neutral `0.5` the shape thickens, above it thins |
| `distance_field_softness` | Extra feathering in source pixels on top of pixel-derived coverage |
| `distance_field_outline_width` | Outline band outside the fill edge, in source pixels |
| `distance_field_outline_color` | Outline colour, composited beneath the fill |

Animating any of them changes the draw command but not the texture cache key, so
the CPU distance transform never re-runs for motion. Widths are given in source
pixels and converted against the encoded spread before reaching the shader.

## License and maintenance notes

- Animato is MIT OR Apache-2.0 and declares Rust 1.89.
- Polymorpher is Apache-2.0 and declares Rust 1.85.1.
- `signed-distance-field` is MIT, CPU-only, and marked passively maintained.

The workspace pins the reviewed versions exactly so these integration contracts
do not drift on an ordinary dependency resolution.
