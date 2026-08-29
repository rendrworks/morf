local mold = require("mold")

local Network = {}

function Network.new()
  local proxy = mold.dbus.proxy(
    "system",
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
    "org.freedesktop.NetworkManager"
  )
  return {
    state = function() return proxy:get("State") end,
    connectivity = function() return proxy:get("Connectivity") end,
    wireless_enabled = function() return proxy:get("WirelessEnabled") end,
    networking_enabled = function() return proxy:get("NetworkingEnabled") end,
  }
end

return Network
