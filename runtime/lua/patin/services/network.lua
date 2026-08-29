local mold = require("mold")

local Network = {}

function Network.new()
  local proxy = mold.dbus.proxy(
    "system",
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
    "org.freedesktop.NetworkManager"
  )
  local properties = mold.dbus.proxy(
    "system",
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
    "org.freedesktop.DBus.Properties"
  )
  return {
    state = function() return proxy:get("State") end,
    connectivity = function() return proxy:get("Connectivity") end,
    wireless_enabled = function() return proxy:get("WirelessEnabled") end,
    networking_enabled = function() return proxy:get("NetworkingEnabled") end,
    watch = function(_, callback)
      properties:subscribe("PropertiesChanged", function(change)
        if change[1] == "org.freedesktop.NetworkManager" then callback(change[2], change[3]) end
      end)
    end,
  }
end

return Network
