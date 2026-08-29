local ui = require("mold.ui")
local theme = require("patin.theme")

return ui.component(function(props)
  return ui.Rect {
    width = props.width or 280,
    radius = theme.radius.md,
    color = theme.colors.surface,
    ui.Column {
      spacing = theme.spacing.xs,
      table.unpack(props.children or {}),
    },
  }
end)
