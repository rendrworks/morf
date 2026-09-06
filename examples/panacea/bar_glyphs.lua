-- Glyphs for the machine's state: which battery, which speaker, which
-- network icon fits what `system` reports. Material Design icons, from the
-- Nerd Font.

local system = require("system")

local glyphs = {}
local state = system.state

local BATTERY = { "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹" }
local WIFI = { "󰤯", "󰤟", "󰤢", "󰤥", "󰤨" }

function glyphs.battery_glyph()
  if state.battery.charging then return "󰂄" end
  local percent = state.battery.percent
  local index = math.max(1, math.min(10, math.floor(percent / 10 + 0.5)))
  return BATTERY[index]
end

function glyphs.volume_glyph()
  if state.volume.muted or state.volume.level <= 0 then return "󰝟" end
  if state.volume.level < 0.34 then return "󰕿" end
  if state.volume.level < 0.67 then return "󰖀" end
  return "󰕾"
end

function glyphs.network_glyph()
  local network = state.network
  if network.kind == "wifi" then
    local index = math.max(1, math.min(5, math.ceil(network.strength / 20)))
    return WIFI[index]
  elseif network.kind == "wired" then
    return "󰈀"
  end
  return "󰤮"
end

return glyphs
