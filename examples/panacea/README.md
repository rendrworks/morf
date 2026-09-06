# panacea

[Panacea](https://github.com/EnsixD/Panacea), the one-pill desktop shell
for Hyprland, on morf instead of Quickshell.

One black capsule at the top edge is the whole shell. Collapsed it shows
the day, the clock, the workspace, the keyboard layout and the battery.
Asked for a page it grows downwards into that page on a spring and flows
back when the page closes. It hugs the screen edge with two concave
corners, like a hardware notch.

Run it:

```sh
EXAMPLE=examples/panacea/init.lua oslo make run
```

## Pages

| Page | Verb | What it does |
|---|---|---|
| Quick settings | `controls` | clock and date, Wi-Fi, Bluetooth and sound tiles, now playing, the recorder, the battery, coffee mode, lock, notifications, settings |
| Networks | `wifi` | scan, connect, forget; the password is typed into the page |
| Bluetooth | `bluetooth` | the adapter's switch, scanning, connect, disconnect, forget |
| Notifications | `notifications` | the history, do not disturb, clear; fresh ones float under the pill as cards |
| Launcher | `launcher` | every desktop entry, recents first; a sum typed instead is worked out and Enter copies it |
| Clipboard | `clipboard` | `cliphist` history with search |
| Calendar | `calendar` | the month, today marked, the weather when `weatherLocation` is set |
| Recorder | `record` | wf-recorder: frame rate, folder, system audio, microphone |
| Sound | `audio` | output, input, and every playing stream with its own slider |
| Battery | `battery` | charge, power profile, capacity, health, power drawn |
| Power | `powermenu` | sleep, lock, log out, restart, shut down; twice to confirm |
| Workspaces | `overview` | every workspace on the focused monitor and its windows |
| Settings | `settings` | island, clock, appearance, motion, notifications, system |
| Shortcuts | `shortcuts` | every binding from the settings |
| Now playing | `media` | art, transport, and an equaliser that is also the seek bar |
| Wallpapers | `wallpapers` | a carousel of the wallpaper folder, set through hyprpaper |
| Weather | `weather` | conditions, three tiles, sunrise and sunset, three days |

`morf ipc call <verb>` toggles a page, as `qs ipc call pill <verb>` does in
the original; `dnd`, `recordToggle`, `smartClose` and `close` are there
too. Every page closes with Escape or a click outside; a click on the
pill opens quick settings. `pillHover` in the settings makes a hover open
it too, as the original does; it is off by default.

## Colours and type

When lule has written `~/.cache/lule/colors.json`, the island takes its
colours from there: the background, the foreground, and the cursor as the
accent. `"themeId": "panacea"` in the settings keeps Panacea's own black
and blue instead. Words are set in `fontFam` when that face is installed
(the default here is Goku, a pixel face); icons always come from a Nerd
Font, since a pixel face has none.

A page reached from another -- Wi-Fi from quick settings, say -- has a
back arrow at its top right that returns to the one before. Escape closes
the island whatever page it is on.

## Settings

`~/.config/panacea/settings.json`, with the original's keys: `pillH`,
`panelW`, `cornerR`, `notchFlare`, `collapsedW`, `expandedH`, `notchMode`,
`pillOverlay`, `fontFam`, `fontSize`, `iconSize`, `colFg`, `colOn`,
`mutedAlpha`, `themeId`, `clock12`, `animMove`, `animBounce`,
`reduceMotion`, `notifTimeout`, `notifDnd`, the `bind_*` keys, and more in
`config.lua`. The settings page writes the same file. `dpiScale` is new:
Panacea's sizes are for a 1080p screen, and "auto" scales them by the
output's height.

Motion follows `animMove` and `animBounce`: a spring whose settle time is
the move and whose damping is the bounce. `reduceMotion` lands every move
at once.

## Motion

Nothing fades in. The strip's five pieces -- the day, the clock, the
workspace, the layout, the battery -- are the same nodes whether the
island is collapsed or open. Opening a page, the clock's letters walk into
the page's title and the day's into its subtitle (morf's text morph), the
battery's glyph walks into the page's icon (a glyph morph in a field), and
each piece travels on a spring from its place in the strip to where the
page keeps it: the battery lands in the quick settings tile and on the
battery page's card, the workspace number in the overview's active tile.
Pieces a page has no place for gather into the title. The page's content
grows out of the strip under them, and the capsule springs to the size of
what it holds. Closing runs it all back.

Each piece is set in type once, at the largest size it takes, and scaled
from there: a scale is a transform the GPU applies for nothing, where a
font size that moves re-shapes the letters every frame.

With `pillHover` on, hovering the strip opens it too -- the player if
something plays, quick settings otherwise -- and it goes when the pointer
has left. A page opened by a click or a key stays until Escape or a click
outside.

The equaliser is cava when it is installed, a real spectrum; otherwise the
loudness from pw-record on the output's monitor, normalised against the
loudest recent moment, spread across the bars.

## Shape

The island lives on the shell's own surface, collapsed or open, so a page
is one spring away. While a page is open the surface asks for exclusive
keyboard focus and gives it back after; the compositor re-reads that on
the commit. (This is what `morf.surface.keyboard_focus` written at runtime
does now.) The island's size is a binding on its content's laid-out
height, with a spring on it, so every page is the same capsule at a
different size.

The surface is the island's size while anything moves, not the screen's.
Every frame is cleared, composed and blended by the compositor at the
surface's size, and a fullscreen surface made a small island cost a whole
4K frame on both sides of the protocol. Once a page is open and still, the
surface grows to the screen so a click anywhere else closes it, and
shrinks again the moment the page starts to close; the island is centred
at the top edge either way, so nothing on screen moves when the surface
does.

Everything that polls a little -- the reveal of a node, a settings section
-- shares one ticker. A timer is a thread that wakes every output's loop
each time it fires, and fifty of them were a quarter of a core doing
nothing.

The concave corners are two small SVGs, a square with a quarter circle
taken out, drawn either side of the capsule's top.

Every output runs the configuration once; the instance on the focused
monitor opens pages. The notification server is owned by one instance,
so cards appear on one output.

Shared with the ChillPill example: `system.lua` (battery, sound,
backlight, network, Bluetooth), `hypr.lua` (workspaces and the keyboard
layout from Hyprland's sockets), `proc.lua`, `media.lua`, `field.lua`,
`weather.lua`, `lib/notifications.lua`.

Not here: the file manager, the media viewer, the password vault, the
agents panel and the lock screen. The login
example carries a lock of its own, which the lock button uses when it is
installed as `logre`.

Needs: `wpctl`, `pactl`, `nmcli` or `iwctl`; optionally `wf-recorder`,
`cliphist`, `wl-copy`, `upower`, `power-profiles-daemon`, `curl`.
