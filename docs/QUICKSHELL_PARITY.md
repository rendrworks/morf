# Quickshell parity

An audit of `~/.config/quickshell` — border, line, osd, settings, board — against
what morf can express, and what it took to close the gaps.

Method: five agents read one module each against the morf source, every claimed
gap then went through three independent adversarial verifiers instructed to
*refute* it, and only claims that survived a majority are recorded here. Every
status is backed by a file:line citation rather than by inference from names.

Status key: **fixed** — landed and verified. **open** — real, not yet done.
**refuted** — the audit claimed a gap and the verifiers disproved it.

## The two complaints

### The border did not move windows — **fixed**

Two stacked limits, both real.

`Border.qml` opens twelve layer surfaces per output: four `Edge` and four
`Corner` at `ExclusionMode.Ignore` that draw *outside* the usable area, and four
zero-size `Reserve` windows (`Border.qml:124-143`) whose entire purpose is
`exclusiveZone: reservedThickness` on one anchored edge each. The reservation is
the module's function; the frame is only its decoration.

morf could express neither half:

- `state_types.rs:29` — `layer: Option<LayerSurface>`. One surface per process,
  created once at `client_connection.rs:137`. Compare its siblings on the next
  two lines: `popups: HashMap<u64, Popup>`, `floatings: HashMap<u64, Window>`.
- Even one surface cannot reserve four edges. A positive zone is honoured only
  for an unambiguous anchored edge, and `set_exclusive_edge` is layer-shell v5
  while smithay-client-toolkit 0.21.1 binds `1..=4` with no wrapper — the
  request is not reachable through the dependency at all.
- Four extra processes are not a workaround either: the IPC socket is keyed on
  `WAYLAND_DISPLAY` (`config.rs:119-128`) and a second bind fails fatally
  (`supervisor.rs:37`).

Fixed by E1 and E2 below. Verified live: `hyprctl monitors` reports
`reserved=[23,23,23,23]` on a 3456-wide output and `[25,25,25,25]` on a 3840-wide
one — exactly `round(width*0.005) + round(short*6/2160)`, matching `Border.qml`.

### There were no popups — **refuted, this was a port omission**

The engine was never the obstacle.

- `api_module.rs:423-424` — `window.popup` and `window.floating` are real
  constructors. (`window.layer_surface` was *not*; it was an alias of the
  `morf.surface` table. That name was a trap.)
- `client_surface.rs:139-147` — a parentless popup is created as an xdg popup and
  adopted by the layer surface via `layer().get_popup(...)`, the same
  construction Quickshell's `PopupWindow` uses.
- `tests/config.rs:271-320` already asserts four concurrent auxiliary surfaces
  stay independent.

And most of these panels never needed a second surface. Every OSD, both settings
panels and the badge are `exclusiveZone: 0`, `keyboardFocus: None`, transparent
overlays — they are *nodes*. The port's surface is already fullscreen, and the
input region is re-derived every paint from live `MouseArea` geometry
(`paint.rs:26-42`), so it tracks a panel as it expands with no bookkeeping. The
port used exactly this for the workspace badge and then stopped.

## Confirmed gaps

| # | Capability | Sev | Modules | What breaks | Status |
|---|---|---|---|---|---|
| 1 | >1 layer surface per process | blocker | all | 12 surfaces collapse to 1 | **fixed** (E1) |
| 2 | Four simultaneous exclusive zones | blocker | border | windows sit under the frame | **fixed** (E2) |
| 3 | Multiple top-level scene roots | blocker | osd | config refuses to start | **fixed** (E1) |
| 4 | Runtime map/unmap of the primary surface | blocker | osd, board | panel mapped for process life | **fixed** (E1) |
| 5 | Auxiliary surface with runtime visibility | major | osd | fade-out cut at frame 0 | **fixed for layer surfaces** (E1); a *popup* hide is an xdg-shell limit, see below |
| 6 | `xdg_popup.reposition` | major | settings, line | animated popup re-creates its swapchain every frame | **fixed** (E3) |
| 7 | Runtime-mutable layer geometry | major | line, settings, osd | `margin_left = x` silently does nothing | **fixed** (E4) |
| 8 | Enumerating every output from Lua | major | line, settings | `barOnRight` needs `hyprctl` | **fixed** (E6) |
| 9 | Pointer coordinates on press/release/click | major | osd, settings, board | every slider needs a cached last-motion x | **fixed** (E5) |

Two further defects were found during implementation, neither in the original
audit, both now **fixed**:

- **Reservers never mapped.** The first cut created the surfaces with their
  exclusive zones and committed — with no buffer attached. wlroots skips
  unmapped layer surfaces when computing usable area, so the reservation
  reserved nothing. The protocol requires the initial commit to carry no buffer,
  a configure, and only then an attach; morf now defers a 1×1 transparent SHM
  buffer to the configure handler. No test could have caught this: the reserve
  tests asserted on the config struct, never on a round trip.
- **Configured layer surfaces repainted forever.** `paint_layer` requested a
  frame callback unconditionally, so four permanent border surfaces meant four
  GPU renders and four commits every vblank with a completely static scene. Now
  gated on a dirty flag the animation tick sets.

And one that predates all of this, **fixed**:

- **Behaviors were installed before properties** (`configure.rs:34` vs `:43`), so
  every element animated its own construction — colours easing up from the
  schema default, widths growing from zero. A flash-in on every config,
  including `examples/board`. Qt's `Behavior` withholds itself during component
  construction; morf now does the same.

## A hidden popup cannot be kept alive — protocol limit, not a morf gap

Gap 5 has two halves. The half that hurt in practice — a popup that *moves* —
is fixed: `sync_window_surfaces` now classifies a popup config change as
structural or positional (`surfaces.rs:148`), and a positional one issues
`xdg_popup.reposition` (`client_surface.rs:194`) instead of tearing the surface
down, so the renderer, the wgpu surface and the swapchain survive the move. A
compositor whose `xdg_popup` predates version 3 has no such request; it changes
nothing and reports `false`, and the caller then falls back to the old close and
re-open (`surfaces.rs:352`).

The other half cannot be fixed from the client. xdg-shell gives a popup no
unmapped-but-alive state: a null buffer unmaps a *toplevel*
(`xdg-shell.xml:644`), while the only client-side way to unmap a popup is
destroying the `xdg_popup`, which "will also dismiss the popup, and unmap the
surface" (`xdg-shell.xml:1283-1285`). An unmapped xdg_surface must redo its
initial commit before a buffer may be attached again (`xdg-shell.xml:455-458`),
and with the role object gone that means a new `xdg_popup`, a new `wl_surface`
and therefore a new swapchain. So `visible = false` on a popup does cut a
fade-out at frame zero, and morf cannot keep the renderer across it. The re-open
path is at least as cheap as it can be — one `open_popup` and one
`WgpuBackend::new_surface` on the first configure, nothing else.

Anything that must fade *out* therefore belongs on a surface that can be
unmapped and re-mapped, or on no extra surface at all:

- `zwlr_layer_surface` supports it explicitly — "The client can re-map the
  surface by performing a commit without any buffer attached, waiting for a
  configure event and handling it as usual"
  (`wlr-layer-shell-unstable-v1.xml:113-119`).
- A node on the shell's own fullscreen surface needs no unmap whatsoever, which
  is what the port already does for every `exclusiveZone: 0` overlay.
- Failing both, run the fade in the config and flip `visible` when it settles.

## Engine work

| | Change | Status |
|---|---|---|
| **E1** | Plural layer role: `SurfaceRole::Layer(u64)`, `layers: HashMap<u64, LayerRecord>`, `open_layer`/`close_layer`, `WindowSurfaceKind::Layer`, a real `window.layer{}` constructor | **done** |
| **E2** | `morf.surface.reserve = { top, right, bottom, left }` opening one internal single-anchor reserver per edge | **done** |
| **E3** | `xdg_popup.reposition` — popup change detection split into structural (parent, grab: `surfaces.rs:148`) and positional (everything the positioner carries), so a moving popup is repositioned at `surfaces.rs:352` and keeps its `wl_surface`, its GPU surface and its swapchain | **done** |
| **E4** | Runtime layer geometry — `set_size`/`set_anchor`/`set_margin`/`set_exclusive_zone`/`set_keyboard_interactivity` re-issued on a mapped surface (`client_layer.rs:132`), applied from `morf.surface` at `surface_run.rs:190`. Nothing is destroyed: the zwlr surface, the `wl_surface`, the viewport and the renderer all survive | **done** |
| **E5** | Pointer coordinates on button events. `Hit` (`morf-layout/src/hit.rs`) now carries `local_x`/`local_y` instead of discarding them, and `EventPoint` (`morf-lua/src/runtime_input.rs`) delivers both spaces to `on_pressed`, `on_released`, `on_clicked`, `on_dragged` and the touch handlers | **done** |
| **E6** | Enumerate all outputs from Lua — the supervisor records the list every worker reports (`supervisor.rs`), applies it before a configuration loads and broadcasts it on hotplug; `Runtime::set_screens` rebuilds `morf.screens` with the instance's own output still at index 1 | **done** |

Every one of these is covered by a test that exercises the behaviour rather
than the struct, which is the lesson the reserver bug taught:

- **E4** — `shell_surface_geometry_reports_only_real_changes` asserts an
  assignment flags exactly one reconfiguration and a re-assignment of the same
  value flags none; `shell_surface_geometry_accepts_interpolated_numbers`
  asserts a float margin rounds rather than raising, which is what makes a
  margin animatable; `one_anchor_conversion_serves_creation_and_reconfiguration`
  asserts creation and reconfiguration derive the same anchor mask.
- **E5** — `pointer_handlers_receive_both_coordinate_spaces` presses a
  `MouseArea` offset inside its parent and asserts the Lua handler is called
  with surface `(130,55)` and node-local `(30,15)` for press, release and click;
  that a drag keeps its displacement in surface space while the local pair runs
  past the node's bounds; and that events carrying no position are refused.

E2's design note: four single-anchor reservers need no protocol work, which is
why making the role plural was the whole fix. `set_exclusive_edge` would only be
an optimisation, and is unreachable on the pinned sctk.

## Port status

**border** — frame verified against the original at 0.83% differing pixels in a
220×220 corner, all of it antialiasing on the arc; thickness and inner radius
identical. Reservation now wired through `border.reserved()`.

**line** — pill geometry exact (99px height, 10px spacing, matching y positions);
active pill matches the original's workspace and colour. Rebuilt on Hyprland's
event socket instead of a 120 ms `hyprctl` poll.

**osd**, **settings** — written (769 and 1110 lines) but not yet wired into
`init.lua`, and carrying known defects from review.

**board** — the dead `board.battery` and `board.brightness` signals, which had no
writer anywhere and rendered a permanently empty bar, now have one.

## What made the shell burn a core

Three defects compounded, and none of them showed up in a test.

**The input region was rasterized over the whole output, on every paint.**
`morf_region::build` allocated a `width * height` boolean mask *per region* and
scanned another one to extract rectangles — 8.3M pixels a pass on a 4K output,
however small the regions were. A shell whose interactive parts are a 19px bar
and two panels paid for 3840×2160, four times over. It now works in windows
merged from the rectangles the tree actually contains: every operation here is
pointwise, so composing over a subset gives the same answer as composing whole
and restricting, and a pixel can only be set where some region covers it. A
plain rectangle also fills whole rows instead of testing corners per pixel, and
run matching walks two sorted lists instead of hashing per row. Measured on the
quickshell port, debug build: **374ms → 8ms**.

**The clock forced a repaint every second whether or not anything read it.**
`update_clock` returned `()` and the caller set `repaint = true`. A shell that
shows no time still re-tessellated and re-submitted, per output, once a second.
It now reports whether the scene actually changed, the same question
`poll_services` answers — and for the same reason: a signal moving is not a
scene changing.

**Configured layer surfaces asked for a frame callback unconditionally**, so
four permanent border reservers meant four GPU renders and four commits every
vblank against a completely static scene. Now gated on a dirty flag.

Together, on three outputs: **131% of a core → 3%** (release, settled), with no
repaints demanded at idle. A debug build idles around 19%, almost all of it
interpreter overhead in the per-tick Lua timers.

The lesson for the engine is the one the reserver bug already taught: none of
this was visible from the config, and every test asserted on structs rather
than on what a frame costs.

## One thing that will not match

morf blends alpha in **linear light**; Qt blends in **sRGB**. Measured:

```
60% of color240 (#6a8389) over color0 (#0e1213)
  blended in sRGB space   → #45565a   (original measures #46565a)
  blended in linear light → #54686d   (morf measures  #54686d)
```

Both predictions land on the measurement, morf's exactly. Linear is the
physically correct way to blend and the reason morf does it — but any port from a
Qt or GTK shell reads lighter wherever alpha is used. Anything that must match
pixel-for-pixel needs the blend precomputed and handed over as an opaque colour.

## Two traps worth knowing

**Child processes inherit the dynamic linker environment.** Launching morf
through a nixGL-style wrapper (`oslo make run` does, when `nixVulkan` is present)
replaces `LD_LIBRARY_PATH` with nix store paths. A system binary that inherits it
fails to load its own libstdc++ and exits 1 with every byte on stderr — which
looks exactly like the command silently doing nothing. Pass
`environment = { LD_LIBRARY_PATH = "" }` to `io.process_view` for system commands.

**`process:next(timeout)` ignores its timeout.** `api_process.rs` always calls
`next_event(Duration::ZERO)`, so a drain must be spread across ticks rather than
waiting on one call.
