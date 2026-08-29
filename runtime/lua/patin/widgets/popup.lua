local ui = require("mold.ui")

return ui.component(function(props)
  return ui.Item {
    x = props.x or 0,
    y = props.y or 0,
    width = props.width or 280,
    height = props.height or 320,
    visible = props.visible == nil and true or props.visible,
    table.unpack(props.children or {}),
  }
end)
