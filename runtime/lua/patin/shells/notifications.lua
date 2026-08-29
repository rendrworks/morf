local mold = require("mold")
local ui = require("mold.ui")
local theme = require("patin.theme")
local Card = require("patin.widgets.card")

return ui.component(function(props)
  local model = props.model or mold.list_model({})
  return ui.ListView {
    width = props.width or 340,
    height = props.height or 520,
    item_extent = 104,
    overscan = 1,
    model = model,
    delegate = function(item)
      local text = type(item) == "table" and (item.summary or item.title) or item
      return Card {
        width = props.width or 340,
        height = 96,
        children = {
          ui.Text { x = theme.spacing.md, y = theme.spacing.md, text = text or "Notification", color = theme.colors.text },
        },
      }
    end,
  }
end)
