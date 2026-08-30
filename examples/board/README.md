# Board example

This ports the layout of `~/.config/quickshell/board` onto mold's general native
core. It composes Rust-backed scene primitives from Lua and reads the optional
pywal JSON palette through `mold.io.file_view` and `mold.io.json`.

It intentionally contains no Hyprland, D-Bus, Bluetooth, MPRIS, notification,
or other shell-service implementation. Those integrations belong in consumer
plugins. The media card marks the downstream extension point rather than
pretending that mold owns a media widget.

Run it through the repository recipe:

    EXAMPLE=examples/board/init.lua oslo make run
