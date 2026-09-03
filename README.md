# morf

morf is a Wayland rendering and shell engine implemented in Rust. It exposes
native scene, layout, rendering, input, surface, IO, and service primitives
through Rust and Lua APIs. Widgets and complete shells are downstream projects.

The `morf-lua` crate embeds [Luna](https://github.com/onix-os/luna) as a bounded
configuration and extension interface. Built-in engine modules are preloaded by
Rust; morf does not ship a Lua implementation tree.

```sh
oslo make build
oslo make run
oslo make test
oslo make verify
```

Run a configuration directly with:

```sh
cargo run --package morf-cli -- shell.lua
```

Run the interactive transformation example with:

```sh
EXAMPLE=examples/fluid-transform.lua oslo make run
```

Clicking its shape animates square-to-circle radius, color, origin-aware
non-uniform scale, skew, rotation, shadows, and spring translation. The
animation clock and interpolation remain in Rust; Lua only changes targets.

Run the combined animation, polygon morph, and signed-distance-field example:

```sh
EXAMPLE=examples/morph-stack.lua oslo make run
```

Animato advances the native tween and spring state — the timing — and signed
distance fields decide what a frame looks like — the view. Analytic fields are
composed and morphed in the fragment shader; reusable raster masks are converted
to cached distance fields. Morf still owns the compositor frame clock.

Use `--no-plugin` to load the configuration without auto-sourced plugins. Use
`--clean` to also exclude external Lua roots; modules beside the selected config
remain available.

An infinite Lua loop is terminated when its fuel budget is exhausted rather than
hanging the process.

## Writing UI

How nodes are sized and placed, how state reaches them, and what makes a
frame: [`docs/UI.md`](docs/UI.md). The examples under `examples/` are the
runnable versions of each section.
