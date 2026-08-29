local mold = require("mold")
local ui = require("mold.ui")
local UPower = require("patin.services.upower")

return ui.component(function(props)
  local battery = props.service or UPower.new()
  local text = mold.signal("patin.battery.text", "--")
  local function refresh()
    local ok, percentage = pcall(battery.percentage)
    if ok then text:set(string.format("%.0f%%", percentage)) end
  end
  refresh()
  mold.timer(props.interval or 30000, refresh)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return text:get() end,
  }
end)
