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
| Idle handling | The frame timebase is dropped once the scene settles, so idle time is never charged to the next animation |
| Delay and time scaling | Animato tween `delay` and `time_scale` on every behavior |
| Loops and ping-pong | Animato `Loop` exposed as `loops` and `ping_pong` on a behavior |
| Playback control | Native pause, resume, stop, finish, restart, reverse, and seek |
| Enabling a behavior | `mold.animation.set_enabled` toggles one without discarding it |
| Completion callbacks | `on_finished(property, reason)` reports completed, stopped, or canceled |
| Compound interpolation | `mold.easing` evaluates a curve and interpolates numbers, points, rects, and colours |
| Sequential and parallel groups | Native Rust scheduler over ordinary property animations, with pause steps and repetition |
| Transform watching | Native Rust watcher includes geometry and ancestor transforms |
| Uniform scale and rotation | Native Rust affine transform |
| Independent X/Y scale | Native Rust `scale_x` and `scale_y` properties |
| Transform origin | Normalized native `transform_origin_x` and `transform_origin_y` properties |
| Translation and skew | Native Rust `translate_x`, `translate_y`, `skew_x`, and `skew_y` properties |
| Rotation path | `numerical`, `shortest`, `clockwise`, and `counterclockwise` paths |
| Interactive transformation proof | `examples/fluid-transform.lua` |
| Rounded polygon morphing | Polymorpher 0.1.4 topology matching and cubic path generation |
| Parametric shape families | `polygon`, `star`, `circle`, `rectangle`, `pill`, and `pill_star` with numeric parameters |
| Cached raster distance fields | `signed-distance-field` 0.6.3 alpha-mask conversion outside the frame hot path |
| Animated field edges | Weight, softness, and an outline band sampled per frame from the cached field |
| Combined native-stack proof | `examples/morph-stack.lua` and `examples/motion-lab.lua` |

The Lua example only declares property targets and handlers. Interpolation,
spring integration, transform composition, damage classification, layout hit
testing, and rendering remain in Rust.

Animato does not own Mold's runtime or compositor loop. Mold supplies frame
deltas from its Rust compositor clock and advances Animato state inside the
scene engine. Polymorpher and `signed-distance-field` are renderer/image
dependencies, not Lua modules. Their exact roles and limits are documented in
[`MOTION_STACK.md`](MOTION_STACK.md).

## Playback surface

A behavior declares its own timing, repetition, and completion handler:

```lua
behavior = {
  opacity = {
    duration = 400,
    easing = "in_out_cubic",
    delay = 120,          -- dead time before the interval starts
    time_scale = 0.5,     -- multiplier on every frame delta
    loops = 3,            -- pass count, "forever", or "ping_pong"
    ping_pong = true,     -- makes a pass count alternate direction
    enabled = true,
    on_finished = function(property, reason) end,
  },
}
```

`loops` carries the pass count because Lua reserves `repeat` as a keyword.
`reason` is `completed`, `stopped`, or `canceled`, and is delivered for spring
and smoothed motion as well as for tweens.

`mold.animation` controls motion already in flight. Every call names a node and
one of its properties and returns whether it found an animation to act on, so
Lua can branch without asking first:

| Call | Effect |
|---|---|
| `pause` / `resume` | Halts the clock in place and picks it up again |
| `stop` | Halts and pins the target to where the property stands |
| `finish` | Ends immediately at the target value |
| `restart` | Replays from the start, delay included |
| `reverse` | Retargets back to the value the animation set out from |
| `seek(node, property, t)` | Scrubs to a normalized position without ending it |
| `active` / `paused` / `progress` | Reports state; `progress` is `nil` when idle |
| `set_enabled` | Turns an installed behavior off without discarding it |

Physics motion has no timeline, so `reverse`, `seek`, and `progress` report
nothing for a spring while `pause`, `stop`, `finish`, and `restart` all apply.

## Animation groups

`mold.animation.play` schedules several property animations against one clock.
The array part is played in order; `parallel` and `sequential` nest, and `pause`
occupies time without changing anything:

```lua
local run = mold.animation.play {
  loops = 2,                                    -- pass count or "forever"
  on_finished = function(reason) end,
  { node = card, property = "opacity", to = 1, duration = 200, easing = "out_quad" },
  { pause = 120 },
  { parallel = {
      { node = card,  property = "x",        to = 240, duration = 400, easing = "out_back" },
      { node = badge, property = "rotation", from = 0, to = 360, duration = 400 },
  }},
}
run:pause(); run:resume(); run:stop(); run:finish(); run:active()
```

A step accepts the same timing fields a behavior does, plus an optional `from`;
without one it departs from wherever the property stands when its turn comes.

The group owns only the schedule. When a step's turn arrives it starts an
ordinary property animation, so retargeting, damage classification, and the
per-property controls all keep working on it. `pause` on the group stops it
starting further steps and leaves anything already running alone.

Every step must be able to finish, since the rest of the schedule waits on it:
a step that repeats endlessly is refused, as is an alternating group repetition.
Property names are resolved when the group starts, so a typo fails at the call
rather than partway through playback. A group whose nodes are destroyed is
dropped and reported as cancelled.

Sub-frame start times are exact. A step whose turn falls partway through a frame
receives the remainder of that frame as tween delay, so it ends the frame at the
progress its start time earned and a long sequence does not drift a frame
further out with every leg.

## Remaining gaps

Mold does not yet provide keyframe tracks with per-segment easing over a single
property. That is an engine primitive rather than a widget, so it can be added
as a native Rust scheduling API when required. It must not be implemented as a
Lua frame runtime.

Polymorpher covers built-in and parametric rounded polygons, not arbitrary SVG
path topology. Distance fields cover cached binary alpha masks sampled with a
live edge; they are not multi-channel fields or a general vector-effect graph.

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

Run the deepened stack with:

```sh
EXAMPLE=examples/motion-lab.lua oslo make run
```

It holds an endless ping-pong pulse, a delayed alternating sweep that can be
paused and resumed mid-flight, a parametric star whose point count changes while
the morph is running, and a distance-field glyph whose weight and outline band
animate off one cached field.
