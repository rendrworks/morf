local ui = require("mold.ui")
local theme = require("patin.theme")

local function read(value)
  if type(value) == "function" then return value() end
  return value
end

return ui.component(function(props)
  return ui.MouseArea {
    width = props.width or 52,
    height = props.height or 30,
    on_clicked = props.on_toggled or function() end,
    ui.Rect {
      width = props.width or 52,
      height = props.height or 30,
      radius = 15,
      color = function()
        return read(props.checked) and theme.colors.primary or theme.colors.muted
      end,
      ui.Rect {
        y = 3,
        x = function() return read(props.checked) and 25 or 3 end,
        width = 24,
        height = 24,
        radius = 12,
        color = theme.colors.text,
        behavior = { x = { duration = 160, easing = "out_cubic" } },
      },
    },
  }
end)
