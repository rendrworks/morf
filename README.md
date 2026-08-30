# mold

mold is a Wayland rendering and shell engine implemented in Rust. It exposes
native scene, layout, rendering, input, surface, IO, and service primitives
through Rust and Lua APIs. Widgets and complete shells are downstream projects.
The implementation follows [PLAN.md](PLAN.md).

The `mold-lua` crate embeds [Luna](https://github.com/onix-os/luna) as a bounded
configuration and extension interface. Built-in engine modules are preloaded by
Rust; mold does not ship a Lua implementation tree.

```sh
oslo make build
oslo make run
oslo make test
oslo make verify
```

Run a configuration directly with:

```sh
cargo run --package mold-cli -- shell.lua
```

Run the interactive transformation example with:

```sh
EXAMPLE=examples/fluid-transform.lua oslo make run
```

Clicking its shape animates square-to-circle radius, color, origin-aware
non-uniform scale, skew, rotation, shadows, and spring translation. The
animation clock and interpolation remain in Rust; Lua only changes targets.

Use `--no-plugin` to load the configuration without auto-sourced plugins. Use
`--clean` to also exclude external Lua roots; modules beside the selected config
remain available.

An infinite Lua loop is terminated when its fuel budget is exhausted rather than
hanging the process.
