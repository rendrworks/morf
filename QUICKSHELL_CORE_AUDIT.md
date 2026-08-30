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

## Visual support boundary

| Quickshell support type | Native mold mechanism |
|---|---|
| ClippingRectangle | `mold.ui.ClipRect` |
| WrapperItem | `mold.ui.Inset` around `mold.ui.Item` |
| WrapperMouseArea | `mold.ui.Inset` around `mold.ui.MouseArea` |
| WrapperRectangle | `mold.ui.Inset` around `mold.ui.Rect` |
| ClippingWrapperRectangle | `mold.ui.Inset` around `mold.ui.ClipRect` |

Inset exposes the shared margin, side override, extra margin, implicit-size,
single-child, and child-resize controls. Rect, MouseArea, and ClipRect retain
their own primitive properties rather than combining them into mold-owned
widgets.

Quickshell's `IconImage` is a convenience composition rather than an engine
primitive. Mold deliberately leaves that composition to consumer Lua: Image
already provides source, aspect-fit, source-size, and implicit-size mechanisms.
Images are decoded for their exact physical target size and every load is
settled before the native frame uses it, so mipmap, asynchronous, and pending
status toggles have no distinct state in this synchronous renderer.

File views preserve preload, atomic-write, and watch policies across path
changes. Empty paths unload the document, non-empty paths rebind active
watchers, and every read or write returns only after its native operation has
settled. Quickshell's asynchronous blocking toggles therefore have no distinct
state in the synchronous mold interface.

Process output remains a raw bounded event stream, so parsers can be attached,
replaced, or removed without changing native process ownership. Delimiter
replacement reprocesses buffered bytes with the new marker. Client sockets
cover connect, disconnect, path mutation while disconnected, reconnect, flush,
and close; socket servers cover active, inactive, path mutation, rebind, accept,
and cleanup transitions.

Color quantizers transactionally recompute after source, depth, crop, or
rescale changes. Desktop models publish a fresh snapshot only when rescanning
detects a change. Menu state mutations are immediately reflected by entry and
child queries, including checkbox and radio activation rules.

Clipping rectangles expose independent `content_inside_border` and
`content_under_border` policies. The renderer uses a separate rounded inner
mask when content may not pass beneath the border, draws the border after its
children, and honors antialiasing and physical-pixel border alignment.

All included Quickshell core, IO, window, and visual support types now have an
audited native mapping. External runtime acceptance remains separate from this
source-surface audit.
