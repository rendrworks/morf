# Quickshell general-core parity

mold uses the general shell-building surface of the Quickshell checkout at:

    xtra/quickshell 2d3b3e9c70ef380dff751b61d334dc88df016c29

The target is Quickshell's reusable core, IO, window, and visual mechanisms.
mold exposes equivalent capabilities through native Rust-backed Lua modules. It
does not embed QML, copy Quickshell's implementation, or move the engine into
Lua.

The source-type mapping and remaining property lanes are tracked in
[`QUICKSHELL_CORE_AUDIT.md`](QUICKSHELL_CORE_AUDIT.md).

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

- signals, effects, hierarchically scoped reloadable IDs, typed persistent
  property scopes, timers, and clock updates;
- coalesced soft and hard reload requests routed through the native supervisor,
  with native surfaces and GPU targets recreated only for hard reloads,
  reloadable state retained only for soft reloads, and bounded completion and
  failure callbacks;
- native working-directory control and runtime file-watcher enablement;
- retainable scene objects and scoped locks for delayed destruction and exit
  transitions;
- reactive native local-time snapshots and bounded date/time formatting;
- bounded variants instantiate every model entry; screen variants expose logical geometry, physical metadata, density,
  orientation and transform data and restart on same-name metadata changes,
  plus list models with stable keyed
  reconciliation, value lookup, virtual lists, and flick state;
- process, instance, shell and application identity, launch time, environment
  lookup, shell paths, version checks, elapsed timers, and detached process
  launch, including native compile-feature checks;
- shell-scoped XDG data, state, and cache directories with bounded relative
  path resolution;
- native easing curves across quadratic, cubic, quartic, quintic, sine,
  exponential, circular, back, bounce, and cubic-Bezier families, with scalar,
  point, and rectangle interpolation;
- bounded native image color quantization with crop and rescale controls;
- bounded XDG icon-theme lookup and availability checks;
- bounded XDG desktop-entry discovery, precedence masking, change-detecting
  refresh, lookup, actions, and detached launching;
- bounded hierarchical menu models with separators, icons, nested children,
  checkbox and radio state, mutation, and protected activation handlers;
- native same-surface and cross-surface transform-chain watchers with bounded,
  deferred change handlers;
- bounded IPC registration and atomic runtime replacement.

### `mold.io`

- restartable streaming processes with process IDs, working directories,
  bounded mutable commands and environment contexts with atomic restart,
  stdin, signals, and exit status;
- bounded stateful file views with persistent preload and watch policies,
  explicit unload and path rebinding, reload, atomic writes, stable error
  categories, and watcher rebinding;
- native JSON encoding and decoding with preserved array, object, and null
  values, plus direct stateful file-view adapters;
- Unix sockets and socket servers;
- line and delimiter parsers;
- bounded byte stream collectors with live or end-of-stream publication.

### `mold.ui`

- item, rectangle, rounded clipping rectangle, text, image, icon, shape, and
  input primitives;
- rows, columns, grids, layouts, repeaters, virtualized views, flickables,
  synchronous and deferred lazy loaders, and timers;
- bounded native reparenting for dynamic wrappers and unwrapping;
- native single-child inset containers with side overrides, extra margins,
  implicit sizing, and optional child resizing;
- rounded clipping rectangles with border-aware content layout and border
  overlay painting;
- native properties, bindings, states, transitions, animation, focus, pointer,
  wheel and touchpad axes, touch, and keyboard routing.

### `mold.window`

- layer-surface namespace, size, anchors, margins, exclusive zone, compositor
  layer, output selection, and keyboard focus policy;
- popup anchor rectangles, edge and gravity selection, offsets, and compositor
  slide, flip, resize and explicit grab behavior, including item-derived
  rectangles with side margins, plus bounded live mutation of size, anchor
  geometry, offsets, placement, constraints, grab policy, and parent;
- typed dynamic popup and undecorated floating-surface models with validated
  roots, mutable bounded size constraints, identity, title, visibility, initial
  and mutable minimized, maximized and fullscreen state, placement state,
  compositor-owned interactive move and resize requests, compositor lifecycle,
  native transient-parent relationships with inherited visibility, popups
  anchored to layer or floating parents, multiple independent GPU-rendered
  instances, and surface-scoped pointer, touch, and keyboard routing;
- native item-position, item-rectangle, point, and rectangle mapping from
  window-local scene nodes through ancestor rotation and scale chains;
- per-window render-update suspension with one catch-up frame when resumed;
- composable rectangular, rounded, and elliptical input masks with combine,
  subtract, intersect, and XOR operations;
- fractional scaling, output tracking, frame callbacks, and input regions.

The Rust engine already contains additional platform mechanisms. They do not
count toward general-core parity unless the included API requires them.
