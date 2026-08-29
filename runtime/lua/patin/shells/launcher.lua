local mold = require("mold")
local ui = require("mold.ui")
local theme = require("patin.theme")
local ListItem = require("patin.widgets.list_item")

return ui.component(function(props)
  local model = props.model or mold.list_model({})
  return ui.Rect {
    width = props.width or 360,
    height = props.height or 560,
    radius = theme.radius.lg,
    color = theme.colors.background,
    visible = props.visible == nil and true or props.visible,
    ui.Text {
      x = theme.spacing.lg,
      y = theme.spacing.lg,
      text = props.title or "Applications",
      font_size = theme.type.title,
      color = theme.colors.text,
    },
    ui.ListView {
      x = theme.spacing.lg,
      y = 64,
      width = (props.width or 360) - theme.spacing.lg * 2,
      height = (props.height or 560) - 84,
      item_extent = 56,
      overscan = 1,
      model = model,
      delegate = function(item)
        local text = type(item) == "table" and (item.name or item.title) or item
        return ListItem { text = text or "Application", width = 320 }
      end,
    },
  }
end)
