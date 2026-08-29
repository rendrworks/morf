# mold

mold is a Wayland shell runtime with a reactive scene graph configured in Lua and
rendered on the GPU. The implementation follows [PLAN.md](PLAN.md).

The repository is an incremental workspace. The first runnable milestone embeds
[Luna](https://github.com/onix-os/luna) behind `mold-lua` and executes bounded Lua
configuration through the `mold` CLI.

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

An infinite Lua loop is terminated when its fuel budget is exhausted rather than
hanging the process.
