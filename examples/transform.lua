local mold = require("mold")
local core = require("mold.core")
local ui = require("mold.ui")
local window = require("mold.window")

mold.surface.width = 320
mold.surface.height = 120
mold.surface.anchors = { top = true, left = true }

local revision = mold.signal("transform.revision", 0)
local shifted = false
local target = ui.Rect {
  x = 16,
  y = 16,
  width = 48,
  height = 48,
  radius = 8,
  color = "#7c3aed",
}
local root = ui.Item {
  target,
  ui.Text {
    x = 16,
    y = 80,
    text = function() return "transform revision " .. revision:get() end,
  },
  ui.Timer {
    interval = 1000,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      shifted = not shifted
      target.x = shifted and 240 or 16
    end,
  },
}

core.transform_watcher {
  a = root,
  b = target,
  common_parent = root,
  on_changed = function(value) revision:set(value) end,
}

local popup_root = ui.Rect {
  width = 160,
  height = 60,
  radius = 8,
  color = "#20242c",
}
window.popup {
  root = popup_root,
  visible = false,
  width = 160,
  height = 60,
  anchor = { node = target, margin = 4 },
}

return root
