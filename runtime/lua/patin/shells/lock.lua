local ui = require("mold.ui")
local theme = require("patin.theme")
local TextField = require("patin.widgets.text_field")

return ui.component(function(props)
  return ui.Rect {
    width = props.width or 720,
    height = props.height or 1280,
    color = theme.colors.background,
    ui.Column {
      x = props.form_x or 220,
      y = props.form_y or 460,
      spacing = theme.spacing.lg,
      ui.Text {
        text = props.clock or "Locked",
        font_size = 48,
        color = theme.colors.text,
      },
      TextField {
        text = props.password or "",
        placeholder = "Password",
        on_key_pressed = props.on_key_pressed,
      },
      ui.Text {
        text = props.message or "",
        color = theme.colors.muted,
      },
    },
  }
end)
