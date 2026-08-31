# Native motion and geometry stack

The stack answers two separate questions, and keeping them separate is what
makes the motion composable:

**Animato is the timing.** It answers *what value, at what moment* — tween
clocks with delay, time scaling and looping, and spring state. It knows nothing
about shape.

**Distance fields are the view.** They answer *what a shape is and how it is
drawn* — resolution independently, and with a defined way for two shapes to
combine. They know nothing about time.

Every animation in Mold is those two meeting: a number that Animato moves, read
by a field that decides what the frame looks like. Neither half needs to know
about the other, which is why a morph, a merge and a colour fade are all the
same mechanism.

| Library | Mold responsibility | Integration boundary |
|---|---|---|
| [Animato 1.7.2](https://github.com/AarambhDevHub/animato) | Tween clocks with delay, time scaling and looping; spring state; friction inertia | `mold-scene`; Mold supplies frame deltas and retains the compositor clock |
| [`signed-distance-field` 0.6.3](https://crates.io/crates/signed-distance-field) | Binary alpha-mask distance transforms | `mold-image`; results are cached by decoded source and spread, then sampled with a live edge on the GPU |

Analytic fields — the `Sdf` element below — are Mold's own, not a dependency.

Polymorpher was the third library and has been removed. It answered the same
question fields do, "what shape is this", by matching two rounded polygons by
topology and interpolating their outlines. That is strictly weaker than
interpolating the fields: an outline correspondence cannot survive a shape
splitting in two, cannot express a seamless join between neighbours, and needs
re-tessellating on the CPU for every frame of a morph. Two answers to one
question is worse than one, so the one that could do less is gone.

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

## Motion that has no destination

A behavior answers one question: *this property was assigned a value — how does
it travel there*. A tween walks a curve to it, a spring is pulled toward it, a
smoothing approaches it at a fixed speed. All three know where they are going
before they start.

A **fling** does not. It is thrown at a speed and stops where friction leaves
it, or where a bound catches it. Nothing chooses the landing point; it is a
consequence of how hard the throw was. That is the motion a flick wants — the
surface keeps going after the finger lifts — and it is what Animato's
`physics` feature provides, through `Inertia`.

```lua
mold.animation.fling {
  node = panel,
  property = "content_y",
  velocity = -1800,
  preset = "smooth",     -- or `snappy`, or `heavy`
  min = -4000, max = 0,  -- optional; the bound catches it and stops it dead
}
```

It is deliberately a **verb rather than another `kind` in a behavior table**.
`set_physics` refuses `Physics::Decay` for the same reason: a behavior is
started by an assignment, and there is no value to assign that would start a
fling. Writing the property while it coasts takes it back — and because the
scene hands the fling's position *and its velocity* to whatever animates it
next, a flick can be caught by a spring and carried past the target before it
comes back. That interaction is the point of expressing it as physics rather
than as a curve, and it is what the tests pin down.

### Forces: `mold.animation.impulse`

A fling *sets* a speed. An impulse *adds* to one:

```lua
-- Every so often, work out what pulls on what, and hand it over.
mold.animation.impulse(blob, "x", acceleration_x * step)
```

That is the whole difference between a flick and a force, and it is what lets a
configuration express motion the scene cannot know about on its own — one node
attracted to another, a drift towards the middle of the screen, a wind — without
becoming the clock. It computes the pushes at whatever rate suits it, thirty
times a second is plenty, and the engine goes on integrating every frame in
between. Writing positions instead would put the configuration back in the frame
loop, which is the one thing this stack exists to avoid.

It returns whether anything was pushed. A property that is not coasting has no
speed to add to, so it is left alone rather than started: starting one would be
a fling wearing the wrong name, and pushing a spring or a smoothing would only
be a fight with a target that is going to win.

`examples/sdf-blobs.lua` is the whole idea in one file. Six blobs attract each
other, nothing else pulls on them, and their opening throws are balanced so the
swarm's momentum sums to zero — mutual attraction cannot move a centre of mass,
so the lamp stays where it was put and turns there, indefinitely, for about a
tenth of one percent of a core.

The three presets are Animato's own: `smooth` for scrolling a list or a canvas,
`snappy` for something under direct manipulation, `heavy` for large panels.
`friction` and `min_velocity` override either number.

`examples/physics-lab.lua` runs a tween, a spring, a smoothing and a fling in
four lanes off one signal, so the difference between being handed a destination
and being handed a speed is visible in one frame.

Animato's physics also carries pointer drag with velocity smoothing, axis locks
and constraints, and swipe recognition. Neither is wired up yet; `Flickable`
would be the place for the first.

## Composed distance fields

Everything above computes a field on the CPU or bakes one into a texture. `Sdf`
is the other direction: analytic fields, evaluated per fragment, composed in the
shader.

```lua
ui.Sdf {
  width = 280, height = 240,
  fill_color = "#b4e1ea", stroke_color = "#0e1213", stroke_width = 3,
  ui.SdfShape { x = 40, y = 70, width = 90, height = 90, shape = "circle" },
  ui.SdfShape {
    x = 150, y = 70, width = 90, height = 90,
    shape = "circle",
    operation = "smooth_union",
    blend = function() return merged:get() and 55 or 0 end,
    behavior = { blend = { duration = 900, easing = "in_out_quad" } },
  },
}
```

A layer is an ordinary scene node. Its rectangle comes from layout, and every
number on it is an ordinary animatable property — so the motion above is one
behavior on one number, not a mechanism of its own. Four examples run it:

| Example | What it shows |
|---|---|
| `sdf-field.lua` | a morph, a seamless merge, and a wedge cut out of a ring |
| `sdf-gallery.lua` | every family with its own parameter moving |
| `sdf-metaballs.lua` | six fields orbiting in one composition, merging and splitting as they pass |
| `sdf-loaders.lua` | a progress arc, a spinner whose gap is cut out, fusing dots, a capsule fill |

Nine shape families: `circle`, `box`, `capsule`, `triangle`, `hexagon`, `star`,
`ring`, `pie`, `cross`. Six operations: `union`, `subtract`, `intersect`, and a
smooth variant of each whose seam is rounded over `blend` pixels.

A field absorbs every shape beneath it, descending through the positioners, so
an ordinary `Row` of `Rect`s laid out by the ordinary layout engine arrives as a
row of fields to fuse — nothing in the configuration is written for the field,
and the container decides whether its contents join. An absorbed rect keeps its
own colour and all four of its corner radii. Anything without a shape of its own
— text, an image — paints over the composition untouched.

**A compound is one animatable number.** A field's `blend`, `fill_color` and
`morph_progress` are handed down to every layer that does not name its own, so a
shape built from several layers — a disc with a ring subtracted and a bar fused
below it — morphs as one thing. A layer opts out by naming a value and rejoins
by removing it; `morph_progress` uses a negative default as the sentinel, since
zero is a real position. Keeping N layers in step from the configuration instead
is how a config acquires a frame runtime, which is the thing the engine exists
to avoid.

Text is deliberately not part of this. It has no distance field, so it is never
absorbed; it paints over the composition and follows the same signal with its
own opacity and size. `examples/sdf-compound.lua` ends on the case that needs:
a disc with a numeral in it collapsing to a bare pill, where the shape family,
the layer rectangle and the label all read one number.

**A composition is one surface but not one colour.** Every layer carries its own
`fill_color`, and the fills cross-fade with exactly the weight the distance
operator uses, so colour bleeds through a smooth seam precisely where the two
shapes are bulging into one another, and separates again as they part. A layer
that names no colour inherits the field's. Only the operators that *add* surface
bring a colour with them: subtracting or intersecting removes area, it does not
paint.

**Size a field to its composition, not to its surface.** The whole composition
is resolved per fragment, so the cost is the node's area times the layer count,
every frame. A fullscreen node with six layers on a 4K output asks for fifty
million shape evaluations a frame and will miss the deadline; the same six
layers in a node sized to what they actually reach costs a fraction of that. The
surface may be fullscreen and transparent — the field inside it should not be.

Two things here are not expressible by interpolating outlines, which is the
whole reason the pass exists:

- **A morph is an interpolation of the fields**, `mix(sd(a), sd(b), t)`, not of
  two outlines. It passes through shapes neither end describes, and it survives
  the outline splitting or merging on the way, because there is no
  correspondence between the ends to preserve. `shape`, `morph_to` and
  `morph_progress` are the whole interface.
- **A smooth operation has no seam.** Two surfaces bulge into each other over
  `blend` pixels the way two drops of liquid meet, and animating that radius
  moves between two separate pieces and one joined piece — a change of topology
  with nothing in between to interpolate.

A star is the one family whose parameter is not continuous — it is defined for
a whole number of points. Rounding `points` makes a new spike appear at full
size between one frame and the next, so `sd_star` blends the two neighbouring
stars as fields instead: at 5.5 the surface is halfway between a five and a
six-pointed star, a shape neither describes, and the new point grows out of the
edge. The intermediate covers *less* area than either end, which is why the
test measures how the change is distributed across a sweep rather than area.

The edge comes from the screen-space derivative of the composed distance, so it
is one pixel wide at any scale, rotation or skew; `softness` widens it
deliberately on top of that, and `stroke_width` centres an outline on the zero
crossing so growing it does not move the edge.

Layers live in a storage buffer rather than in vertex attributes — a composition
is far past the attribute limit — and every field in a frame shares one buffer,
each recording where its own run begins. A composition is capped at
`MAX_FIELD_LAYERS` (16): the whole composition is resolved per fragment, so
every layer costs every pixel of the node, and the cap is what keeps one node
from becoming an unbounded per-fragment loop.

## Frame cost

`cargo run --release -p mold-cli --example frame_bench -- <config.lua>` times the
three things every paint does on the CPU — layout, the draw list, the input
region — without needing a compositor. All three are pure functions of the
scene, so they measure exactly and repeatably. It reports the fastest of several
batches, because background load only ever adds time and an average would mostly
measure whatever else the machine was doing.

| | 134-node shell | | 1601-node scene | |
|---|---|---|---|---|
| | before | after | before | after |
| `Layout::compute` | 104µs | 53µs | 1.66ms | 0.94ms |
| `DrawList` | 45µs | 27µs | 4.59ms | 1.72ms |
| `input_geometry` | 23µs | 15µs | 913µs | 627µs |
| **frame CPU** | **171µs** | **95µs** | **7.17ms** | **3.28ms** |

The gain is larger on the bigger scene, which is the part that matters: the cost
was growing faster than the node count, and most of what follows is why.

**Nothing on this path hashes with SipHash any more.** Property names are one to
twenty characters and every read of every node in every frame hashes one; the
scene's own tables, layout's geometry and implicit-size maps, and the text
measurement cache are all keyed by something the engine controls. SipHash is
built to resist deliberate collisions, which a table built from a compile-time
schema has no exposure to, and it was costing about twenty nanoseconds a lookup
for the protection.

**`Scene::children` borrows instead of allocating.** It returned a fresh
`Vec<NodeHandle>` per call and layout asks five times per node, paint and hit
testing once each. `Node` stores handles rather than raw ids so children hand
out as a slice.

**Work that was repeated is done once.** The size a node asks for took five
property reads and a map parse, and every node was asked at least twice a pass —
three times inside a `RowLayout`; it is now resolved beside the implicit size.
`clip` was read twice per node and `rect_radii` — five reads itself — ran twice
for every rect.

**Work that was never needed is skipped.** A gradient read both its colours
before asking whether there was a gradient at all; a shadow read five numbers
and a flag before asking whether the colour was visible; a layer parsed a
property map that is empty on all but a handful of nodes. Each of those is the
common case in a real configuration, and each now costs one read.

**Anchors and attached layout are borrowed, not cloned** — both returned a
cloned `BTreeMap` of owned strings and values, per child, per pass.

The largest win is not in the table at all: **layout is skipped entirely on
frames that do not move geometry**, at 288ns to reuse against 53µs to recompute.
A colour easing, an opacity fading, a morph advancing — none of them move a box,
and most frames are one of those. `Scene::layout_revision` moves only when
something layout reads changes, and a paint that finds it unmoved, at the same
surface size and scale, reuses the layout it already has.

That revision deliberately does **not** use `PropertyClass`. That answers "what
work does this change need", and it calls `x` a transform because the renderer
offsets by it — but layout bakes `x` into the geometry it produces, and reads
`border_width` for a `ClipRect`'s content inset even though painting owns the
border. `affects_layout` is a separate, negative list: everything counts unless
it is known never to be read, so a property added to the schema without a
thought here costs one extra layout pass rather than a frame drawn at stale
geometry.

The input region is derived every paint but only **sent** when it differs from
the last one. It is double-buffered surface state, so an identical one costs a
region object, a round of protocol traffic and a commit, and changes nothing.

Idle, the whole shell — three outputs, one process — sits at 1–3% of a core.

## License and maintenance notes

- Animato is MIT OR Apache-2.0 and declares Rust 1.89.
- `signed-distance-field` is MIT, CPU-only, and marked passively maintained.

The workspace pins the reviewed versions exactly so these integration contracts
do not drift on an ordinary dependency resolution.
