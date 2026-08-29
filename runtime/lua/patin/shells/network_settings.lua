local ui = require("mold.ui")
local theme = require("patin.theme")
local Toggle = require("patin.widgets.toggle")

return ui.component(function(props)
  return ui.Rect {
    width = props.width or 360,
    height = props.height or 520,
    radius = theme.radius.lg,
    color = theme.colors.background,
    ui.Column {
      x = theme.spacing.lg,
      y = theme.spacing.lg,
      spacing = theme.spacing.lg,
      ui.Text { text = "Network", font_size = theme.type.title, color = theme.colors.text },
      ui.Row {
        spacing = theme.spacing.lg,
        ui.Text { text = "Wi-Fi", color = theme.colors.text },
        Toggle { checked = props.wifi_enabled, on_toggled = props.on_wifi_toggled },
      },
      table.unpack(props.networks or {}),
    },
  }
end)
