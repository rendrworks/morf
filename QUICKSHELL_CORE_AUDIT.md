# Quickshell general-core audit

Reference: `xtra/quickshell` at `2d3b3e9c70ef380dff751b61d334dc88df016c29`.

This audit covers reusable core, IO, and window APIs. It excludes compositor
extensions, D-Bus, DBusMenu, Bluetooth, networking, media, power,
notifications, PipeWire, authentication, tray, and X11 APIs.

## Core types

| Quickshell type | Native mold equivalent |
|---|---|
| ShellRoot, Scope, Reloadable, Singleton | `mold.core.scope`, reload supervisor, scoped IDs |
| PersistentProperties | `mold.core.persistent` |
| Retainable, RetainableLock | `mold.core.retainable`, `retain_lock` |
| Variants | bounded `mold.core.variants` instances |
| LazyLoader, BoundComponent | native sync/deferred loaders and bindings |
| ObjectModel, ScriptModel | keyed list models, repeaters, virtual and sync views |
| SystemClock, ElapsedTimer | `system_clock`, `clock`, `elapsed_timer`, timers |
| EasingCurve | native easing families and geometry interpolation |
| ShellScreen | native hotplug screen model and metadata |
| DesktopEntry, DesktopAction, DesktopEntries | native XDG desktop-entry scanner and launcher |
| ColorQuantizer | bounded native image quantizer |
| QsMenuHandle, QsMenuEntry, QsMenuOpener | native hierarchical menu model and activation |
| PopupAnchor | typed anchor rectangles, item geometry, margins and constraints |
| Region | native composable rectangular, rounded and elliptical regions |
| TransformWatcher | native cross-tree transform watcher |
| Quickshell global and settings | `mold.core` identity, paths, environment, reload and watcher API |

Qt/QML engine support objects, scanners, image providers, logging adapters,
incubators, and proxy objects are implementation details rather than public
shell mechanisms.

## IO types

| Quickshell type | Native mold equivalent |
|---|---|
| Process | restartable process and process-view handles |
| DataStream, SplitParser, StdioCollector | bounded stream, line, split and collector handles |
| FileView, FileViewAdapter, JsonAdapter | synchronous stateful file view and native JSON adapters |
| Socket, SocketServer | bounded Unix stream and server handles |
| IpcHandler | named bounded IPC registry and persistent local server |

## Window types

| Quickshell type | Native mold equivalent |
|---|---|
| QsWindow | common native surface state, rendering and scoped input |
| PanelWindow | typed layer-surface configuration |
| FloatingWindow | native xdg toplevel with state, constraints and system operations |
| PopupWindow | native anchored xdg popup with constraints and grab behavior |
| Anchors and surface masks | typed anchors, margins and composable regions |

Floating and popup handles support multiple independent instances. Native
transient parents are created before their children, parent recreation rebuilds
the dependent surface, and a hidden or missing parent suppresses its child.

## Remaining audit lanes

- verify process parser replacement and every socket connection transition;
- verify reactive mutation behavior for quantizers, desktop models, and menus;
- verify every visual primitive property against the bundled Quickshell visual
  support types.

File views preserve preload, atomic-write, and watch policies across path
changes. Empty paths unload the document, non-empty paths rebind active
watchers, and every read or write returns only after its native operation has
settled. Quickshell's asynchronous blocking toggles therefore have no distinct
state in the synchronous mold interface.
