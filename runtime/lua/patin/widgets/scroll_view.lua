local ui = require("mold.ui")

return ui.component(function(props)
  return ui.Flickable {
    width = props.width or 320,
    height = props.height or 480,
    clip = true,
    content_x = props.content_x or 0,
    content_y = props.content_y or 0,
    content_width = props.content_width or props.width or 320,
    content_height = props.content_height or props.height or 480,
    table.unpack(props.children or {}),
  }
end)
