local ui = require("mold.ui")
local theme = require("patin.theme")

local function read(value)
  if type(value) == "function" then return value() end
  return value or 0
end

return ui.component(function(props)
  local width = props.width or 220
  return ui.MouseArea {
    width = width,
    height = props.height or 28,
    on_pressed = props.on_pressed or function() end,
    on_released = props.on_released or function() end,
    ui.Rect {
      y = 11,
      width = width,
      height = 6,
      radius = 3,
      color = theme.colors.muted,
      ui.Rect {
        width = function() return width * math.max(0, math.min(1, read(props.value))) end,
        height = 6,
        radius = 3,
        color = theme.colors.primary,
        behavior = { width = { duration = 100, easing = "out_cubic" } },
      },
    },
  }
end)
