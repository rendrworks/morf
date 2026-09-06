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
--   morf ipc call overview | settings | shortcuts | dnd | smartClose
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

-- The shell's own surface: the collapsed pill and the notification cards.
-- Overlay mode floats over the windows; otherwise the pill's strip is
-- reserved.
morf.surface.namespace = "panacea"
morf.surface.width = theme.WIDTH
morf.surface.height = theme.HEIGHT
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = "top"
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1
if not config.pillOverlay then
  morf.surface.reserve = { top = S(config.pillH) + (config.notchMode and 0 or S(config.islandGap)) }
end

-- ------------------------------------------------------------------ pages --

for _, name in ipairs { "main", "power", "battery", "record", "wifi", "bt", "notif", "clip",
  "launcher", "cal", "audio", "settings", "shortcuts", "overview" } do
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

-- The collapsed pill fades while the expanded surface has the island.
local pill_shown = morf.signal("panacea.pill.shown", true)

-- `PANACEA_OPEN=main` opens a page at start, for a look at it with
-- frame_bench.
local open_at_start = core.env("PANACEA_OPEN") or ""

local scene = {
  width = theme.WIDTH,
  height = theme.HEIGHT,
  island.build_collapsed(pill_shown),
  require("cards").build(),
  ui.Timer {
    interval = 40, running = true, ["repeat"] = true,
    on_triggered = function()
      proc.tick()
      hypr.tick()
    end,
  },
}

-- ----------------------------------------------------------- the expanded --

-- Fullscreen, over everything, with the keyboard: a page closes with
-- Escape or a click anywhere outside the island.
local expanded_island = island.build_expanded()
local expanded_root = ui.Item {
  width = theme.WIDTH,
  height = theme.HEIGHT,
  ui.MouseArea {
    anchors = { fill = true },
    z = -10,
    on_clicked = function() island.close() end,
    on_key_pressed = function(keysym, text) island.handle_key(keysym, text) end,
  },
  expanded_island,
}

if open_at_start ~= "" then
  -- For frame_bench, which draws only the first root: the expanded island
  -- joins the shell's own scene instead of a surface of its own.
  scene[#scene + 1] = expanded_root
  island.surface.open = function() pill_shown:set(false) end
  island.surface.close = function() pill_shown:set(true) end
  island.open(open_at_start)
else
  local expanded_window = morf.window.layer {
    namespace = "panacea-island",
    layer = "overlay",
    keyboard_focus = "exclusive",
    width = theme.WIDTH,
    height = theme.HEIGHT,
    anchors = { top = true, left = true, right = true, bottom = true },
    visible = false,
    root = expanded_root,
  }
  island.surface.open = function()
    pill_shown:set(false)
    expanded_window:open()
  end
  island.surface.close = function()
    expanded_window:close()
    pill_shown:set(true)
  end
end

ui.Item(scene)
