local ui = require("mold.ui")
local UPower = require("patin.services.upower")

return ui.component(function(props)
  local battery = props.service or UPower.new()
  return ui.Text {
    color = props.color or "#eceff4",
    text = function()
      return string.format("%.0f%%", battery.percentage())
    end,
  }
end)
