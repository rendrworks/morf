# Board example

A port of `~/.config/quickshell/board` onto morf primitives: the same six cards
on the same screen-scaled grid, the same pywal palette, borders, font family and
weights, and the same easing. Everything is composed from `Item`, `Rect`,
`ClipRect`, `Text`, `Image`, `MouseArea` and `Timer`; nothing about a "card" or a
"progress bar" is known to the engine.

The two bundled IosevkaTerm Nerd Font Mono faces make the result independent of
system font installation; their license is stored beside them. The font files
come from Nerd Fonts 3.4.0's `IosevkaTerm` package.

## What is live

| Card | Reads | How |
| --- | --- | --- |
| Logo | battery percentage | `/sys/class/power_supply/*/capacity` through `io.file_view`, re-read every 30s |
| User | uptime | `/proc/uptime`, formatted the way `uptime -p` prints it, every 60s |
| User | volume | `pamixer --get-volume` every 3s; the bar seeks with `pamixer --set-volume` |
| User | brightness | `/sys/class/backlight/*/brightness` and `max_brightness` every 2s; the bar seeks through `~/.local/sbin/bright`, the same script the original drives |
| Clock | time and date | `core.system_clock`, with the colon blinking on a 500ms timer |
| Calendar | month grid | computed in Lua; `<` and `>` step the shown month, and the today ring follows the hour-precision clock |
| Media | player state | `playerctl -a metadata --format …` every 500ms while a player is up, every 2s otherwise |

The system card is a label, exactly as `cards/SystemCard.qml` is.

Battery and brightness are plain sysfs reads, so they cost no subprocess at all.
Brightness is converted with the same perceptual curve `bright get` applies
(`log(raw) / log(max)`), so the readout matches the original's rather than the
raw register value.

## Interaction

The whole rounded board takes the pointer, as `mask: Region { item: container }`
does in `Board.qml`. On top of that:

* The volume and brightness bars are click-and-drag, and update locally while
  the write is in flight, as `StyledProgressBar`'s `interactive` mode does.
* The calendar's month arrows step the grid and light up under the pointer;
  day cells take a hover ring.
* The media card has the original's five transport buttons — shuffle, previous,
  play/pause, next, loop — plus a draggable seek bar and a player volume bar.
  Previous restarts the track when it is more than eight seconds in, exactly as
  `MediaPanel.qml` does.

Pressed and released carry no coordinates in morf, so a bar keeps the pointer
position from the motion and drag events that precede them.

## Deliberate omissions

* **`Watcher.qml`.** The original shows the board only when the focused Hyprland
  workspace goes empty, and hides it on a mouse-movement timer. That is
  compositor policy for a consumer plugin, not for an example; the board here is
  simply always up, and `LogoCard`'s pin toggle — which exists only to suppress
  that policy — is left out with it.
* **khal.** `CalendarService.qml` shells out to `khal` for event dots. khal is
  not installed on this machine, so the original draws no dots either.
* **The distro glyph.** `UserInfoCard.qml` puts a hardcoded Arch glyph between
  the user name and the uptime. Adding it would move the two lines the port was
  measured against, so the left column is left as it is.
* **Album art over http.** morf's image cache resolves `file://` and nothing
  else, so a player advertising a remote cover falls back to the same note glyph
  the empty state uses.
* **The album-art blob.** `DankAlbumArt.qml` morphs a 28-segment `Shape` behind
  the cover from a fake audio spectrum. That is a per-frame path rebuild, which
  needs a driver morf does not expose to Lua; the cover keeps its ring instead.

Nothing here is a Hyprland, Bluetooth or notification integration, and nothing
here adds a widget to morf. The example renders the board through general
primitives.

## Running

    EXAMPLE=examples/board/init.lua oslo make run
