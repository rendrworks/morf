local ui = require("mold.ui")
local theme = require("patin.theme")

return ui.component(function(props)
  return ui.Rect {
    width = props.width or 240,
    height = props.height or 96,
    radius = props.radius or theme.radius.md,
    color = props.color or theme.colors.surface,
    table.unpack(props.children or {}),
  }
end)
