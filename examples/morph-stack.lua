local morf = require("morf")
local ui = require("morf.ui")

morf.surface.width = 820
morf.surface.height = 360
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local active = morf.signal("morph-stack.active", 1)
local hovered = morf.signal("morph-stack.hovered", 0)
local pinned = morf.signal("morph-stack.pinned", true)

local stages = {
  { name = "Plan", detail = "Scope locked and ready", progress = 0.18, color = "#a78bfa" },
  { name = "Build", detail = "Compiling native engine", progress = 0.48, color = "#38bdf8" },
  { name = "Test", detail = "Running the full verify gate", progress = 0.76, color = "#2dd4bf" },
  { name = "Ship", detail = "Release artifact is ready", progress = 1.00, color = "#fbbf24" },
}

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

local function stage_entry(index, x)
  local stage = stages[index]
  return ui.Item {
    x = x,
    y = 76,
    width = 172,
    height = 84,
    ui.Rect {
      anchors = { fill = true },
      radius = 14,
      color = function()
        if active:get() == index then return "#263449" end
        if hovered:get() == index then return "#1d293b" end
        return "#182234"
      end,
      border_width = 1,
      border_color = function()
        return active:get() == index and stage.color or "#334155"
      end,
      behavior = {
        color = { duration = 150, easing = "out_quad" },
        border_color = { duration = 180, easing = "out_quad" },
      },
    },
    ui.Rect {
      x = 16,
      y = 17,
      width = 9,
      height = 9,
      radius = 5,
      color = stage.color,
    },
    ui.Text {
      x = 34,
      y = 12,
      width = 120,
      text = string.format("%02d  %s", index, stage.name),
      color = "#f8fafc",
      font_size = 16,
      font_weight = 700,
    },
    ui.Text {
      x = 16,
      y = 48,
      width = 140,
      text = string.format("%d%%", math.floor(stage.progress * 100)),
      color = "#94a3b8",
      font_size = 13,
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function() write(hovered, index) end,
      on_exited = function() write(hovered, 0) end,
      on_clicked = function() write(active, index) end,
    },
  }
end

ui.Item {
  width = 820,
  height = 360,
  ui.Rect {
    anchors = { fill = true },
    gradient = { angle = 135, stops = { "#0b1220", "#111c31" } },
  },
  ui.MouseArea {
    anchors = { fill = true },
  },
  ui.Text {
    x = 30,
    y = 20,
    width = 420,
    text = "Release pipeline",
    color = "#f8fafc",
    font_size = 23,
    font_weight = 700,
  },
  ui.Text {
    x = 570,
    y = 25,
    width = 145,
    text = function() return morf.clock:get() end,
    horizontal_alignment = "right",
    color = "#94a3b8",
    font_size = 14,
  },
  ui.Rect {
    x = function() return 30 + (active:get() - 1) * 190 end,
    y = 158,
    width = 172,
    height = 3,
    radius = 2,
    color = function() return stages[active:get()].color end,
    shadow_color = function() return stages[active:get()].color end,
    shadow_blur = 10,
    behavior = {
      x = { kind = "spring", mass = 1, damping = 20, stiffness = 180, epsilon = 0.001 },
      color = { duration = 180, easing = "out_quad" },
      shadow_color = { duration = 180, easing = "out_quad" },
    },
  },
  stage_entry(1, 30),
  stage_entry(2, 220),
  stage_entry(3, 410),
  stage_entry(4, 600),
  ui.Rect {
    x = 30,
    y = 184,
    width = 760,
    height = 130,
    radius = 18,
    color = "#131f32",
    border_width = 1,
    border_color = "#26364d",
    shadow_color = "#70000000",
    shadow_blur = 18,
    shadow_offset_y = 6,
  },
  -- The badge is a field, not a path. `morph_progress` interpolates the two
  -- distance fields, so the circle does not merely deform into the star — the
  -- intermediate frames are shapes neither end describes, and the outline is
  -- free to gain or lose a lobe on the way.
  ui.Sdf {
    x = 54,
    y = 211,
    width = 70,
    height = 70,
    fill_color = function() return stages[active:get()].color end,
    scale = function() return hovered:get() == active:get() and 1.12 or 1 end,
    behavior = {
      fill_color = { duration = 240, easing = "out_quad" },
      scale = { kind = "spring", mass = 1, damping = 16, stiffness = 220, epsilon = 0.001 },
    },
    ui.SdfShape {
      width = 70,
      height = 70,
      shape = "circle",
      morph_to = "star",
      points = 8,
      inner_radius = 0.62,
      morph_progress = function() return (active:get() - 1) / 3 end,
      rotation = function() return (active:get() - 1) * 75 end,
      behavior = {
        morph_progress = { duration = 520, easing = "in_out_cubic" },
        rotation = { duration = 560, easing = "out_back", rotation_direction = "shortest" },
      },
    },
  },
  ui.Text {
    x = 148,
    y = 204,
    width = 390,
    text = function() return stages[active:get()].name end,
    color = "#f8fafc",
    font_size = 21,
    font_weight = 700,
  },
  ui.Text {
    x = 148,
    y = 238,
    width = 500,
    text = function() return stages[active:get()].detail end,
    color = "#94a3b8",
    font_size = 14,
  },
  ui.Rect {
    x = 148,
    y = 276,
    width = 560,
    height = 8,
    radius = 4,
    color = "#26364d",
  },
  ui.Rect {
    x = 148,
    y = 276,
    width = function() return 560 * stages[active:get()].progress end,
    height = 8,
    radius = 4,
    color = function() return stages[active:get()].color end,
    behavior = {
      width = { kind = "spring", mass = 1, damping = 22, stiffness = 170, epsilon = 0.001 },
      color = { duration = 220, easing = "out_quad" },
    },
  },
  ui.Item {
    x = 724,
    y = 212,
    width = 48,
    height = 48,
    ui.Image {
      x = 7,
      y = 7,
      width = 34,
      height = 34,
      source = "examples/assets/sdf-star.svg",
      fill_mode = "preserve_aspect_fit",
      distance_field = true,
      distance_field_spread = 10,
      color_overlay = function() return pinned:get() and "#fbbf24" or "#64748b" end,
      rotation = function() return pinned:get() and 0 or 140 end,
      scale = function() return pinned:get() and 1 or 0.78 end,
      behavior = {
        color_overlay = { duration = 180, easing = "out_quad" },
        rotation = { duration = 380, easing = "out_back" },
        scale = { kind = "spring", mass = 1, damping = 15, stiffness = 230, epsilon = 0.001 },
      },
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_clicked = function() write(pinned, not pinned:get()) end,
    },
  },
  ui.Text {
    x = 30,
    y = 327,
    width = 760,
    text = "Click a stage to inspect it. Click the SDF star to pin the current pipeline.",
    horizontal_alignment = "center",
    color = "#64748b",
    font_size = 13,
  },
}
