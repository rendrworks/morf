-- Panacea, on morf.
--
-- One pill for everything: the desktop shell at github.com/EnsixD/Panacea,
-- a single capsule at the top edge that morphs into whatever page was
-- asked for -- quick settings, networks, Bluetooth, notifications, the
-- launcher, the clipboard, the calendar, the recorder, the battery, the
-- power menu, sound, the workspaces, the settings and the shortcuts --
-- and flows back when done. Drawn by morf instead of Quickshell.
--
-- Run it:
--
--   EXAMPLE=examples/panacea/init.lua oslo make run
--
-- Bind keys to the verbs Panacea's own bindings use:
--
--   morf ipc call controls | launcher | wifi | bluetooth | notifications
--   morf ipc call clipboard | calendar | record | audio | powermenu
--   morf ipc call overview | settings | shortcuts | media | wallpapers
--   morf ipc call weather | battery | dnd | recordToggle | smartClose
--
-- Settings are read from ~/.config/panacea/settings.json; see config.lua.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local proc = require("proc")
local system = require("system")
local hypr = require("hypr")
local island = require("island")
local notify = require("notify")

local S = theme.S
local C = theme.color

-- The shell's own surface: the island, collapsed or open, and the
-- notification cards under it. While anything moves it is the island's
-- size, not the screen's: every frame the shell paints is cleared,
-- composed and then blended by the compositor at the surface's size, and
-- a fullscreen surface made a small island cost a whole 4K frame on both
-- sides of the protocol. Once a page is open and still, the surface grows
-- to the screen so a click anywhere else can close it, and shrinks again
-- before the page runs back. The island is centred at the top edge either
-- way, so nothing on screen moves when the surface does. Overlay mode
-- floats over the windows; otherwise the pill's strip is reserved.
local SURFACE_W = theme.phone and theme.WIDTH or math.min(theme.WIDTH, S(config.panelW * 1.6) + S(80))
local SURFACE_H = math.min(theme.HEIGHT, S(config.expandedH) + S(160))
morf.surface.namespace = "panacea"
morf.surface.width = SURFACE_W
morf.surface.height = SURFACE_H
morf.surface.anchors = theme.phone and { top = true, left = true, right = true } or { top = true }
morf.surface.layer = "top"
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1
-- Declared here, inert: the backdrop is what hears a click anywhere else
-- while a page is open. It is a blank surface the compositor stretches
-- over the output, never painted, so it costs nothing until then.
morf.surface.backdrop = false
-- Awake, it dims the screen behind the page, as a phone's shade does.
morf.surface.backdrop_dim = 0.3
if not config.pillOverlay then
  morf.surface.reserve = { top = S(config.pillH) + (config.notchMode and 0 or S(config.islandGap)) }
end

-- ------------------------------------------------------------------ pages --

for _, name in ipairs { "main", "shade", "power", "battery", "record", "wifi", "bt", "notif", "clip",
  "launcher", "cal", "audio", "settings", "shortcuts", "overview", "media", "wallpapers", "weather" } do
  local ok, page = pcall(require, "pages." .. name)
  if ok and type(page) == "table" then island.register(name, page) end
end

--- Locks the screen with what the config names, else with the login
--- example's own lock if it is installed, else hyprlock.
function island.lock()
  island.close()
  local command = config.lockCommand
  if command == "" then
    if system.slurp("/usr/bin/logre") then command = "logre -- lock" else command = "hyprlock" end
  end
  system.launch(command)
end

-- --------------------------------------------------------- focused output --

-- Every output runs this file once; the instance on the focused monitor
-- is the one that opens pages.
local own_output = ((morf.screens or {})[1] or {}).name or ""
local function focused()
  -- One output -- a phone, a nested compositor -- is always the one.
  if #(morf.screens or {}) <= 1 then return true end
  return hypr.state.monitor == "" or hypr.state.monitor == own_output
end

-- ------------------------------------------------------------------- IPC --

local function verb(name)
  return function()
    if not focused() then return end
    return island.toggle(name)
  end
end

morf.ipc.controls = verb("main")
morf.ipc.launcher = verb("launcher")
morf.ipc.wifi = verb("wifi")
morf.ipc.bluetooth = verb("bt")
morf.ipc.notifications = verb("notif")
morf.ipc.clipboard = verb("clip")
morf.ipc.calendar = verb("cal")
morf.ipc.record = verb("record")
morf.ipc.audio = verb("audio")
morf.ipc.powermenu = verb("power")
morf.ipc.battery = verb("battery")
morf.ipc.overview = verb("overview")
morf.ipc.settings = verb("settings")
morf.ipc.shortcuts = verb("shortcuts")
morf.ipc.media = verb("media")
morf.ipc.theme = verb("wallpapers")
morf.ipc.wallpapers = verb("wallpapers")
morf.ipc.weather = verb("weather")
morf.ipc.dnd = function()
  notify.silent:set(not notify.silent:get())
  return notify.silent:get()
end
morf.ipc.recordToggle = function()
  if not focused() then return end
  local record = require("pages.record")
  if record.state.running then record.stop() else record.start() end
  return record.state.running
end
morf.ipc.smartClose = function()
  if island.page:get() ~= "" then
    island.close()
    return "closed"
  end
  hypr.eval("hl.dispatch(hl.dsp.window.close())")
  return "window"
end
morf.ipc.close = function() island.close() return "closed" end
morf.ipc.page = function() return island.page:get() end

-- ------------------------------------------------------------------- scene --

-- `PANACEA_OPEN=main` opens a page at start, for a look at it with
-- frame_bench.
local open_at_start = core.env("PANACEA_OPEN") or ""

local function open() return island.page:get() ~= "" end

ui.Item {
  width = SURFACE_W,
  height = SURFACE_H,
  -- While a page is open, every key goes to the page, Escape closes, and
  -- a click beside the island closes too.
  ui.MouseArea {
    anchors = { fill = true },
    z = -10,
    visible = open,
    on_clicked = function() island.close() end,
    on_key_pressed = function(keysym, text) island.handle_key(keysym, text) end,
  },
  island.build(),
  require("cards").build(),
  ui.Timer {
    interval = 40, running = true, ["repeat"] = true,
    on_triggered = function()
      proc.tick()
      hypr.tick()
    end,
  },
}

-- ---------------------------------------------------------------- keyboard --

-- The keyboard is the shell surface's own: it asks for exclusive focus
-- while a page is open and gives it back after. The compositor re-reads
-- the policy on the commit, so a page is one spring away, never a surface
-- away. The backdrop wakes with the page, and a click on it closes.
island.surface.open = function()
  morf.surface.keyboard_focus = "exclusive"
  morf.surface.backdrop = true
end
island.surface.close = function()
  morf.surface.keyboard_focus = "none"
  morf.surface.backdrop = false
end
morf.on_backdrop_click(function() island.close() end)

if open_at_start ~= "" then island.open(open_at_start) end
