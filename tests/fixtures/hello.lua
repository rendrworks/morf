local mold = require("mold")
local ui = require("mold.ui")
local Button = require("patin.widgets.button")
local clicks = mold.signal("clicks", 0)

ui.Rect {
  color = "#1f2430",
  ui.Text {
    x = 12,
    y = 5,
    text = function() return mold.clock:get() end,
    color = "#ffffff",
    font_size = 18,
  },
  Button {
    x = 120,
    y = 2,
    text = function() return "Clicks " .. clicks:get() end,
    on_clicked = function() clicks:set(clicks:get() + 1) end,
  },
}
