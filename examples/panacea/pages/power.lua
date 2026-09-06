-- The power menu: sleep, lock, log out, restart, shut down. Everything but
-- the lock asks for a second press within a few seconds.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
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
  { id = "sleep", glyph = "󰒲", label = "Sleep", hint = "Suspend to memory", accent = morf.color "#38bdf8", run = function() system.launch({ "systemctl", "suspend" }) end },
  { id = "lock", glyph = "󰌾", label = "Lock", hint = "Keep everything running", accent = morf.color "#a78bfa", instant = true, run = function() require("island").lock() end },
  { id = "logout", glyph = "󰗽", label = "Log out", hint = "End the session", accent = C.warn, run = function() hypr.eval("hl.dispatch(hl.dsp.exit())") end },
  { id = "reboot", glyph = "󰜉", label = "Restart", hint = "Reboot the machine", accent = morf.color "#fb923c", run = function() system.launch({ "systemctl", "reboot" }) end },
  { id = "poweroff", glyph = "󰐥", label = "Shut down", hint = "Power off", accent = C.crit, run = function() system.launch({ "systemctl", "poweroff" }) end },
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
  local W = theme.page_w()
  local rows = {}
  for index, action in ipairs(ACTIONS) do
    rows[index] = kit.row {
      width = W,
      icon = action.glyph, accent = action.accent, tint = action.accent:alpha(0.25), edge = action.accent:alpha(0.6),
      title = action.label,
      subtitle = function() return page.armed:get() == index and "Once more to confirm" or action.hint end,
      active = function() return page.armed:get() == index end,
      on_click = function() trigger(index, island) end,
    }
  end
  return kit.page(W, rows)
end

return page
