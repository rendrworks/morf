local ui = require("mold.ui")
local theme = require("patin.theme")

return ui.component(function(props)
  return ui.MouseArea {
    width = props.width or 260,
    height = props.height or 44,
    on_key_pressed = props.on_key_pressed or function() end,
    ui.Rect {
      width = props.width or 260,
      height = props.height or 44,
      radius = theme.radius.sm,
      color = theme.colors.surface,
      border_width = 1,
      border_color = props.border_color or theme.colors.muted,
      ui.Text {
        x = theme.spacing.md,
        y = theme.spacing.md,
        text = props.text or props.placeholder or "",
        color = props.text and theme.colors.text or theme.colors.muted,
      },
    },
  }
end)
