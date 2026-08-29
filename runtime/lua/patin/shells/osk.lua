local ui = require("mold.ui")
local theme = require("patin.theme")
local Button = require("patin.widgets.button")

local rows = {
  { "q", "w", "e", "r", "t", "y", "u", "i", "o", "p" },
  { "a", "s", "d", "f", "g", "h", "j", "k", "l" },
  { "z", "x", "c", "v", "b", "n", "m" },
}

return ui.component(function(props)
  local children = {}
  for row_index, keys in ipairs(rows) do
    local buttons = {}
    for _, key in ipairs(keys) do
      buttons[#buttons + 1] = Button {
        text = key,
        width = 42,
        on_clicked = function()
          if props.on_key then props.on_key(key) end
        end,
      }
    end
    children[#children + 1] = ui.Row { spacing = theme.spacing.xs, table.unpack(buttons) }
  end
  return ui.Rect {
    width = props.width or 520,
    height = props.height or 150,
    color = theme.colors.background,
    radius = theme.radius.md,
    ui.Column { spacing = theme.spacing.xs, table.unpack(children) },
  }
end)
