# quickshell example

A port of `~/.config/quickshell` onto morf primitives, continuing where
`examples/board` left off.

Run it:

```sh
EXAMPLE=examples/quickshell/init.lua oslo make run
MORF_MONITOR=eDP-1 EXAMPLE=examples/quickshell/init.lua oslo make run
```

## What is covered

| Original | Here | Status |
|---|---|---|
| `board/` | `examples/board` | already ported |
| `border/modules/border/Border.qml` | `border.lua` | frame and rounded inner corners |
| `line/modules/line/Line.qml` | `line.lua` | ten workspace pills, live from `hyprctl` |
| `line/modules/line/Numbers.qml` | `line.lua` | the morphing workspace badge |
| `settings/…/Theme.qml`, `osd/…/Theme.qml` | `theme.lua` | pywal palette, geometry tokens |
| `shared/ribbon/Ribbon.qml` | `line.lua` | the ribbon's track and pill layout |
| `osd/` | `osd.lua` | volume, brightness, and the battery warning |
| `settings/…/Settings.qml` | `settings.lua` | the volume and brightness column |
| `border/…/Border.qml` reservers | `border.reserved()` | the exclusive zones that move windows |

`SettingsManager.qml` declares `settingsOnRight: !barOnRight`, so the volume and
brightness column sits on the edge opposite the workspace ribbon and shares its
`(monitorHeight - trackHeight) / 2` centring, with a track two pills tall.

Not yet ported: `shared/ribbon/RibbonPopup.qml`'s expanded state.

## Structure

The original is four Quickshell processes, and the border alone opens twelve
layer surfaces: four edges, four corners that punch a quarter-disc out of a
filled square with a `Canvas`, and four zero-size windows that exist only to
claim an exclusive zone.

morf binds one IPC socket per Wayland display and hosts one layer surface per
process, so all of it composes into a single fullscreen overlay with the input
region trimmed back to the workspace bar. For the border that is a
simplification rather than a workaround: a frame with rounded inner corners is
one path — the output rectangle, then the inset rounded rectangle wound as a
hole — and an even-odd fill leaves exactly the border, corners included, with no
seams to align.

## Fidelity

Checked against the running original with `grim`, sampling pixels rather than
eyeballing screenshots.

**Border.** Drawn over the original, the composite is unchanged: 0.83% of pixels
in a 220×220 corner differ by more than 8/255, all of them on the arc itself,
which is tessellation-versus-`Canvas` antialiasing. Thickness and inner radius
are identical.

**Ribbon.** Pill height (99px), spacing (10px), and every y position match
exactly. With `MORF_MONITOR` pointed at the same output, the active pill lands
on the same workspace in the same colour.

**One difference remains, and it is the engine's, not the port's.** Empty pills
are `color240` at 60% opacity. Qt composites that in sRGB space; morf linearises
colour for the GPU and blends in linear light. The two disagree, predictably:

```
60% of color240 (#6a8389) over color0 (#0e1213)
  blended in sRGB space   → #45565a   (original measures #46565a)
  blended in linear light → #54686d   (morf measures  #54686d)
```

Both predictions land on the measurement, the morf one exactly. Linear is the
physically correct way to blend and the reason morf does it, but it means any
port from a Qt or GTK shell will read lighter wherever alpha is used. Anything
that has to match a Qt original pixel-for-pixel needs the blend done up front
and handed over as an opaque colour.

## One thing worth knowing about child processes

The workspace data comes from `hyprctl monitors -j` and `hyprctl workspaces -j`
through `io.process_view`. Two things about that bit:

**Children inherit this process's dynamic linker environment.** Launching morf
through `nixVulkan` — which `oslo make run` does when the wrapper is present —
replaces `LD_LIBRARY_PATH` with nix store paths. A system binary that inherits
it fails to load its own libstdc++ and exits before printing anything, which
looks exactly like the command silently doing nothing: the process starts, exits
with status 1, and every byte arrives on stderr. `hypr.lua` clears
`LD_LIBRARY_PATH` for its children, and any config spawning system commands
needs the same.

**`process:next(timeout)` ignores its timeout.** It always polls with
`Duration::ZERO` (`api_process.rs`, `process_next`), so a drain has to be spread
across ticks rather than waiting on a single call. That is why `hypr.lua` keeps
a small per-tick budget and picks up where it left off.
