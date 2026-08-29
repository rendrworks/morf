local mold = require("mold")
local ui = require("mold.ui")

return ui.component(function(props)
  local pressed = mold.signal("patin.button.pressed", false)
  return ui.Item {
    x = props.x or 0,
    y = props.y or 0,
    width = props.width or 100,
    height = props.height or 28,
    ui.Rect {
      anchors = { fill = true },
      radius = props.radius or 6,
      color = function()
        if pressed:get() then return props.pressed_color or "#4c566a" end
        return props.color or "#3b4252"
      end,
      ui.Text {
        x = props.padding or 10,
        y = 5,
        text = props.text or "Button",
        color = props.text_color or "#eceff4",
      },
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_pressed = function() pressed:set(true) end,
      on_released = function() pressed:set(false) end,
      on_exited = function() pressed:set(false) end,
      on_clicked = props.on_clicked,
    },
  }
end)
