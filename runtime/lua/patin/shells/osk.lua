local mold = require("mold")
local ui = require("mold.ui")
local theme = require("patin.theme")
local Button = require("patin.widgets.button")

local key_rows = {
  { "AD01", "AD02", "AD03", "AD04", "AD05", "AD06", "AD07", "AD08", "AD09", "AD10" },
  { "AC01", "AC02", "AC03", "AC04", "AC05", "AC06", "AC07", "AC08", "AC09" },
  { "AB01", "AB02", "AB03", "AB04", "AB05", "AB06", "AB07" },
  { "SPCE", "BKSP", "RTRN" },
}

return ui.component(function(props)
  local keymap = mold.xkb.compile {
    rules = props.rules or "",
    model = props.model or "pc105",
    layout = props.layout or "us",
    variant = props.variant or "",
    options = props.options,
  }
  local by_name = {}
  for _, key in ipairs(keymap.keys) do by_name[key.name] = key end
  local children = {}
  for _, names in ipairs(key_rows) do
    local buttons = {}
    for _, name in ipairs(names) do
      local key = by_name[name]
      if key then
        local level = props.shift and 2 or 1
        local symbol = key.layouts[1] and key.layouts[1][level] and key.layouts[1][level][1]
        local label = symbol and symbol.text ~= "" and symbol.text or (symbol and symbol.name or name)
        buttons[#buttons + 1] = Button {
          text = label,
          width = name == "SPCE" and 210 or 42,
          on_clicked = function()
            mold.virtual_keyboard.key(key.evdev_code, true)
            mold.virtual_keyboard.key(key.evdev_code, false)
            if props.on_key then props.on_key(label, key.evdev_code) end
          end,
        }
      end
    end
    children[#children + 1] = ui.Row { spacing = theme.spacing.xs, table.unpack(buttons) }
  end
  return ui.Rect {
    width = props.width or 520,
    height = props.height or 200,
    color = theme.colors.background,
    radius = theme.radius.md,
    ui.Column { spacing = theme.spacing.xs, table.unpack(children) },
  }
end)
