local mold = require("mold")
local ui = require("mold.ui")

mold.surface.width = 620
mold.surface.height = 320
mold.surface.anchors = { top = true, left = true }

local changed = mold.signal("morph-stack.changed", false)

local morph = ui.Shape {
  x = 54,
  y = 86,
  width = 150,
  height = 150,
  morph_from = "square",
  morph_to = "circle",
  morph_progress = function() return changed:get() and 1 or 0 end,
  fill_color = function() return changed:get() and "#22d3ee" or "#8b5cf6" end,
  rotation = function() return changed:get() and 315 or 0 end,
  scale_x = function() return changed:get() and 1.18 or 1 end,
  scale_y = function() return changed:get() and 0.86 or 1 end,
  translate_x = function() return changed:get() and 300 or 0 end,
  behavior = {
    morph_progress = { duration = 520, easing = "in_out_cubic" },
    fill_color = { duration = 300, easing = "in_out_quad" },
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
    translate_x = {
      kind = "spring",
      mass = 1,
      damping = 20,
      stiffness = 150,
      epsilon = 0.001,
    },
  },
  on_clicked = function()
    local ok, error = changed:set(not changed:get())
    assert(ok, error)
  end,
}

ui.Item {
  width = 620,
  height = 320,
  ui.Rect {
    width = 620,
    height = 320,
    color = "#111827",
  },
  morph,
  ui.Image {
    x = 278,
    y = 250,
    width = 42,
    height = 42,
    source = "examples/assets/sdf-star.svg",
    fill_mode = "preserve_aspect_fit",
    distance_field = true,
    distance_field_spread = 10,
    color_overlay = "#fff59e0b",
    rotation = function() return changed:get() and 180 or 0 end,
    behavior = {
      rotation = { duration = 600, easing = "out_cubic" },
    },
  },
  ui.Text {
    x = 34,
    y = 24,
    width = 552,
    text = "Animato timing + Polymorpher geometry + cached SDF mask",
    color = "#f8fafc",
    font_size = 19,
    font_weight = 700,
  },
  ui.Text {
    x = 34,
    y = 278,
    width = 552,
    text = "Click the shape: its topology, transform, color, and motion stay native Rust",
    color = "#94a3b8",
    font_size = 13,
  },
}
