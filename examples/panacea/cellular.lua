-- The modem, when there is one: signal strength, the generation (5G, LTE,
-- 3G, 2G) and the operator, from ModemManager on the system bus. A machine
-- without a modem -- most laptops -- leaves `state.present` false and the
-- status icon stays away.

local morf = require("morf")

local cellular = {}

cellular.state = morf.state { present = false, quality = 0, tech = "", operator = "", connected = false }
local state = cellular.state

-- MM_MODEM_ACCESS_TECHNOLOGY_*, as bits, best first.
local TECHS = {
  { bit = 15, label = "5G" }, { bit = 14, label = "LTE" }, { bit = 16, label = "LTE" }, { bit = 17, label = "LTE" },
  { bit = 9, label = "H+" }, { bit = 8, label = "H" }, { bit = 7, label = "H" }, { bit = 6, label = "H" },
  { bit = 5, label = "3G" }, { bit = 13, label = "3G" }, { bit = 12, label = "3G" }, { bit = 11, label = "3G" },
  { bit = 4, label = "E" }, { bit = 3, label = "G" }, { bit = 10, label = "1x" }, { bit = 1, label = "2G" }, { bit = 2, label = "2G" },
}
local MM_MODEM_STATE_CONNECTED = 11

local function tech_label(mask)
  mask = tonumber(mask) or 0
  for _, tech in ipairs(TECHS) do
    if math.floor(mask / (2 ^ tech.bit)) % 2 == 1 then return tech.label end
  end
  return ""
end

local manager = nil
do
  local ok, proxy = pcall(morf.dbus.proxy, "system", "org.freedesktop.ModemManager1",
    "/org/freedesktop/ModemManager1", "org.freedesktop.DBus.ObjectManager", 1500)
  if ok then manager = proxy end
end

local function poll()
  if not manager then return end
  local ok, reply = pcall(manager.call, manager, "GetManagedObjects")
  if not ok or type(reply) ~= "table" or type(reply[1]) ~= "table" then
    state.present = false
    return
  end
  local found = false
  for _, interfaces in pairs(reply[1]) do
    local modem = type(interfaces) == "table" and interfaces["org.freedesktop.ModemManager1.Modem"]
    if type(modem) == "table" then
      found = true
      local quality = modem.SignalQuality
      if type(quality) == "table" then quality = quality[1] end
      state.quality = tonumber(quality) or 0
      state.tech = tech_label(modem.AccessTechnologies)
      state.connected = (tonumber(modem.State) or 0) >= MM_MODEM_STATE_CONNECTED
      local gpp = interfaces["org.freedesktop.ModemManager1.Modem.Modem3gpp"]
      state.operator = type(gpp) == "table" and tostring(gpp.OperatorName or "") or ""
      break
    end
  end
  state.present = found
end

if manager then
  poll()
  morf.timer(10000, poll, true)
end

--- The bars glyph for the quality now.
function cellular.glyph()
  if not state.present then return "" end
  local q = state.quality
  if q <= 5 then return "󰤬" end
  if q < 25 then return "󰤟" end
  if q < 50 then return "󰤢" end
  if q < 75 then return "󰤥" end
  return "󰤨"
end

return cellular
