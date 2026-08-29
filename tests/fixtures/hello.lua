local mold = require("mold")
local ui = require("mold.ui")

ui.Rect {
  color = "#1f2430",
  ui.Text {
    x = 12,
    y = 5,
    text = function() return mold.clock:get() end,
    color = "#ffffff",
    font_size = 18,
  },
}
