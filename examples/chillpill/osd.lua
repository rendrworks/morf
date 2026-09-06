-- The on-screen display: one pill near the bottom that appears when the
-- volume, brightness or battery changes, or a timer ends, and goes again
-- after `osdDuration`.
--
-- An icon, a bar, a number. The same node serves every kind; only what it
-- shows changes, so a volume key followed by a brightness key crossfades
-- rather than stacking two pills.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local bar = require("bar")

local S = theme.S
local C = theme.color
local state = system.state

local osd = {}

osd.shown = morf.signal("chillpill.osd.shown", false)
osd.kind = morf.signal("chillpill.osd.kind", "volume")
osd.message = morf.signal("chillpill.osd.message", "")

local hide_clock = morf.elapsed_timer()
local hide_after = 0

--- Shows the pill for one kind, restarting the countdown.
function osd.show(kind, message)
  osd.kind:set(kind)
  osd.message:set(message or "")
  osd.shown:set(true)
  hide_after = config.osdDuration
  if kind == "battery" or kind == "timer" then hide_after = config.osdDuration * 3 end
  hide_clock:restart()
end

-- Watched values: a change after the first read shows the pill. The first
-- read is the poll filling in, not the person pressing a key.
local seen = { volume = nil, muted = nil, brightness = nil, battery_low = nil, charging = nil }

local function watch()
  local volume = math.floor(state.volume.level * 100 + 0.5)
  if seen.volume ~= nil and (volume ~= seen.volume or state.volume.muted ~= seen.muted) then
    osd.show("volume")
  end
  seen.volume, seen.muted = volume, state.volume.muted

  local brightness = math.floor(state.brightness.level * 100 + 0.5)
  if seen.brightness ~= nil and brightness ~= seen.brightness then osd.show("brightness") end
  seen.brightness = brightness

  local low = state.battery.present and not state.battery.plugged and state.battery.percent <= 15
  if seen.battery_low ~= nil and low and not seen.battery_low then osd.show("battery") end
  seen.battery_low = low
  if seen.charging ~= nil and state.battery.charging ~= seen.charging then osd.show("battery") end
  seen.charging = state.battery.charging
end

--- Called on the shell's tick: watches values and expires the pill.
function osd.tick()
  watch()
  if osd.shown:get() and hide_clock:elapsed_ms() >= hide_after then osd.shown:set(false) end
end

-- ------------------------------------------------------------------ glyphs --

local function glyph()
  local kind = osd.kind:get()
  if kind == "volume" then return bar.volume_glyph() end
  if kind == "brightness" then
    local level = state.brightness.level
    if level < 0.34 then return "󰃞" end
    if level < 0.67 then return "󰃟" end
    return "󰃠"
  end
  if kind == "battery" then return bar.battery_glyph() end
  if kind == "timer" then return "󱎫" end
  return "󰂚"
end

local function fraction()
  local kind = osd.kind:get()
  if kind == "volume" then return state.volume.muted and 0 or state.volume.level end
  if kind == "brightness" then return state.brightness.level end
  if kind == "battery" then return state.battery.percent / 100 end
  return 1
end

local function label()
  local kind = osd.kind:get()
  if kind == "volume" then
    if state.volume.muted then return "Muted" end
    return string.format("%d%%", math.floor(state.volume.level * 100 + 0.5))
  end
  if kind == "brightness" then return string.format("%d%%", math.floor(state.brightness.level * 100 + 0.5)) end
  if kind == "battery" then return string.format("%d%%", state.battery.percent) end
  return osd.message:get()
end

local function glyph_color()
  local kind = osd.kind:get()
  if kind == "battery" then return bar.battery_color() end
  if kind == "timer" then return C.green end
  return C.text
end

-- ------------------------------------------------------------------- node --

local WIDTH = S(420)
local HEIGHT = S(72)
local BAR_WIDTH = S(180)

function osd.build()
  local pill = theme.box {
    width = WIDTH,
    height = HEIGHT,
    radius = 36,
    opacity = function() return osd.shown:get() and 1 or 0 end,
    translate_y = function() return osd.shown:get() and 0 or S(28) end,
    scale = function() return osd.shown:get() and 1 or 0.94 end,
    behavior = {
      opacity = theme.motion.fade,
      translate_y = theme.motion.spring,
      scale = theme.motion.spring,
    },
    ui.Row {
      gap = S(24),
      align = "center",
      height = HEIGHT,
      anchors = { center_in = true },
      ui.Item {
        width = S(28), height = S(28),
        theme.icon { text = glyph, size = 22, color = glyph_color, anchors = { center_in = true } },
      },
      theme.meter { width = BAR_WIDTH, fraction = fraction, height = S(8) },
      ui.Item {
        width = S(70), height = S(28),
        theme.text { text = label, size = 16, anchors = { center_in = true } },
      },
    },
  }
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "end",
    anchors = { left = true, right = true, bottom = true, bottom_margin = S(96) },
    pill,
  }
end

return osd
