-- The power menu: sleep, lock, log out, restart, shut down. Everything but
-- the lock asks for a second press within a few seconds.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local hypr = require("hypr")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Power"
page.icon = "󰐥"

page.armed = morf.signal("panacea.power.armed", 0)
page.current = morf.signal("panacea.power.current", 1)
local armed_clock = morf.elapsed_timer()

local ACTIONS = {
  { id = "sleep", glyph = "󰒲", label = "Sleep", accent = morf.color "#38bdf8", run = function() system.launch({ "systemctl", "suspend" }) end },
  { id = "lock", glyph = "󰌾", label = "Lock", accent = morf.color "#a78bfa", instant = true, run = function() require("island").lock() end },
  { id = "logout", glyph = "󰗽", label = "Log out", accent = C.warn, run = function() hypr.eval("hl.dispatch(hl.dsp.exit())") end },
  { id = "reboot", glyph = "󰜉", label = "Restart", accent = morf.color "#fb923c", run = function() system.launch({ "systemctl", "reboot" }) end },
  { id = "poweroff", glyph = "󰐥", label = "Shut down", accent = C.crit, run = function() system.launch({ "systemctl", "poweroff" }) end },
}

local function trigger(index, island)
  local action = ACTIONS[index]
  if not action then return end
  if not action.instant and (page.armed:get() ~= index or armed_clock:elapsed_ms() > 2600) then
    page.armed:set(index)
    page.current:set(index)
    armed_clock:restart()
    return
  end
  page.armed:set(0)
  island.close()
  action.run()
end

morf.timer(300, function()
  if page.armed:get() ~= 0 and armed_clock:elapsed_ms() > 2600 then page.armed:set(0) end
end, true)

function page.on_key(keysym)
  local island = require("island")
  if keysym == 0xff51 then page.current:set(math.max(1, page.current:get() - 1)) page.armed:set(0) return true end
  if keysym == 0xff53 then page.current:set(math.min(#ACTIONS, page.current:get() + 1)) page.armed:set(0) return true end
  if keysym == 0xff0d or keysym == 0xff8d then trigger(page.current:get(), island) return true end
  return false
end

function page.on_open()
  page.armed:set(0)
  page.current:set(1)
end

page.subtitle = function()
  local armed = page.armed:get()
  if armed ~= 0 then return "Once more to confirm: " .. ACTIONS[armed].label end
  return "Sleep, lock, log out, restart or shut down"
end

function page.build(island)
  local W = S(config.panelW) - S(32)
  local cell = math.floor((W - S(8) * (#ACTIONS - 1)) / #ACTIONS)
  local buttons = {}
  for index, action in ipairs(ACTIONS) do
    buttons[index] = theme.button {
      width = cell, height = S(84),
      color = function()
        if page.armed:get() == index then return action.accent:alpha(0.3) end
        return page.current:get() == index and C.card_hover or C.card
      end,
      hover_color = function() return page.armed:get() == index and action.accent:alpha(0.3) or C.card_hover end,
      border_width = 1,
      border_color = function() return page.armed:get() == index and action.accent:alpha(0.7) or C.edge end,
      on_click = function() trigger(index, island) end,
      ui.Column {
        gap = S(8), anchors = { center_in = true },
        ui.Item { width = cell - S(10), height = S(26),
          theme.icon { text = action.glyph, size = config.iconSize + 4, anchors = { center_in = true }, color = action.accent } },
        ui.Item { width = cell - S(10), height = S(16),
          theme.text { text = action.label, size = config.fontSize - 3, anchors = { center_in = true },
            color = function() return page.armed:get() == index and C.fg or C.muted end } },
      },
    }
  end
  return ui.Column {
    gap = S(12),
    ui.Row { gap = S(8), table.unpack(buttons) },
  }
end

return page
