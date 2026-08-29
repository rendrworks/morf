local mold = require("mold")
local ui = require("mold.ui")
local Brightness = require("patin.services.brightness")

return ui.component(function(props)
  local brightness = props.service or Brightness.new(props.value_path, props.maximum_path)
  local text = mold.signal("patin.brightness.text", "--")
  local function refresh()
    local ok, level = pcall(brightness.level)
    if ok then text:set(string.format("%.0f%%", level * 100)) end
  end
  refresh()
  mold.timer(props.interval or 5000, refresh)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return text:get() end,
  }
end)
