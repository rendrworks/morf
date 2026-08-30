# Animation and transform parity

This audit compares Mold with the Quickshell source pinned in `xtra/quickshell`.
Quickshell obtains its general animation model from Qt Quick; the files below
show which parts the pinned Quickshell tree actually uses or exposes.

## Reference evidence

- `xtra/quickshell/src/ui/ReloadPopup.qml` uses `SequentialAnimation`,
  `NumberAnimation`, `PauseAnimation`, property `Behavior`, and
  `ColorAnimation`.
- `xtra/quickshell/src/ui/Tooltip.qml` uses property `Behavior` and a `Scale`
  transform with explicit X/Y origin and independent X/Y scale.
- `xtra/quickshell/src/core/easingcurve.hpp` exposes curve evaluation and
  interpolation for numbers, points, and rectangles.
- `xtra/quickshell/src/core/transformwatcher.cpp` watches geometry, scale,
  rotation, parent chains, and window chains.

## Mold coverage

| Capability | Mold status |
|---|---|
| Numeric and color property behavior | Native Rust behavior with target/current values; Animato 1.7.2 advances tween progress |
| Easing | Animato-backed easing families and cubic Bezier curves |
| Spring and smoothed motion | Animato-backed springs plus Mold smoothed motion, with retargeted velocity preservation |
| State transitions | Native Rust property transitions and reparent transitions |
| Frame clock | Animation advances on Rust frame ticks without running Lua |
| Transform watching | Native Rust watcher includes geometry and ancestor transforms |
| Uniform scale and rotation | Native Rust affine transform |
| Independent X/Y scale | Native Rust `scale_x` and `scale_y` properties |
| Transform origin | Normalized native `transform_origin_x` and `transform_origin_y` properties |
| Translation and skew | Native Rust `translate_x`, `translate_y`, `skew_x`, and `skew_y` properties |
| Rotation path | `numerical`, `shortest`, `clockwise`, and `counterclockwise` paths |
| Interactive transformation proof | `examples/fluid-transform.lua` |
| Rounded polygon morphing | Polymorpher 0.1.4 topology matching and cubic path generation |
| Cached raster distance fields | `signed-distance-field` 0.6.3 alpha-mask conversion outside the frame hot path |
| Combined native-stack proof | `examples/morph-stack.lua` |

The Lua example only declares property targets and handlers. Interpolation,
spring integration, transform composition, damage classification, layout hit
testing, and rendering remain in Rust.

Animato does not own Mold's runtime or compositor loop. Mold supplies frame
deltas from its Rust compositor clock and advances Animato state inside the
scene engine. Polymorpher and `signed-distance-field` are renderer/image
dependencies, not Lua modules. Their exact roles and limits are documented in
[`MOTION_STACK.md`](MOTION_STACK.md).

## Remaining gaps

Mold does not yet provide the explicit Qt Quick timeline/lifecycle surface:

- sequential and parallel animation groups;
- pause steps, loops, and ping-pong playback;
- completion, cancellation, stop, and restart callbacks;
- dynamically enabling or disabling an installed behavior;
- direct eased interpolation helpers for compound point and rectangle values.

These are engine primitives rather than widgets, so they can be added as native
Rust scheduling APIs when required. They must not be implemented as a Lua frame
runtime.

Polymorpher currently covers its built-in rounded polygons, not arbitrary SVG
path topology. Distance fields currently cover cached binary alpha masks, not
multi-channel fields or a general live vector-effect graph.

## Example

```sh
EXAMPLE=examples/fluid-transform.lua oslo make run
```

Click the shape to animate square-to-circle radius, color, shadow, non-uniform
origin-aware scale, skew, shortest-path rotation, and spring translation.

Run the combined dependency proof with:

```sh
EXAMPLE=examples/morph-stack.lua oslo make run
```

Click its shape to tween a native rounded-polygon morph while spring motion and
a cached signed-distance-field image render in the same scene.
