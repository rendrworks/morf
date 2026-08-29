local mold = require("mold")
local ui = require("mold.ui")

return ui.component(function(props)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return mold.clock:get() end,
  }
end)
