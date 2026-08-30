# Native motion and geometry stack

Mold uses three focused Rust libraries together. None of them turns Lua into a
runtime, and none introduces widgets.

| Library | Mold responsibility | Integration boundary |
|---|---|---|
| [Animato 1.7.2](https://github.com/AarambhDevHub/animato) | Tween easing and spring state updates | `mold-scene`; Mold supplies frame deltas and retains the compositor clock |
| [Polymorpher 0.1.4](https://docs.rs/polymorpher/0.1.4/polymorpher/) | Rounded-polygon topology matching and cubic morph curves | `mold-render`; generated paths use the existing Lyon tessellation path |
| [`signed-distance-field` 0.6.3](https://crates.io/crates/signed-distance-field) | Binary alpha-mask distance transforms | `mold-image`; results are cached by decoded source and spread before GPU upload |

Animato's published timeline, loop, callback, and orchestration facilities are
not exposed yet. Mold currently uses its tween and spring crates through the
existing property behavior API. This preserves Mold's scene ownership,
retargeting rules, damage tracking, and Rust frame clock.

Polymorpher currently accepts these built-in shape names through
`morph_from`/`morph_to`:

- `circle`, `square`, `slanted`, `arch`, `fan`, `arrow`, `semi_circle`, `oval`,
  `pill`, `triangle`, `diamond`, `clam_shell`, `pentagon`, and `gem`;
- `sunny`, `very_sunny`, `cookie4`, `cookie6`, `cookie7`, `cookie9`, `cookie12`,
  `ghostish`, `clover4`, `clover8`, `burst`, `soft_burst`, `boom`, and
  `soft_boom`;
- `flower`, `puffy`, `puffy_diamond`, `pixel_circle`, `pixel_triangle`, `bun`,
  and `heart`.

`morph_progress` is a normal animated numeric scene property. Lua declares the
two shape names and changes the target progress; Rust builds, caches,
tessellates, and renders the intermediate cubic path.

`distance_field = true` converts an Image or Icon alpha mask once per cache key.
`distance_field_spread` controls the source-pixel distance range encoded around
the edge. The CPU transform is not run for every animation frame and is not used
to implement live polygon morphing.

## License and maintenance notes

- Animato is MIT OR Apache-2.0 and declares Rust 1.89.
- Polymorpher is Apache-2.0 and declares Rust 1.85.1.
- `signed-distance-field` is MIT, CPU-only, and marked passively maintained.

The workspace pins the reviewed versions exactly so these integration contracts
do not drift on an ordinary dependency resolution.
