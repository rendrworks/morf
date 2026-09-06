-- ChillPill, on morf.
--
-- A pill bar for Hyprland with a control center, notifications, an OSD, an
-- app launcher, a mini dashboard, a clipboard manager and a wallpaper
-- switcher -- the shell at github.com/LUCKYS1NGHH/ChillPill-Shell, drawn by
-- morf instead of Quickshell.
--
-- Run it:
--
--   EXAMPLE=examples/chillpill/init.lua oslo make run
--
-- The pill and every panel that takes only the pointer live on one
-- fullscreen layer surface whose input region is exactly the parts that can
-- be clicked. The launcher, clipboard manager and wallpaper switcher each
-- get a layer surface of their own with exclusive keyboard focus, opened
-- and closed from here. Bind keys to the IPC verbs:
--
--   morf ipc call launcher      morf ipc call cliphist
--   morf ipc call control       morf ipc call dashboard
--   morf ipc call wallpapers    morf ipc call wifi | bluetooth
--   morf ipc call toggle <name> morf ipc call close
--
-- Settings are read from ~/.config/chillpill/config.json; see config.lua.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local proc = require("proc")
local system = require("system")
local bar = require("bar")
local hypr = require("hypr")
local notify = require("notify")
local osd = require("osd")
local control = require("control")
local dashboard = require("dashboard")
local mediapopup = require("mediapopup")
local field = require("field")

morf.surface.namespace = "chillpill"
morf.surface.width = theme.WIDTH
morf.surface.height = theme.HEIGHT
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = "top"
morf.surface.keyboard_focus = "none"
-- The overlay itself pushes nothing; the reservation below is what keeps
-- windows out from under the pill.
morf.surface.exclusive_zone = -1
morf.surface.reserve = { top = config.pillOnHover and 0 or bar.zone() }

local S = theme.S
local TOP = bar.zone() - S(config.pillBottomMargin) + S(14)

-- ------------------------------------------------------------------ panels --

-- Which panel is open. One at a time, as in the original: opening the
-- dashboard closes the control center.
local panel = morf.signal("chillpill.panel", core.env("CHILLPILL_OPEN") or "")

local function toggle(name)
  panel:set(panel:get() == name and "" or name)
end

bar.open.control = function() toggle("control") end
bar.open.dashboard = function() toggle("dashboard") end
bar.open.wifi = function()
  if panel:get() ~= "control" then toggle("control") end
  control.wifi_open:set(not control.wifi_open:get())
  if control.wifi_open:get() then control.scan_wifi() end
end
bar.open.bluetooth = function()
  if panel:get() ~= "control" then toggle("control") end
  control.bluetooth_open:set(not control.bluetooth_open:get())
  if control.bluetooth_open:get() then control.refresh_devices() end
end
bar.open.weather = function() toggle("dashboard") end

control.osd = osd.show

-- Each panel's `shown` follows the one signal.
local function sync_panels()
  local open = panel:get() == "control"
  if control.shown:get() ~= open then
    control.shown:set(open)
    if open then control.refresh_devices() end
  end
  dashboard.shown:set(panel:get() == "dashboard")
end
sync_panels()
morf.timer(50, sync_panels, true)

-- --------------------------------------------------------------------- IPC --

-- Every output runs this file once, and a verb from the terminal reaches
-- all of them. The instance on the focused monitor is the one that acts;
-- the others answer nothing. A pin (`MORF_MONITOR`) makes that one the
-- pinned output instead, which is what the pill follows too.
local own_output = ((morf.screens or {})[1] or {}).name or ""
local function focused()
  return hypr.state.monitor == "" or hypr.state.monitor == own_output
end

--- Wraps a verb so only the focused output's instance runs it.
local function here(verb)
  return function(...)
    if not focused() then return end
    return verb(...)
  end
end

-- The keyboard surfaces, filled in below; `close` shuts every one.
local closers = {}

morf.ipc.control = here(function() toggle("control") return panel:get() end)
morf.ipc.wifi = here(function() bar.open.wifi() return control.wifi_open:get() end)
morf.ipc.bluetooth = here(function() bar.open.bluetooth() return control.bluetooth_open:get() end)
morf.ipc.dashboard = here(function() toggle("dashboard") return panel:get() end)
morf.ipc.toggle = here(function(name)
  if name == "control" or name == "controlCenter" then toggle("control")
  elseif name == "dashboard" or name == "miniDashboard" then toggle("dashboard")
  end
  return panel:get()
end)
-- Every instance closes; a panel may be open on any of them.
morf.ipc.close = function()
  panel:set("")
  for _, close in ipairs(closers) do close() end
  return "closed"
end
morf.ipc.osd = here(function(kind, message) osd.show(kind or "volume", message) return "shown" end)

-- ------------------------------------------------------------------- scene --

-- `CHILLPILL_ONLY=bar|control|notify|osd` draws one part, for a look at
-- it off-screen with frame_bench.
local only = core.env("CHILLPILL_ONLY") or ""
local function part(name, build, ...)
  if only ~= "" and only ~= name then return ui.Item { width = 1, height = 1 } end
  return build(...)
end

ui.Item {
  width = theme.WIDTH,
  height = theme.HEIGHT,
  -- A click on the desktop closes whatever is open. Under everything, and
  -- only there while a panel is open, so the desktop is not blocked.
  ui.MouseArea {
    anchors = { fill = true },
    z = -10,
    visible = function() return panel:get() ~= "" end,
    on_clicked = function() panel:set("") end,
  },
  part("control", control.build, TOP),
  part("dashboard", dashboard.build, TOP),
  part("notify", notify.build, TOP),
  part("media", mediapopup.build, TOP),
  part("osd", osd.build),
  part("bar", bar.build),
  -- Child processes and the compositor's sockets report on this tick.
  ui.Timer {
    interval = 40, running = true, ["repeat"] = true,
    on_triggered = function()
      proc.tick()
      hypr.tick()
      osd.tick()
      mediapopup.tick()
    end,
  },
  ui.Timer {
    interval = 500, running = true, ["repeat"] = true,
    on_triggered = control.timer_tick,
  },
}

-- ------------------------------------------------------------------ prompt --

-- A password, asked in a surface of its own so it can take the keyboard.
-- Made after the scene, so the scene's root is the first root there is.
local prompt_window
local prompt_title = morf.signal("chillpill.prompt.title", "")
local prompt_answer = nil
local prompt_field = field.new {
  placeholder = "password",
  secret = true,
  on_submit = function(text)
    local answer = prompt_answer
    prompt_answer = nil
    prompt_window:close()
    if answer then answer(text) end
  end,
  on_escape = function()
    prompt_answer = nil
    prompt_window:close()
  end,
}

prompt_window = morf.window.layer {
  namespace = "chillpill-prompt",
  layer = "overlay",
  keyboard_focus = "exclusive",
  width = S(420),
  height = S(150),
  visible = false,
  root = theme.box {
    width = S(420), height = S(150), radius = 28,
    ui.Column {
      gap = S(14),
      anchors = { left = true, top = true, margins = S(24) },
      theme.text { text = function() return prompt_title:get() end, size = 16, font_weight = 700 },
      field.node(prompt_field, { width = S(372), height = S(52) }),
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_key_pressed = function(keysym, text) prompt_field.handle(keysym, text) end,
    },
  },
}

control.prompt = function(title, answer)
  prompt_title:set(title)
  prompt_answer = answer
  prompt_field.clear()
  prompt_window:open()
end

-- ------------------------------------------------------------- keyboard --

-- The launcher, the clipboard manager and the wallpaper switcher: each a
-- surface of its own with the keyboard, made after the scene like the
-- prompt so the scene's root stays first.
local launcher = require("launcher")
local cliphist = require("cliphist")
local wallpapers = require("wallpapers")
closers = { launcher.close, cliphist.close, wallpapers.close }

local function solo(open_one)
  return function()
    for _, close in ipairs(closers) do close() end
    panel:set("")
    return open_one()
  end
end

morf.ipc.launcher = here(solo(launcher.toggle))
morf.ipc.cliphist = here(solo(cliphist.toggle))
morf.ipc.wallpapers = here(solo(wallpapers.toggle))
local plain_toggle = morf.ipc.toggle
morf.ipc.toggle = function(name)
  if name == "launcher" or name == "appLauncher" then return morf.ipc.launcher() end
  if name == "cliphist" or name == "clipboard" then return morf.ipc.cliphist() end
  if name == "wallpapers" or name == "wallpaperSwitcher" then return morf.ipc.wallpapers() end
  return plain_toggle(name)
end

-- The bandwidth alert: once, when this session has pulled more than the
-- config's number of megabytes.
if config.bandwidthAlertMB > 0 then
  local base = system.state.traffic.rx
  local said = false
  morf.timer(10000, function()
    if said then return end
    local used = (system.state.traffic.rx - base) / 1024 / 1024
    if used >= config.bandwidthAlertMB then
      said = true
      notify.local_notice("Bandwidth Alert", "Bandwidth Alert",
        string.format("Download usage exceeded %d MB", config.bandwidthAlertMB), "󰢾")
    end
  end, true)
end

