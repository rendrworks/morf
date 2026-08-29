local mold = require("mold")
local ui = require("mold.ui")
local Button = require("patin.widgets.button")
local clicks = mold.signal("clicks", 0)

mold.variants(mold.screens, function(screen)
  return ui.Rect {
    color = "#1f2430",
    ui.Text {
      x = 12,
      y = 5,
      text = function() return screen.name .. "  " .. mold.clock:get() end,
      color = "#ffffff",
      font_size = 18,
    },
    Button {
      x = 220,
      y = 2,
      text = function() return "Clicks " .. clicks:get() end,
      on_clicked = function() clicks:set(clicks:get() + 1) end,
    },
  }
end)
