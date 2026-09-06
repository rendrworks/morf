-- Settings, read from `~/.config/panacea/settings.json`.
--
-- The keys are Panacea's own, so the file its settings page writes reads
-- here unchanged; anything it leaves out takes the default below.

local core = require("morf.core")
local io = require("morf.io")

local config = {
  -- Type.
  fontFam = "JetBrainsMono Nerd Font",
  fontSize = 15,
  iconSize = 17,
  -- Colours: the text, the accent, and how far the quiet text is dimmed.
  colFg = "#ffffff",
  colOn = "#3b82f6",
  mutedAlpha = 0.45,
  themeId = "default",
  -- The island: its collapsed height and width, the width of a page, the
  -- tallest a page grows, its corner radius, and the flare of the two
  -- concave corners where it meets the screen edge.
  pillH = 38,
  collapsedW = 260,
  panelW = 540,
  expandedH = 620,
  cornerR = 14,
  notchMode = true,
  notchFlare = 12,
  islandGap = 12,
  pillPos = "top",
  -- Floating over the windows, or reserving its strip.
  pillOverlay = true,
  pillAutoHide = false,
  -- Hovering the strip opens it, as the original does. Off: only a click
  -- or a key opens a page.
  pillHover = false,
  -- Clock.
  clock12 = false,
  clockSeconds = false,
  clockWeekday = true,
  clockDateFmt = "%-d %B",
  -- Motion, in milliseconds, and the overshoot of a move in percent.
  animMs = 230,
  animMove = 230,
  animFade = 200,
  animHover = 150,
  animBounce = 79,
  reduceMotion = false,
  -- Notifications.
  notifDnd = false,
  notifTimeout = 5000,
  notifCritTimeout = 0,
  notifPreview = true,
  -- Features that can be switched off.
  featLauncher = true,
  featWifi = true,
  featBluetooth = true,
  featClipboard = true,
  featNotifications = true,
  featCalendar = true,
  featRecord = true,
  featAudio = true,
  featPowermenu = true,
  featLock = true,
  -- Where recordings go.
  recordDir = "~/Videos",
  recordFps = 60,
  -- Where the wallpapers are; empty tries ~/.config/hypr/wallpaper and
  -- ~/Pictures/wallpapers.
  wallpaperDir = "",
  -- Weather, for the calendar and weather pages.
  weatherLocation = "",
  weatherUnits = "metric",
  -- Programs.
  terminal = "foot",
  lockCommand = "",
  -- A number, or "auto": 1 on a laptop panel, more on a 4K output. Not
  -- Panacea's key: its sizes are for a 1080p screen.
  dpiScale = "auto",
  -- Bindings, shown on the shortcuts page.
  bind_pillControls = "SUPER + Z",
  bind_pillLauncher = "SUPER + A",
  bind_pillClip = "SUPER + V",
  bind_pillWifi = "SUPER + SHIFT + W",
  bind_pillBt = "SUPER + SHIFT + B",
  bind_pillSettings = "SUPER + I",
  bind_pillPower = "CTRL + ALT + delete",
  bind_terminal = "SUPER + T",
  bind_browser = "SUPER + F",
  bind_fileManager = "SUPER + E",
  bind_closeWindow = "SUPER + Q",
  bind_fullscreen = "SUPER + SHIFT + F",
  bind_screenshot = "SUPER + SHIFT + S",
  bind_emptyWorkspace = "SUPER + Space",
  bind_specialWorkspace = "SUPER + S",
  bind_toggleSplit = "SUPER + J",
  bind_exitHypr = "SUPER + SHIFT + M",
  bind_themeSwitch = "SUPER + SHIFT + T",
}

local home = core.env("HOME") or ""
config.home = home
-- Set while frame_bench draws one page: sizes land instead of springing,
-- since the bench draws one frame and a spring needs many.
config.bench = (core.env("PANACEA_OPEN") or "") ~= ""

--- `~` at the start of a path is the home directory.
function config.expand(path)
  if type(path) ~= "string" then return "" end
  if path:sub(1, 2) == "~/" then return home .. path:sub(2) end
  if path == "~" then return home end
  return path
end

local dir = (core.env("XDG_CONFIG_HOME") or (home .. "/.config")) .. "/panacea"
local path = dir .. "/settings.json"
config.dir = dir
config.path = path

local ok, handle = pcall(io.file, path)
if ok and handle then
  local read, text = pcall(handle.read, handle)
  if read and type(text) == "string" and text ~= "" then
    local decoded, values = pcall(io.json.decode, text)
    if decoded and type(values) == "table" then
      for key, value in pairs(values) do
        if config[key] == nil or type(value) == type(config[key]) then
          config[key] = value
        end
      end
    end
  end
end

-- What the shared modules read.
config.clockFormat = config.clock12 and "12h" or "24h"
config.maxWorkspaces = 10
config.recordDir = config.expand(config.recordDir)

--- Writes the settings back, for the settings page.
function config.save(changes)
  for key, value in pairs(changes) do config[key] = value end
  local out = {}
  for key, value in pairs(config) do
    local kind = type(value)
    if kind == "string" or kind == "number" or kind == "boolean" then
      if key ~= "home" and key ~= "dir" and key ~= "path" and key ~= "clockFormat" and key ~= "maxWorkspaces" then
        out[key] = value
      end
    end
  end
  local encoded, text = pcall(io.json.encode, out)
  if not encoded then return false end
  local opened, file = pcall(io.file, path)
  if not opened then return false end
  return pcall(file.write, file, text)
end

return config
