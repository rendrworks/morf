# Quickshell whole-source audit

Reference: `xtra/quickshell` at
`2d3b3e9c70ef380dff751b61d334dc88df016c29`.

This is a source-surface audit of the entire pinned checkout, not a claim that
Mold must copy every Quickshell feature. It separates reusable engine
primitives from downstream widgets, optional desktop integrations, and
platform-specific APIs. Mold remains a Rust engine with Lua as its
configuration interface. Nothing in this audit authorizes Mold-owned widgets
or a Lua implementation tree.

## Method

The audit used:

- every non-test header and QML file below `xtra/quickshell/src`;
- every top-level feature switch in `xtra/quickshell/CMakeLists.txt:56-83`;
- every module and protocol registration in the nested `CMakeLists.txt` files;
- the exact declarations found by `oslo make quickshell-inventory`;
- direct comparison with the Rust implementation and Rust-owned Lua bindings;
- behavioral implementation files where declarations alone were insufficient.

The inventory now scans the whole source tree instead of only `core`, `io`,
`widgets`, `window`, and `windowmanager`. At the pinned revision it emits 2,012
lexical declarations across Bluetooth, core, D-Bus, IO, IPC, network,
services, UI, Wayland, widgets, windows, window management, and X11. Those
records are candidates for manual review; internal QObject signals in a public
header are not automatically treated as public parity requirements.

### Status meanings

| Status | Meaning |
|---|---|
| implemented | The relevant capability and lifecycle are present and evidenced. |
| partial | A useful native mechanism exists, but public behavior is reduced. |
| missing-general-core | A reusable engine or protocol primitive is absent. |
| downstream-widget | A visual/control composition must remain outside Mold. |
| optional-desktop | A non-widget desktop integration is absent or reduced but is not required for the renderer core. |
| platform-specific | An X11, i3, Hyprland, or vendor protocol may remain excluded. |
| internal | A Qt/QML/C++ implementation detail is not a Mold API target. |
| exceeds-reference | Mold exposes a native mechanism not present as a pinned Quickshell public module. |

## Whole-checkout coverage

| Upstream source area | Classification | Result |
|---|---|---|
| `core` | general engine | mixed: implemented, partial, and missing mechanisms |
| `io` | general engine | partial; JSON codec is strong, async and ownership semantics are not |
| `widgets` | convenience compositions over QtQuick | downstream-widget; enabling primitives audited separately |
| `window` | general surface abstraction | partial |
| `windowmanager` | general shell data/control abstraction | missing-general-core |
| `wayland` | general and vendor protocols | mixed; core surfaces exist, several general protocols are absent |
| `services` | low-level desktop integrations | mixed, mostly optional-desktop and reduced |
| `network` | NetworkManager integration | optional-desktop, absent |
| `bluetooth` | BlueZ integration | optional-desktop, absent |
| `dbus` | internal D-Bus support and public DBusMenu | generic Mold client exceeds the internal helper; DBusMenu absent |
| `x11` | X11 and i3/Sway integration | platform-specific, absent by design |
| `ui` | Quickshell-owned reload popup and tooltip | downstream-widget/internal, excluded |
| `ipc`, `launch` | command and IPC implementation | IPC behavior audited against `IpcHandler`; launch parsing is internal |
| `crash`, `debug`, `build` | process diagnostics and build machinery | internal, not public shell primitives |

Quickshell enables Wayland, layer shell, session lock, foreign toplevel,
screencopy, X11/i3, every listed service, Bluetooth, and network by default at
`xtra/quickshell/CMakeLists.txt:56-83`. Therefore the previous narrow audit
could not support a whole-Quickshell claim.

## Runtime topology and lifecycle

| Surface | Upstream evidence | Mold evidence | Status and gap |
|---|---|---|---|
| Shell-global engine | One generation owns the shell at `core/generation.cpp:37-60`; screens are one reactive list at `core/qmlglobal.hpp:103-123`. | A complete Runtime executes shell and plugins once per output at `crates/mold-cli/src/main.rs:613-660,743-781`; each runtime publishes one screen at `crates/mold-lua/src/lib.rs:4400-4445`; IPC dispatch selects the first worker at `mold-cli/src/main.rs:791-827`. | **missing-general-core.** Config, process, socket, plugin, persistence, and IPC state can be duplicated or fragmented per output. Mold needs one shell-global native ownership domain with per-output surface instances. |
| Reactive graph | QML follows `Q_PROPERTY`/`NOTIFY` properties, including live animation values. | Signals, effects, dependency capture, staged writes, depth ordering, cycle handling, and fuel bounds exist at `mold-reactive/src/lib.rs:227-403` and `mold-lua/src/lib.rs:2680-2725,10388-10510`. | **partial.** Only Mold signals and instrumented reads react; animated rendered values explicitly do not notify Lua at `mold-lua/src/lib.rs:704-736`. |
| Effect ownership | QML object destruction owns its bindings. | Graph removal exists at `mold-reactive/src/lib.rs:240-251`, but bindings registered at `mold-lua/src/lib.rs:10167-10193` are not removed by subtree destruction at `mold-lua/src/lib.rs:1751-1785`. | **missing-general-core.** Loader/model churn can retain closures and effects targeting stale nodes. |
| Candidate reload | Quickshell constructs a generation, transfers state, swaps, then fires post-reload at `core/generation.cpp:132-168`. Process and socket-server activation is deferred at `io/process.cpp:180-217` and `io/socket.cpp:111-118`. | Candidate construction and swap are at `mold-cli/src/main.rs:902-935`, but `process_view` can spawn and `socket_server` can bind during candidate evaluation at `mold-lua/src/lib.rs:5167-5195,5653-5669`. | **missing-general-core.** Reload needs prepare, validate, commit/activate, and rollback phases so rejected candidates have no external side effects. |
| File dependency reload | Imported and explicit dependencies are content-hashed with truncate/unchanged guards at `core/generation.cpp:170-255` and `core/scan.cpp:25-41`. | Every Lua file under every runtime root is metadata-polled at `mold-cli/src/main.rs:454-475,692-722`. | **partial.** Unused files trigger reload; same-content metadata changes trigger; same-size/same-mtime changes may be missed. |
| Persistent properties | All declared compatible properties transfer by name at `core/persistentprops.cpp:6-23`. | Named scopes and loaded/reloaded callbacks exist at `mold-lua/src/lib.rs:2778-2927`. Values are restricted to scalar primitives at `mold-lua/src/lib.rs:1872-1893`. | **partial.** Lists, maps, and structured state are absent, and scopes are prefixes rather than recursive ownership domains. |
| Reloadable, Scope, Singleton | Recursive instance matching and singleton transfer occur at `core/reload.cpp:57-126` and `core/singleton.cpp:40-52`. | Soft reload seeds explicitly marked values; both soft and hard replace the Lua/scene runtime at `mold-cli/src/main.rs:908-935`. | **partial.** This is state-seeded replacement, not complete object-tree Reloadable/Singleton parity. |
| Retainable | Dropped handlers may acquire locks; final release emits destruction state at `core/retainable.cpp:31-86`. | Native lock/drop gates exist at `mold-lifecycle/src/lib.rs:48-105`; Lua exposes lock state and lifecycle callbacks at `mold-lua/src/lib.rs:2929-3064`. | **partial/close.** Core delayed destruction works, but attachment is scene-node-only and signal/state shape differs. |
| Last-window lifecycle | `lastWindowClosed` is public at `core/qmlglobal.hpp:45-58,292-305`. | No equivalent lifecycle signal or quit-on-last-closed policy exists. | **missing-general-core.** |

## Core objects, models, components, and time

| Surface | Upstream evidence | Mold evidence | Status and gap |
|---|---|---|---|
| Signals and explicit effects | Public QML bindings and notifiers throughout core. | `mold.signal`, effects, scene bindings, staged writes, and protected handlers are native. | **implemented capability**, with the ownership and animation-notify gaps above. |
| `Variants` | Live model updates, duplicate suppression, stable matching, removal, and reload overlap mapping at `core/variants.cpp:16-178`. | `core.variants` is a one-shot dense-table factory at `mold-lua/src/lib.rs:4446-4492`. | **missing-general-core.** The existing implemented claim was false. |
| `ObjectModel` | Values, index lookup, and pre/post insertion/removal signals at `core/model.hpp:40-77`. | Stable IDs, insert/remove/move/update, journal, and structural reconciliation at `mold-scene/src/model.rs:18-177`; Lua mutation at `mold-lua/src/lib.rs:4494-4620`. | **partial.** No public change subscriptions or pre/post signals. |
| `ScriptModel` | Live values, persistent comparison mode, object property, reentrant staged updates, and unique values at `core/scriptmodel.hpp:20-140` and `core/scriptmodel.cpp:66-253`. | Structural or one-call object-property reconciliation exists. | **partial.** No identity mode, persistent comparison property, duplicate rule, or consumer-visible granular signals. |
| `LazyLoader` | URL/component sources, true incubation, item/loading state, cancellation, error, and reload propagation at `core/lazyloader.hpp:85-168` and `core/lazyloader.cpp:15-200`. | A Lua closure is synchronously or later executed at `mold-lua/src/lib.rs:8435-8477,1519-1555`. | **partial.** Deferred execution is not asynchronous incubation; source/component, status/error, cancellation, and transfer are absent. |
| `BoundComponent` | Component/source selection, `bindValues`, property forwarding, implicit sizing, loaded/error behavior at `core/boundcomponent.hpp:47-103` and `core/boundcomponent.cpp:114-229`. | No equivalent native component/factory object. | **missing-general-core.** |
| Component/source registry | Quickshell scans imported directories, synthesizes module metadata, supports singleton/internal declarations, and confines URLs at `core/scan.cpp:44-280` and `core/qsintercept.cpp:22-69`. | Mold `require` has ordered runtime roots, cache, cycle sentinel, and safe name forms at `mold-lua/src/lib.rs:7063-7140`. | **partial.** Lua modules work, but there is no native reusable component/factory registry or Loader source resolver. This must not become a built-in Lua runtime tree. |
| `SystemClock` | Precision truncates fields and schedules the chosen boundary at `core/clock.cpp:46-87`; fields are public at `core/clock.hpp:35-76`. | Precision is parsed but ignored by snapshot/format at `mold-lua/src/lib.rs:3566-3618`; the CLI refreshes every second at `mold-cli/src/main.rs:1650-1655`. | **partial with bug.** Hour/minute precision and reactive field behavior are not equivalent. |
| `ElapsedTimer` | Six methods at `core/elapsedtimer.hpp:12-41`. | Matching native methods at `mold-lua/src/lib.rs:3503-3565`. | **implemented.** |
| `EasingCurve` | Curve evaluation and scalar/point/rectangle interpolation at `core/easingcurve.hpp:12-35`. | Native easing families and geometry interpolation are exposed. | **implemented capability.** |
| Desktop entries | Collection excludes both Hidden and NoDisplay at `core/desktopentry.hpp:287-318` and changes automatically. | Metadata/actions/lookup/launch are native at `mold-desktop/src/lib.rs:13-162`; refresh is explicit. | **partial.** This audit fixed the incorrect inclusion of `NoDisplay` entries; automatic monitoring remains absent. |
| Menus | Menu entries plus platform display/open/close/anchor behavior at `core/qsmenu.hpp:43-115` and `core/qsmenuanchor.hpp:35-71`. | Native in-memory hierarchy, mutation, radio/checkbox rules at `mold-menu/src/lib.rs:28-195`. | **partial.** Data is present; platform menu display/anchor lifecycle and DBusMenu are absent. |
| Region | Rectangle, ellipse, rounded corners, nesting, and boolean operations at `core/region.hpp`. | Every per-corner radius and combine/subtract/intersect/XOR is exposed at `mold-lua/src/lib.rs:9865-9972`. | **implemented capability.** |
| Transform watcher | Cross-tree endpoints/common parent/transform at `core/transformwatcher.hpp:21-54`. | Native transform-chain signatures and watcher at `mold-layout/src/lib.rs:210-233,770-827`. | **implemented core.** |
| Screen metadata | Public geometry, physical/logical density, orientation, DPR, serial, and changes at `core/qmlscreen.hpp:23-76`. | Native screen tables expose geometry, densities, orientation, primary orientation, DPR, and a nil serial at `mold-lua/src/lib.rs:4407-4440`. | **partial/close.** Backend serial data and exact signal/toString surface differ; the larger shell-global screen-list gap remains. |
| Color quantizer | Mutable source/depth/crop/rescale and automatic results at `core/colorquantizer.hpp:66-119`. | Bounded native quantization at `mold-image/src/lib.rs:34-85`, exposed at `mold-lua/src/lib.rs:3748-3849`. | **partial/close.** Capability is present; result publication is explicit and synchronous. |

Qt engine scanning, QML preprocessing, Qt-version queries, and the C++
`QsEnginePlugin` ABI are **internal**, not Mold parity targets. Mold's ordered
Lua config/plugin fragments are a valid separate interface, but are not the
same ABI and must not be described as such.

## IO, JSON, sockets, and IPC

| Surface | Upstream evidence | Mold evidence | Status and gap |
|---|---|---|---|
| Process | Running/pid/config, parser ownership, stdin policy, started/exited with crash status, exec/signal/write/detached launch at `io/process.hpp:34-245`. | Mutable process handles, pid, stdin, signals, output polling, and global detached execution at `mold-lua/src/lib.rs:4908-5195`; native readers at `mold-io/src/lib.rs:85-180`. | **partial.** Parser attachment, started callback, crash classification, stdin policy, and handle-local exec/detach are absent. Config mutation immediately replaces a running process rather than changing the next start. |
| Process queue | Native QProcess signal delivery. | Output readers use unbounded channels at `mold-io/src/lib.rs:94-96,169-180`. | **missing-general-core safety.** The previous “bounded event stream” claim was false. |
| Streams/parsers | `DataStream` owns and replaces parsers; read/data/finish signals at `io/datastream.hpp:15-124`. | Split reprocessing and collector wait-for-end exist at `mold-io/src/lib.rs:216-369`. | **partial.** Manual pull/push composition lacks declarative parser ownership and signal lifecycle. |
| File view | Sync/async load/save, cancellation, block policies, atomic/watch, typed errors, adapters, and signals at `io/fileview.hpp:164-363` and `io/fileview.cpp:300-384`. | Synchronous preload/reload/read/write/atomic/watch/rebind at `mold-lua/src/lib.rs:5265-5460` and `mold-io/src/lib.rs:481-610`. | **partial.** No native async job, cancel/wait, completion/failure callbacks, block policies, or adapter lifecycle. The old audit incorrectly declared async state irrelevant. |
| File watcher queue | Push changes through Qt watcher state. | An unbounded channel is used at `mold-io/src/lib.rs:625-688`. | **missing-general-core safety.** A bounded/coalescing queue and diagnostics are required. |
| JSON codec | JSON values and explicit null. | Bounded codec preserves objects, arrays, and null at `mold-lua/src/lib.rs:6284-6419`. | **implemented codec.** |
| JSON adapter | Typed reactive property tree, defaults, dirty notification, recursive load/save, and FileView lifecycle at `io/jsonadapter.hpp:19-115` and `io/jsonadapter.cpp:25-160`. | Read/write returns an arbitrary Lua table at call time. | **missing-general-core.** A codec is not a reactive adapter. |
| Client socket | Nonblocking target state, errors/signals, parser delivery at `io/socket.hpp:20-76`. | Connect/disconnect/path/send/flush/receive at `mold-lua/src/lib.rs:5462-5570`; receive can block up to five seconds. | **partial.** No state/error callbacks, parser ownership, or nonblocking delivery contract. |
| Socket server | Handler component per client, owned connection teardown, activation deferral at `io/socket.hpp:95-152` and `io/socket.cpp:109-230`. | Manual `accept()` polling at `mold-lua/src/lib.rs:5572-5669`. | **partial/missing ownership.** No handler factory, accepted-client ownership, bulk teardown, or reload activation barrier. |
| IPC handler | Mutable target/enabled, typed functions, readable properties, signals, metadata, introspection, get/show/wait/listen at `io/ipchandler.hpp:123-255` and `io/ipccomm.cpp:33-411`. | Peer-UID-checked bounded local server and name-to-function calls at `mold-io/src/lib.rs:864-1011`, `mold-lua/src/lib.rs:4332-4360`, and `mold-cli/src/main.rs:112-123`. | **partial.** No hierarchy, schemas, properties, remotely streamed signals, metadata, target mutation, or listen/wait. |
| Generic D-Bus client | Quickshell's `src/dbus` helper is primarily internal; DBusMenu is public separately. | Bounded system/session proxy, properties, calls, introspection, signals, and typed values at `mold-io/src/lib.rs:1230-1650`. | **exceeds-reference** as a generic client; server export/name ownership and FD crossing remain absent. |

## Scene, layout, rendering, input, and animation

Quickshell itself links QtQuick and exposes its primitive substrate to every
configuration. Mold need not copy Qt class names, but the project plan claims
equivalent native primitives. The Quickshell.Widgets convenience compositions
remain downstream.

| Surface | Upstream use/evidence | Mold evidence | Status and gap |
|---|---|---|---|
| Item tree | Widgets use arbitrary QQuickItem children and reparenting at `widgets/wrapper.hpp:84-126`. | Native nodes/common properties at `mold-scene/src/lib.rs:25-64,1297-1318`; bounded reparenting at `mold-lua/src/lib.rs:6250-6265`. | **partial.** Basic geometry/tree exists; transform lists/origins, baseline/resources/focus scopes and broader mapping/polish semantics do not. |
| Transforms | QtQuick items accept transform origin, nonuniform scale, and transform lists. | Scalar rotation and uniform scale, always around center, at `mold-scene/src/lib.rs:1311-1313` and `mold-layout/src/lib.rs:859-871`. | **partial.** Missing transform origin, scale X/Y, arbitrary matrix/translate/rotate/scale/shear lists. |
| Anchors | Quickshell QML uses full anchors at `widgets/ClippingRectangle.qml:55-74` and `widgets/IconImage.qml:59-65`. | Parent fill/center/edges and margins at `mold-layout/src/lib.rs:995-1058`. | **partial.** Missing sibling targets, horizontal/vertical center, baseline and offsets, and complete dynamic semantics. |
| Rectangles and clipping | Per-corner radius, border, antialiasing, content-under/inside-border at `widgets/ClippingRectangle.qml:15-53`. | Rect/ClipRect schema at `mold-scene/src/lib.rs:1357-1392`; inner mask and overlay border at `mold-layout/src/lib.rs:524-535` and `mold-render/src/lib.rs:806-840`. | **implemented native subset.** This is a valid primitive mapping. |
| Gradients | QtQuick Rectangle accepts arbitrary gradient stops. | Two endpoints for linear/radial/conical gradients at `mold-scene/src/lib.rs:1360-1370`. | **partial.** Stop lists, positions, spread/interpolation and reactive gradient objects are absent. |
| Text | Configurations receive QtQuick Text. | Plain shaped text with family/source/weight, wrap/elide/alignment at `mold-scene/src/lib.rs:1395-1407`. | **partial.** Line count/height, detailed wrap, italic/stretch/capitalization/spacing, rich text/links/selection, metrics/baselines, render controls and truncation state are absent. |
| Image | `IconImage` exposes source, asynchronous, status, mipmap, backer and exact sizing at `widgets/IconImage.qml:26-66`. | Synchronous size-keyed decode and stretch/fit/crop at `mold-image/src/lib.rs:220-251` and `mold-scene/src/lib.rs:1409-1415`. | **partial.** Observable ready/error state, reload/invalidation, mirror/auto-transform/source crop and animated images are absent. Synchronous loading does not make status/error meaningless. |
| Icon | Quickshell has icon-provider infrastructure; `IconImage` consumes it. | Native XDG lookup/cache at `mold-image/src/lib.rs:254-285`. | **implemented XDG lookup**, with the Image state gaps above. |
| Shapes | QtQuick Shapes is available to configurations. | One SVG path string with fill/stroke/width/rule at `mold-scene/src/lib.rs:1426-1433`. | **partial.** No reactive ShapePath element tree, arcs/segments, caps/joins/miter/dash, gradient fills/strokes, renderer choice, or status. |
| Layers/effects | Quickshell's clipping widget uses `ShaderEffectSource` and `ShaderEffect` at `widgets/ClippingRectangle.qml:77-91`. | Native offscreen opacity, blur, shadow, rounded mask at `mold-render/src/lib.rs:650-695`. | **partial.** No public source capture, custom shader, effect graph, sampler controls, color matrix, saturation/brightness/contrast, or generic mask controls. |
| Row/Column/Grid | QtQuick positioners are part of the supplied substrate. | Native sequential placement/spacing at `mold-layout/src/lib.rs:537-566,639-660,943-973`. | **partial.** Direction, alignment, padding, rows/flow and positioner transitions/signals are incomplete. |
| Layouts | Quickshell wrapper docs direct multi-child users to QtQuick.Layouts at `widgets/WrapperItem.qml:3-9,31`. | Row/Column/GridLayout with size/fill constraints at `mold-layout/src/lib.rs:501-597`. | **partial.** Alignment/margins, attached row/column/span, stretch, uniform cells and direction are absent. |
| Flickable and views | ScriptModel promises view identity/animation behavior at `core/scriptmodel.hpp:30-58`. | Fixed-extent virtualizer and one-axis physics at `mold-scene/src/model.rs:205-433`. | **partial.** No full drag/flick state, bounds/margins/directions/signals, variable extents, sections/header/footer/current/highlight/snap/range or view transition execution. |
| Repeater | QML Repeater retains delegates with ScriptModel. | Native ListModel delegate construction at `mold-lua/src/lib.rs:8543-8595`. | **partial.** No count/itemAt, roles, delegate lifecycle signals or broad input models. |
| Pointer/touch/wheel | QQuick MouseArea and pointer handlers are available. | Basic hover/move/press/release/click/drag, touch identity and pixel/step wheel axes at `mold-lua/src/lib.rs:508-561,1064-1163`. | **partial.** Double/hold/cancel/composed propagation, cursor state, complete drag constraints, gesture arbitration, pointer handlers and device metadata are absent. |
| Keyboard/focus | QQuick Item/Keys/FocusScope exposes press/release/repeat/modifiers/navigation and focus state. | Wayland emits press/release/repeat and modifiers at `mold-wayland/src/lib.rs:547-562`, but Lua UI handlers expose only key press/keysym/text at `mold-lua/src/lib.rs:531-560,1047-1062`; focus is one boolean/tree cycle. | **partial.** Release, repeat, modifiers, native codes, propagation, shortcuts, navigation, active/requested focus, reasons and focus scopes are absent from the primitive API. |
| States/transitions | QML has conditions, extension, parent/anchor changes, selector and group semantics. | Name-keyed state tables and numeric/color interpolation at `mold-lua/src/lib.rs:9111-9170` and `mold-scene/src/lib.rs:1092-1197`. | **partial.** No state extension/priority, parent/anchor/script changes, selector/group lifecycle, or general value animation. |
| Animation | QtQuick supplies animation objects, groups, pause/path/spring/smoothed lifecycle. | Duration/easing behavior plus spring/smoothed physics exist at `mold-scene/src/lib.rs:720-920`. | **partial.** Running/paused/loops/direction, groups, pause/script/path and per-type lifecycle are absent. |
| `Color` storage | Mold parses ordinary sRGB hex colors and linearizes before sRGB framebuffer upload at `mold-render/src/lib.rs:1224-1238`. | The scene doc called stored channels linear. | **fixed by this audit.** Stored channels are now documented as sRGB encoded. |

### Widget boundary

`WrapperItem`, `WrapperMouseArea`, `WrapperRectangle`,
`ClippingWrapperRectangle`, and `IconImage` are convenience compositions over
QtQuick (`xtra/quickshell/src/widgets/CMakeLists.txt:7-18`). They are
**downstream-widget**, not Mold types. `Inset`, `ClipRect`, `Image`, input, and
layout primitives may enable downstream equivalents, but must not be described
as exact widget API parity where their behavior is reduced. Buttons, sliders,
bars, indicators, notification centers, and complete shells remain forbidden.

## Windows and general Wayland protocols

| Surface/protocol | Quickshell evidence | Mold evidence | Status and gap |
|---|---|---|---|
| Layer shell | Layer, namespace, focus, anchors, exclusion mode, margins and aliases at `wayland/wlr_layershell/wlr_layershell.hpp:102-115`; auto exclusion at `window/panelinterface.hpp:55-124`. | Layer/namespace/size/raw exclusive zone/output/anchors/margins/focus at `mold-wayland/src/lib.rs:137-163`. | **partial.** Core protocol exists; Normal/Ignore/Auto exclusion semantics and `aboveWindows`/`focusable` aliases are absent. |
| Own xdg toplevel | Floating window and common window state at `window/floatingwindow.hpp:100-105` and `window/windowinterface.hpp:53-205`. | Native `FloatingConfig`, move/resize, min/max/fullscreen and rendering at `mold-wayland/src/lib.rs:363-388,1333-1431`. | **partial/close.** Common surface behavior exists; exact observable content/data/backing/DPR/format state differs. |
| Xdg popup | Dynamic parent, anchor, relative geometry and grab at `window/popupwindow.hpp:52-87` and `core/popupanchor.hpp:76-136`. | Typed positioner, every constraint flag, grab, live mutation and parent validation at `mold-wayland/src/lib.rs:217-309,1232-1332`. | **partial/close.** Verify automatic item-geometry following and explicit update semantics before claiming full parity. |
| Session lock | Locked/secure/per-screen surfaces at `wayland/session_lock.hpp:56-179`. | Native ext-session-lock events and per-output surfaces at `mold-wayland/src/lib.rs:597-610,1442-1528`. | **partial.** Rust protocol primitive exists; direct Lua lock/unlock/surface configuration is not a public construct, and unlock is tied to internal PAM flow. |
| Idle notification | Enabled/timeout/respectInhibitors/isIdle at `wayland/idle_notify/monitor.hpp:21-76`. | Timeout subscriptions and idle events at `mold-wayland/src/lib.rs:563-564,879-886` and `mold-lua/src/lib.rs:4029-4052`. | **partial.** No handle state, unsubscribe/current value, or respect-inhibitors selection. |
| Idle inhibit | Enabled plus associated window at `wayland/idle_inhibit/inhibitor.hpp:27-73`. | No protocol binding. | **missing-general-core.** |
| Shortcut inhibit | Enabled/window/active/cancelled at `wayland/shortcuts_inhibit/inhibitor.hpp:26-87`. | No protocol binding. | **missing-general-core.** |
| Foreign toplevel | Other-client app ID/title/parent/state/screens/control and global model at `wayland/toplevel/qml.hpp:24-180`. | Mold own floating windows only. | **missing-general-core.** Own xdg toplevels are not foreign-toplevel management. |
| Workspace/windowset | Generic windowsets, projections, capability flags, actions, and global/screen models at `windowmanager/windowset.hpp:21-169` and `windowmanager/windowmanager.hpp:60-89`; ext-workspace backend under `wayland/windowmanager`. | No model or protocol binding. | **missing-general-core.** Backend-specific Hyprland/i3 models do not justify excluding the generic abstraction. |
| Screencopy | Still/live capture source abstraction, output or toplevel source, cursor, content state, constraint sizing at `wayland/screencopy/view.hpp:14-102`; ext image-copy/capture-source and wlr backends in nested CMake files. | One-shot output capture through wlr, bounded requests and shared-memory ARGB/XRGB at `mold-wayland/src/lib.rs:193-215,909-941`. | **partial.** No live view/source abstraction, ext capture source, foreign-toplevel capture, constraints, or dmabuf. |
| Background effect | Surface blur region through ext-background-effect at `wayland/background_effect/qml.hpp:18-78`. | Renderer blur affects Mold's own subtree content only. | **missing-general-core.** Renderer blur is not compositor blur-behind. |
| Clipboard | Global text read/write/change at `core/qmlglobal.hpp:138-141`. | Data-device text events/set/subscribe at `mold-wayland/src/lib.rs:570-571,944-973` and `mold-lua/src/lib.rs:4071-4097`. | **partial/close.** No synchronous current getter and no data-control persistence. |
| Outputs | Screen model and hotplug. | Geometry/metadata/hotplug and fractional scaling. | **partial** for metadata and shell-global model reasons above. |
| Output power | No equivalent pinned public module. | Native wlr output-power at `mold-wayland/src/lib.rs:417-424,888-907`. | **exceeds-reference.** |
| Virtual keyboard | No equivalent pinned public module. | Native virtual-keyboard-v1 at `mold-wayland/src/lib.rs:80-87,976-1014`. | **exceeds-reference.** |
| Input method and text input | No equivalent pinned public modules. | Native input-method-v2 and text-input-v3 at `mold-wayland/src/lib.rs:72-86,1021-1134`. | **exceeds-reference.** |
| Output management | No wlr output-management module. | No wlr output-management binding. | **absent from both.** Do not conflate output power with output management. |

The strongest protocol gaps that respect the rendering-engine/no-widget
boundary are foreign toplevels, ext-workspace/windowsets, idle inhibition,
shortcut inhibition, extended capture sources/live screencopy, dmabuf capture,
background effects, and exact layer exclusion behavior.

## Services and desktop integrations

These are protocol/data mechanisms, not widgets. They do not require Mold to
own a settings panel or indicator. They may be kept outside a renderer-only
milestone, but their absence prevents any whole-Quickshell parity claim.

| Integration | Quickshell surface | Mold status |
|---|---|---|
| Notifications | Freedesktop server, capabilities, notification model, urgency/actions/images/replies/expire/dismiss at `services/notifications/qml.hpp:31-76` and `notification.hpp:72-261`. | **optional-desktop, absent.** |
| Status notifier | Watcher/host, full item model, icons/tooltips/menu/activation/scroll at `services/status_notifier/watcher.hpp:18-60`, `host.hpp:19-48`, and `item.hpp:105-136`. | **optional-desktop, major reduction.** Mold only discovers addresses from an existing watcher at `mold-services/src/status_notifier.rs:53-135`; it neither owns a watcher nor reads/hosts full items. |
| DBusMenu | Remote menu items, handle, layout refresh at `dbus/dbusmenu/dbusmenu.hpp:51-122`. | **optional-desktop, absent.** Local `mold-menu` is not DBusMenu. |
| PipeWire | Nodes, links/groups, defaults, node audio channels/mute/volumes, readiness, peak monitor at `services/pipewire/qml.hpp:67-441` and `peak.hpp:31-43`. | **optional-desktop, major reduction.** Only node snapshot and volume read/write exist at `mold-services/src/pipewire.rs:23-46` and `mold-lua/src/lib.rs:5960-6039`. The plan's “graph” claim is not currently true. |
| PAM | Interactive conversation, arbitrary prompts, abort/respond/config at `services/pam/qml.hpp:20-101`. | **optional-desktop, partial.** Mold has a fixed username/password callback at `mold-services/src/pam.rs:97-219`. |
| greetd | Reactive state and asynchronous create/cancel/respond/launch lifecycle at `services/greetd/qml.hpp:16-95`. | **optional-desktop, partial.** Low-level bounded create/respond/start/cancel exists at `mold-services/src/greetd.rs:60-135`. |
| Polkit | Agent registration, authentication flows, identities/prompts/cancel at `services/polkit/qml.hpp:24-68` and `flow.hpp:24-97`. | **optional-desktop, absent.** |
| MPRIS | Player discovery, metadata, capabilities, transport controls at `services/mpris/player.hpp:90-272`. | **optional-desktop, absent.** |
| UPower/power profiles | Battery devices/states and profile selection/degradation/holds under `services/upower`. | **optional-desktop, absent.** |
| NetworkManager | Devices, connectivity, wired/Wi-Fi networks, scan/connect/disconnect/forget/settings under `network`. | **optional-desktop, absent.** |
| BlueZ | Adapter/device models and power/discovery/pair/trust/block/connect/battery under `bluetooth`. | **optional-desktop, absent.** |
| udev/xkb | No equivalent public Quickshell service modules. | **native Mold mechanisms; outside Quickshell surface comparison.** |
| logind | Not provided by the current Mold service crate. | **plan mismatch.** `mold-services/src/lib.rs:3-15` exports no logind module. |

## Correct exclusions

- X11 panel support and i3/Sway IPC under `xtra/quickshell/src/x11` are
  **platform-specific** and may remain absent from a Wayland-only Mold target.
- Hyprland IPC, global shortcuts, focus grab, surface extensions, and
  toplevel-export APIs under `xtra/quickshell/src/wayland/hyprland` are
  **platform-specific** and may remain absent.
- The Quickshell internal reload popup and tooltip under `src/ui` are
  **downstream-widget/internal**. Mold should expose reload status and errors,
  not own that presentation.
- Qt's QML engine, preprocessor, object scanners, logging adapters, image
  providers, and C++ plugin ABI are **internal** implementation choices.
- Every Button, Slider, Toggle, TextField, Card, menu presentation, bar,
  launcher, lock-screen presentation, network UI, notification center, and
  complete shell remains downstream.

## Contradicted previous claims

The previous documents said all included core, IO, window, and visual types had
an audited native mapping and marked every ledger row implemented. That was not
supported by the source:

1. `Variants` is one-shot, not live or reload-matched.
2. `BoundComponent` is absent and Loader has no component/source or true async lifecycle.
3. Candidate reload may start processes and bind sockets before validation.
4. Per-output runtimes duplicate shell-global state and side effects.
5. Scene property effects are not deterministically removed with their nodes.
6. Process and file-watcher queues are unbounded.
7. FileView async jobs, JsonAdapter reactivity, socket-server handlers, and most IpcHandler behavior are absent.
8. Image status/error, full focus/keyboard, transforms, anchors, views, effects, and broad text/shape/layout behavior are reduced.
9. Menus map only their local data model, not display/anchor/DBusMenu behavior.
10. Layer exclusion modes, foreign toplevels, windowsets, idle/shortcut inhibitors, extended screencopy, and background effects are absent or reduced.
11. Status notifier is address discovery, not item or watcher hosting; PipeWire is not a full graph; logind is not implemented.
12. Build, GPU, and live-surface smoke tests prove those lanes only. They do not prove public API parity.

## Prioritized general-core work

### P0: correctness and ownership

1. Move shell/config/service/IPC ownership to one native global runtime with
   per-output surface instances.
2. Add reload prepare/validate/commit/rollback activation barriers for every
   external side effect.
3. Own every reactive effect by a node or scope and tear it down
   deterministically.
4. Replace unbounded process/file queues with bounded or coalescing native
   queues and diagnostics.
5. Keep the audit and parity documents honest; no green smoke test may promote
   a partial surface to implemented.

### P1: missing reusable mechanisms

1. Implement live `Variants`, structured persistence, and dependency-hash file
   watching.
2. Add a native cancellable async-job model for FileView and Loader, plus a
   component/factory registry and BoundComponent-style value forwarding.
3. Add schema/property/signal/introspection support to IPC; add owned
   socket-server connection handlers and process parser/callback attachment.
4. Add foreign toplevel, ext-workspace/windowset, idle-inhibit,
   shortcut-inhibit, capture-source/live-screencopy, dmabuf capture, and
   background-effect primitives in Rust.
5. Expose session-lock construction independently of authentication policy.
6. Add image ready/error/reload state, full key release/repeat/modifiers, focus
   scopes, sibling anchors, transform origins and nonuniform transforms.

### P2: breadth after correctness

1. Expand text, shape, layout, view, Flickable, gesture, state, transition, and
   effect primitives according to downstream shell needs.
2. Fix SystemClock precision, model subscriptions/comparison mode, reactive
   JSON adapter behavior, and last-window lifecycle.
3. Decide explicitly which optional desktop integrations belong in Mold. Do
   not call them widgets, and do not claim parity for integrations left out.

## Current conclusion

Mold has a substantial native Rust engine: reactive signals, scene nodes,
rendering, clipping, regions, text shaping, image/icon loading, models,
process/file/socket basics, generic D-Bus client support, layer surfaces,
popups, own toplevels, session lock internals, screencopy basics, clipboard,
output tracking, input method, text input, and service primitives.

It does **not** have whole-Quickshell parity, and even the previously declared
general-core parity is not complete under the repository's own no-reduction
rule. The tables above are the authoritative gap ledger. The no-widget and
pure-Rust boundaries remain intact.
