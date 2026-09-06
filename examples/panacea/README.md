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

`morf ipc call <verb>` toggles a page, as `qs ipc call pill <verb>` does in
the original; `dnd`, `recordToggle`, `smartClose` and `close` are there
too. Every page closes with Escape or a click outside. Hovering is not
opening here: a click on the pill opens quick settings.

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

## Shape

The collapsed pill lives on the shell's own surface, which takes no
keyboard. A page lives on a fullscreen overlay surface with exclusive
keyboard focus, opened when a page opens and closed once the island has
flowed back; the surface is what hears Escape and the click outside. The
island's size is a binding on its content's laid-out height, with a spring
on it, so every page is the same capsule at a different size.

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
agents panel, the wallpaper carousel and the lock screen. The login
example carries a lock of its own, which the lock button uses when it is
installed as `logre`.

Needs: `wpctl`, `pactl`, `nmcli` or `iwctl`; optionally `wf-recorder`,
`cliphist`, `wl-copy`, `upower`, `power-profiles-daemon`, `curl`.
