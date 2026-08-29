local ui = require("mold.ui")
local theme = require("patin.theme")

return ui.component(function(props)
  local label = ui.Text {
    x = theme.spacing.md,
    y = theme.spacing.md,
    text = props.text or "",
    color = theme.colors.text,
  }
  if props.label_ref then props.label_ref(label) end
  return ui.MouseArea {
    width = props.width or 320,
    height = props.height or 52,
    on_clicked = props.on_clicked or function() end,
    ui.Rect {
      width = props.width or 320,
      height = props.height or 52,
      radius = theme.radius.sm,
      color = props.color or theme.colors.surface,
      label,
    },
  }
end)
