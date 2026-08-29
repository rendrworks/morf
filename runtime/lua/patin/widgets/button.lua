local mold = require("mold")
local ui = require("mold.ui")

return ui.component {
  name = "Button",
  properties = {
    x = { type = "number", default = 0 },
    y = { type = "number", default = 0 },
    width = { type = "number", default = 100 },
    height = { type = "number", default = 28 },
    radius = { type = "number", default = 6 },
    padding = { type = "number", default = 10 },
    text = { type = "string", default = "Button" },
    color = { type = "color", default = "#3b4252" },
    pressed_color = { type = "color", default = "#4c566a" },
    text_color = { type = "color", default = "#eceff4" },
    content = { type = "table", default = {} },
  },
  signals = { "clicked" },
  default_slot = "content",
  build = function(self)
    local pressed = mold.signal("patin.button.pressed", false)
    local content = self.content
    if #content == 0 then
      content = {
        ui.Text {
          x = self:binding("padding"),
          y = 5,
          text = self:binding("text"),
          color = self:binding("text_color"),
        },
      }
    end
    return ui.Item {
      x = self:binding("x"),
      y = self:binding("y"),
      width = self:binding("width"),
      height = self:binding("height"),
      ui.Rect {
        anchors = { fill = true },
        radius = self:binding("radius"),
        color = function()
          if pressed:get() then return self.pressed_color end
          return self.color
        end,
        table.unpack(content),
      },
      ui.MouseArea {
        anchors = { fill = true },
        on_pressed = function() pressed:set(true) end,
        on_released = function() pressed:set(false) end,
        on_exited = function() pressed:set(false) end,
        on_clicked = function() self:emit("clicked") end,
      },
    }
  end,
}
