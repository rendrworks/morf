local ui = require("mold.ui")
local Network = require("patin.services.network")

return ui.component(function(props)
  local network = props.service or Network.new()
  return ui.Text {
    color = props.color or "#eceff4",
    text = function()
      if not network.networking_enabled() then return "offline" end
      if network.wireless_enabled() then return "wifi" end
      return "network"
    end,
  }
end)
