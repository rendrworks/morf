local morf = require("morf")
local ui = require("morf.ui")

morf.surface.width = 520
morf.surface.height = 280
morf.surface.anchors = { top = true, left = true }

local transformed = morf.signal("fluid-transform.transformed", false)

local shape = ui.Rect {
  x = 56,
  y = 72,
  width = 120,
  height = 120,
  transform_origin_x = 0.5,
  transform_origin_y = 0.5,
  radius = function() return transformed:get() and 60 or 12 end,
  color = function() return transformed:get() and "#22d3ee" or "#8b5cf6" end,
  rotation = function() return transformed:get() and 315 or 0 end,
  scale_x = function() return transformed:get() and 1.25 or 1 end,
  scale_y = function() return transformed:get() and 0.82 or 1 end,
  skew_x = function() return transformed:get() and -8 or 0 end,
  translate_x = function() return transformed:get() and 270 or 0 end,
  translate_y = function() return transformed:get() and 18 or 0 end,
  shadow_color = "#80000000",
  shadow_blur = function() return transformed:get() and 24 or 8 end,
  shadow_spread = function() return transformed:get() and 5 or 1 end,
  behavior = {
    radius = { duration = 420, easing = "out_quint" },
    color = { duration = 260, easing = "in_out_quad" },
    rotation = {
      duration = 560,
      easing = "out_back",
      rotation_direction = "shortest",
    },
    scale_x = {
      kind = "spring",
      mass = 1,
      damping = 18,
      stiffness = 190,
      epsilon = 0.001,
    },
    scale_y = {
      kind = "spring",
      mass = 1,
      damping = 18,
      stiffness = 190,
      epsilon = 0.001,
    },
    skew_x = { duration = 380, easing = "out_cubic" },
    translate_x = {
      kind = "spring",
      mass = 1,
      damping = 20,
      stiffness = 150,
      epsilon = 0.001,
    },
    translate_y = {
      kind = "spring",
      mass = 1,
      damping = 20,
      stiffness = 150,
      epsilon = 0.001,
    },
    shadow_blur = { duration = 300, easing = "out_cubic" },
    shadow_spread = { duration = 300, easing = "out_cubic" },
  },
  ui.MouseArea {
    anchors = { fill = true },
    on_clicked = function()
      local ok, error = transformed:set(not transformed:get())
      assert(ok, error)
    end,
  },
}

ui.Item {
  width = 520,
  height = 280,
  ui.Rect {
    width = 520,
    height = 280,
    color = "#111827",
  },
  shape,
  ui.Text {
    x = 32,
    y = 22,
    width = 456,
    text = "Click the shape: square ↔ circle",
    color = "#f8fafc",
    font_size = 20,
    font_weight = 700,
  },
  ui.Text {
    x = 32,
    y = 230,
    width = 456,
    text = function()
      if transformed:get() then
        return "origin-aware scale + skew + rotation + spring translation"
      end
      return "all motion is ticked and interpolated in Rust"
    end,
    color = "#94a3b8",
    font_size = 14,
  },
}
