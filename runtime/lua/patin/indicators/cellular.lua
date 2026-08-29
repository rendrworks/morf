local mold = require("mold")
local ui = require("mold.ui")
local Network = require("patin.services.network")

return ui.component(function(props)
  local network = props.service or Network.new()
  local text = mold.signal("patin.cellular.text", "cell")
  local function refresh()
    local ok, state = pcall(network.state)
    text:set(ok and state ~= 20 and "cell" or "offline")
  end
  refresh()
  mold.timer(props.interval or 5000, refresh)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return text:get() end,
  }
end)
