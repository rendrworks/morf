local mold = require("mold")
local ui = require("mold.ui")
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
    ui.Item {
      x = 220,
      y = 2,
      width = 100,
      height = 28,
      ui.Rect {
        anchors = { fill = true },
        radius = 6,
        color = "#3b4252",
        ui.Text {
          x = 10,
          y = 5,
          text = function() return "Clicks " .. clicks:get() end,
          color = "#eceff4",
        },
      },
      ui.MouseArea {
        anchors = { fill = true },
        on_clicked = function() clicks:set(clicks:get() + 1) end,
      },
    },
  }
end)
