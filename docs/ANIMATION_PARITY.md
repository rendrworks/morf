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
| Numeric and color property behavior | Native Rust behavior with target/current values |
| Easing | Native Rust easing families and cubic Bezier curves |
| Spring and smoothed motion | Native Rust physics with retargeted velocity preservation |
| State transitions | Native Rust property transitions and reparent transitions |
| Frame clock | Animation advances on Rust frame ticks without running Lua |
| Transform watching | Native Rust watcher includes geometry and ancestor transforms |
| Uniform scale and rotation | Native Rust affine transform |
| Independent X/Y scale | Native Rust `scale_x` and `scale_y` properties |
| Transform origin | Normalized native `transform_origin_x` and `transform_origin_y` properties |
| Translation and skew | Native Rust `translate_x`, `translate_y`, `skew_x`, and `skew_y` properties |
| Rotation path | `numerical`, `shortest`, `clockwise`, and `counterclockwise` paths |
| Interactive transformation proof | `examples/fluid-transform.lua` |

The Lua example only declares property targets and handlers. Interpolation,
spring integration, transform composition, damage classification, layout hit
testing, and rendering remain in Rust.

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

## Example

```sh
EXAMPLE=examples/fluid-transform.lua oslo make run
```

Click the shape to animate square-to-circle radius, color, shadow, non-uniform
origin-aware scale, skew, shortest-path rotation, and spring translation.
