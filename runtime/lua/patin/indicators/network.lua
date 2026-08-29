local mold = require("mold")
local ui = require("mold.ui")
local Network = require("patin.services.network")

return ui.component(function(props)
  local network = props.service or Network.new()
  local text = mold.signal("patin.network.text", "offline")
  local function refresh()
    local enabled, networking = pcall(network.networking_enabled)
    if not enabled or not networking then text:set("offline"); return end
    local wireless, active = pcall(network.wireless_enabled)
    text:set(wireless and active and "wifi" or "network")
  end
  refresh()
  if network.watch then pcall(network.watch, network, refresh) end
  mold.timer(props.interval or 5000, refresh)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return text:get() end,
  }
end)
