# Board example

This reproduces the general visual surface of `~/.config/quickshell/board` with
Rust-backed mold primitives configured from Lua. It uses the same screen-scaled
geometry, pywal colors, borders, font family and weights, empty media state,
calendar layout, battery and progress easing, and blinking clock colon. The two
bundled IosevkaTerm Nerd Font Mono faces make the result independent of system
font installation; their license is stored beside them.

The font files come from Nerd Fonts 3.4.0's `IosevkaTerm` package.

It intentionally contains no Hyprland, D-Bus, Bluetooth, MPRIS, notification,
or other shell-service implementation. Those integrations belong in consumer
plugins. The example renders the board through general primitives without
adding widgets to mold.

Run it through the repository recipe:

    EXAMPLE=examples/board/init.lua oslo make run
