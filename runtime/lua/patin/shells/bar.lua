local ui = require("mold.ui")
local theme = require("patin.theme")

return ui.component(function(props)
  return ui.Rect {
    width = props.width or 720,
    height = props.height or 36,
    color = theme.colors.background,
    ui.Row {
      x = theme.spacing.sm,
      y = theme.spacing.sm,
      spacing = theme.spacing.md,
      table.unpack(props.left or {}),
    },
    ui.Row {
      x = props.right_x or 480,
      y = theme.spacing.sm,
      spacing = theme.spacing.md,
      table.unpack(props.right or {}),
    },
  }
end)
