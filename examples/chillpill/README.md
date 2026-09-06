# chillpill

[ChillPill-Shell](https://github.com/LUCKYS1NGHH/ChillPill-Shell), the pill
bar for Hyprland, on morf instead of Quickshell.

Run it:

```sh
EXAMPLE=examples/chillpill/init.lua oslo make run
```

or as one file that carries its font with it:

```sh
oslo make bundle --example examples/chillpill/init.lua --name chillpill
./target/dist/chillpill
```

## What is here

| Original | Here | What it does |
|---|---|---|
| `PillBarModules.qml`, `Battery`, `Volume`, `Workspaces`, `Network`, `Clock`, `Bluetooth`, `Weather`, `Vpn` | `bar.lua` | the pill, with the modules `pillModules` lists |
| `MediaPlayer`, `CcButtons`, `CcSliders`, `NotificationModule`, `WifiPanel`, `BluetoothPanel`, `CountdownModule` | `control.lua` | the control center and its two side panels |
| `NotificationPopup`, `NotificationStack` | `notify.lua` | the `org.freedesktop.Notifications` server and the popups |
| `OsdBar`, `FullscreenOsd` | `osd.lua` | volume, brightness, battery and timer |
| `MediaPopup`, `MprisModule` | `mediapopup.lua`, `media.lua` | the track that just started |
| `CalendarBox`, `Datetime`, `IpStatus`, `DataUsage`, `WeatherModule`, `WeatherPopup` | `dashboard.lua`, `weather.lua` | the mini dashboard, its calendar and the weather |
| `AppLauncher` | `launcher.lua` | every desktop entry, filtered as you type |
| `Cliphist` | `cliphist.lua` | the clipboard history, over `cliphist` |
| `WallpaperSwitcher` | `wallpapers.lua` | a grid of `wallpapersDir`, set through hyprpaper |
| `Theme.qml` | `theme.lua` | colours, the two faces, one scale |
| `config.jsonc` | `config.lua` | the same keys, read from `~/.config/chillpill/config.json` |

`system.lua` is where the machine is read from: battery and backlight from
sysfs, sound from `wpctl` with `pactl subscribe` for the pushes, the network
from `nmcli`, Bluetooth from bluez over D-Bus, uptime and traffic from
`/proc`. `hypr.lua` reads workspaces from Hyprland's own sockets, and
`proc.lua` runs the child processes all of this needs. `field.lua` is a
line of text being typed, shared by the launcher, the clipboard search and
the Wi-Fi password.

## Keys

The original toggles its panels with `qs ipc call <module> toggle`; here
it is `morf ipc call <verb>`:

| verb | opens |
|---|---|
| `launcher` | the app launcher |
| `cliphist` | the clipboard history |
| `wallpapers` | the wallpaper switcher |
| `control` | the control center |
| `wifi`, `bluetooth` | the control center with that panel out |
| `dashboard` | the mini dashboard |
| `osd volume` | the OSD, for a look at it |
| `close` | everything |

`toggle <name>` takes the original's names too: `appLauncher`,
`controlCenter`, `miniDashboard`, `wallpaperSwitcher`, `cliphist`.

In the launcher, clipboard and wallpaper surfaces: type to filter, arrows
to move, Enter to choose, Escape to leave. The wheel over the volume module
changes the volume; over the workspaces it switches them; over the timer
button it cycles the presets. The timer button starts and stops. Reboot and
power off ask for a second click within three seconds.

## Settings

`~/.config/chillpill/config.json`, with the original's keys; comments and
trailing commas are fine. The defaults are in `config.lua`. Two are new:
`dpiScale = "auto"` picks a scale from the output's height so the pill is
the same size on a laptop panel and a 4K monitor, and `country` is the two
letters the calendar's public holidays are fetched for.

The text face is Monocraft, shipped in `assets/fonts` (OFL-1.1). Icons come
from the first Nerd Font installed, or the one `iconFont` names.

## Shape

One fullscreen layer surface holds the pill, the panels, the popups and
the OSD; its input region is exactly the parts that can be clicked, so the
desktop underneath keeps working. The launcher, the clipboard history, the
wallpaper switcher and the Wi-Fi password prompt each get a layer surface of
their own with exclusive keyboard focus, opened and closed from `init.lua`.

Every output runs the configuration once. A verb from the terminal reaches
all of them and the instance on the focused monitor acts; the pill on each
output follows the focused monitor's workspaces, or `MORF_MONITOR`'s. The
notification server can be owned by only one instance, so popups appear on
one output.

Needs: `wpctl`, `pactl`, `nmcli`, `hyprpaper`; optionally `cliphist` and
`wl-copy` for the clipboard, `brightnessctl` when the backlight file is not
writable, `curl` for the weather, album art and holidays.
