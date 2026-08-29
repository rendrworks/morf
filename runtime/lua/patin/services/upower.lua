local mold = require("mold")

local UPower = {}

function UPower.new()
  local proxy = mold.dbus.proxy(
    "system",
    "org.freedesktop.UPower",
    "/org/freedesktop/UPower/devices/DisplayDevice",
    "org.freedesktop.UPower.Device"
  )
  return {
    percentage = function() return proxy:get("Percentage") end,
    state = function() return proxy:get("State") end,
    time_to_empty = function() return proxy:get("TimeToEmpty") end,
    time_to_full = function() return proxy:get("TimeToFull") end,
  }
end

return UPower
