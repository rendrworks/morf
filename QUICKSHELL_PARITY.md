# Quickshell capability parity

mold targets the public capability surface of the Quickshell checkout at:

    xtra/quickshell 2d3b3e9c70ef380dff751b61d334dc88df016c29

The reference is Quickshell's native implementation and QML API declarations.
mold exposes equivalent capabilities through native Rust-backed Lua modules. It
does not embed QML, copy Quickshell's implementation, or move the engine into
Lua.

## Completion rule

A module is complete only when every public type, property, enum, method, and
signal in its reference module has:

1. an equivalent native mold mechanism;
2. a typed Lua API with bounded inputs and protected handlers;
3. lifecycle, mutation, failure, and reload tests;
4. live integration evidence for protocols or services that require it;
5. documentation and at least one consumer example.

Names may follow mold's Lua conventions instead of QML syntax, but omissions and
behavioral reductions do not count as parity.

## Module ledger

| Quickshell module | mold Lua module | status |
|---|---|---|
| `Quickshell` | `mold.core` | partial |
| `Quickshell.Io` | `mold.io` | partial |
| `Quickshell.DBusMenu` | `mold.dbusmenu` | missing |
| `Quickshell.Widgets` | `mold.ui` | partial |
| window interfaces | `mold.window` | partial |
| `Quickshell.Wayland` | `mold.wayland` | partial |
| `Quickshell.Hyprland` | `mold.hyprland` | missing |
| `Quickshell.Services.SystemTray` | `mold.services.status_notifier` | partial |
| `Quickshell.Services.Pipewire` | `mold.services.pipewire` | partial |
| `Quickshell.Services.Mpris` | `mold.services.mpris` | missing |
| `Quickshell.Services.Pam` | `mold.services.pam` | partial |
| `Quickshell.Services.Greetd` | `mold.services.greetd` | partial |
| `Quickshell.Services.Polkit` | `mold.services.polkit` | missing |
| `Quickshell.Services.UPower` | `mold.services.upower` | missing |
| `Quickshell.Services.Notifications` | `mold.services.notifications` | missing |
| `Quickshell.Bluetooth` | `mold.bluetooth` | missing |
| `Quickshell.Networking` | `mold.network` | missing |
| `Quickshell.WindowManager` | `mold.window_manager` | missing |
| `Quickshell.X11` | `mold.x11` | missing |
| `Quickshell.I3` | `mold.i3` | missing |

## Implemented native surface

### `mold.core`

- signals, effects, reloadable state, timers, and clock updates;
- screen variants, list models, virtual lists, and flick state;
- process identity, environment lookup, shell paths, version checks, elapsed
  timers, and detached process launch;
- bounded IPC registration and atomic runtime replacement.

### `mold.io`

- streaming processes;
- watched files;
- Unix sockets and socket servers;
- line and delimiter parsers;
- typed D-Bus calls, introspection, and signals.

### `mold.ui`

- item, rectangle, text, image, icon, shape, and input primitives;
- rows, columns, grids, layouts, repeaters, virtualized views, flickables,
  loaders, and timers;
- native properties, bindings, states, transitions, animation, focus, pointer,
  touch, and keyboard routing.

### `mold.window` and `mold.wayland`

- layer-surface namespace, size, anchors, margins, exclusive zone, compositor
  layer, output selection, and keyboard focus policy;
- fractional scaling, output tracking, frame callbacks, and input regions;
- idle notification, output power, clipboard, screencopy, virtual keyboard,
  input method, and text input.

The Rust Wayland engine also has popup, floating-surface, and session-lock
mechanisms. Their complete Lua object APIs remain part of the partial window row.

### Native services

- low-level PipeWire graph operations;
- PAM and greetd authentication mechanisms;
- status-notifier discovery;
- udev monitoring and XKB keymap compilation.

The partial rows remain partial until their complete reference declarations and
runtime behavior satisfy the completion rule above.
