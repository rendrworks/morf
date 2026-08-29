# Quickshell general-core parity

mold uses the general shell-building surface of the Quickshell checkout at:

    xtra/quickshell 2d3b3e9c70ef380dff751b61d334dc88df016c29

The target is Quickshell's reusable core, IO, window, and visual mechanisms.
mold exposes equivalent capabilities through native Rust-backed Lua modules. It
does not embed QML, copy Quickshell's implementation, or move the engine into
Lua.

## Scope

Included:

- lifecycle, reload, scopes, persistent properties, variants, and lazy loading;
- screens, models, clocks, elapsed timers, easing, paths, and environment data;
- desktop entries, menus, popup anchors, regions, and transform watching;
- processes, files, JSON, sockets, streams, parsers, and local IPC;
- panel, floating, and popup window mechanisms;
- general scene, image, icon, clipping, wrapper, layout, and input primitives.

Excluded from the parity target:

- Hyprland, i3, and compositor-specific extension APIs;
- D-Bus and DBusMenu parity;
- Bluetooth and NetworkManager parity;
- MPRIS, notifications, UPower, PipeWire, PAM, greetd, polkit, and system-tray
  parity;
- X11-specific APIs;
- protocol-specific Wayland extras that are not required by the general window
  API.

Existing low-level mold integrations may remain available, but they are not part
of this Quickshell parity effort.

## Completion rule

A core area is complete only when every included public type, property, enum,
method, and signal has:

1. an equivalent native mold mechanism;
2. a typed Lua API with bounded inputs and protected handlers;
3. lifecycle, mutation, failure, and reload tests;
4. live integration evidence where external state is required;
5. documentation and at least one consumer example.

Names may follow mold's Lua conventions instead of QML syntax, but omissions and
behavioral reductions do not count as parity.

## Core ledger

| Quickshell area | mold Lua module | status |
|---|---|---|
| reusable core | `mold.core` | partial |
| general IO | `mold.io` | partial |
| visual primitives | `mold.ui` | partial |
| general windows | `mold.window` | partial |

## Implemented native surface

### `mold.core`

- signals, effects, reloadable state, timers, and clock updates;
- screen variants, list models, virtual lists, and flick state;
- process identity, environment lookup, shell paths, version checks, elapsed
  timers, and detached process launch;
- bounded XDG desktop-entry discovery, precedence masking, lookup, actions, and
  detached launching;
- bounded IPC registration and atomic runtime replacement.

### `mold.io`

- restartable streaming processes with process IDs, working directories,
  bounded environment overrides, stdin, signals, and exit status;
- bounded stateful file views with preload, reload, atomic writes, stable error
  categories, and change watching;
- native JSON encoding and decoding with preserved array, object, and null
  values;
- Unix sockets and socket servers;
- line and delimiter parsers.

### `mold.ui`

- item, rectangle, text, image, icon, shape, and input primitives;
- rows, columns, grids, layouts, repeaters, virtualized views, flickables,
  loaders, and timers;
- native properties, bindings, states, transitions, animation, focus, pointer,
  touch, and keyboard routing.

### `mold.window`

- layer-surface namespace, size, anchors, margins, exclusive zone, compositor
  layer, output selection, and keyboard focus policy;
- fractional scaling, output tracking, frame callbacks, and input regions.

The Rust engine already contains additional platform mechanisms. They do not
count toward general-core parity unless the included API requires them.
