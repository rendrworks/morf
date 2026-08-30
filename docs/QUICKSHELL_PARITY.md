# Quickshell parity

An audit of `~/.config/quickshell` — border, line, osd, settings, board — against
what mold can express, and what it took to close the gaps.

Method: five agents read one module each against the mold source, every claimed
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

mold could express neither half:

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
  `mold.surface` table. That name was a trap.)
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
| 5 | Auxiliary surface with runtime visibility | major | osd | fade-out cut at frame 0 | open (E3) |
| 6 | `xdg_popup.reposition` | major | settings, line | animated popup re-creates its swapchain every frame | open (E3) |
| 7 | Runtime-mutable layer geometry | major | line, settings, osd | `margin_left = x` silently does nothing | open (E4) |
| 8 | Enumerating every output from Lua | major | line, settings | `barOnRight` needs `hyprctl` | open (E6) |
| 9 | Pointer coordinates on press/release/click | major | osd, settings, board | every slider needs a cached last-motion x | open (E5) |

Two further defects were found during implementation, neither in the original
audit, both now **fixed**:

- **Reservers never mapped.** The first cut created the surfaces with their
  exclusive zones and committed — with no buffer attached. wlroots skips
  unmapped layer surfaces when computing usable area, so the reservation
  reserved nothing. The protocol requires the initial commit to carry no buffer,
  a configure, and only then an attach; mold now defers a 1×1 transparent SHM
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
  construction; mold now does the same.

## Engine work

| | Change | Status |
|---|---|---|
| **E1** | Plural layer role: `SurfaceRole::Layer(u64)`, `layers: HashMap<u64, LayerRecord>`, `open_layer`/`close_layer`, `WindowSurfaceKind::Layer`, a real `window.layer{}` constructor | **done** |
| **E2** | `mold.surface.reserve = { top, right, bottom, left }` opening one internal single-anchor reserver per edge | **done** |
| **E3** | `xdg_popup.reposition` — sctk already has it (`popup.rs:139`); split popup change detection into structural vs positional so a moving popup keeps its renderer | open |
| **E4** | Runtime layer geometry — `set_size`/`set_anchor`/`set_margin` on a mapped surface, no reconnect | open |
| **E5** | Pointer coordinates on button events. `hit_node` already computes `local_x`/`local_y` at `layout.rs:173` and discards them | open |
| **E6** | Enumerate all outputs from Lua — the list already travels worker→supervisor and is dropped at `surface_events.rs:404-409` | open |

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

## One thing that will not match

mold blends alpha in **linear light**; Qt blends in **sRGB**. Measured:

```
60% of color240 (#6a8389) over color0 (#0e1213)
  blended in sRGB space   → #45565a   (original measures #46565a)
  blended in linear light → #54686d   (mold measures  #54686d)
```

Both predictions land on the measurement, mold's exactly. Linear is the
physically correct way to blend and the reason mold does it — but any port from a
Qt or GTK shell reads lighter wherever alpha is used. Anything that must match
pixel-for-pixel needs the blend precomputed and handed over as an opaque colour.

## Two traps worth knowing

**Child processes inherit the dynamic linker environment.** Launching mold
through a nixGL-style wrapper (`oslo make run` does, when `nixVulkan` is present)
replaces `LD_LIBRARY_PATH` with nix store paths. A system binary that inherits it
fails to load its own libstdc++ and exits 1 with every byte on stderr — which
looks exactly like the command silently doing nothing. Pass
`environment = { LD_LIBRARY_PATH = "" }` to `io.process_view` for system commands.

**`process:next(timeout)` ignores its timeout.** `api_process.rs` always calls
`next_event(Duration::ZERO)`, so a drain must be spread across ticks rather than
waiting on one call.
