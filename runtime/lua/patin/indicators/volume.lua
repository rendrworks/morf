local ui = require("mold.ui")
local Volume = require("patin.services.volume")

return ui.component(function(props)
  local volume = props.service or Volume.new()
  return ui.Text {
    color = props.color or "#eceff4",
    text = function()
      if volume.muted() then return "mute" end
      return string.format("%.0f%%", volume.level() * 100)
    end,
  }
end)
