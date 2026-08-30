-- Every piece of motion below is declared in Lua and advanced in Rust: the
-- tween clocks, the spring integration, the rounded-polygon morph, and the
-- distance-field edge are all evaluated on mold's frame tick without Lua
-- running per frame.

local mold = require("mold")
local ui = require("mold.ui")

mold.surface.width = 860
mold.surface.height = 420
mold.surface.anchors = { top = true, left = true }
mold.surface.keyboard_focus = "none"

local points = mold.signal("motion-lab.points", 5)
local extended = mold.signal("motion-lab.extended", false)
local bold = mold.signal("motion-lab.bold", false)
local running = mold.signal("motion-lab.running", true)
local laps = mold.signal("motion-lab.laps", 0)
local intro = mold.signal("motion-lab.intro", "idle")

-- Declared here so a button below can close over it; the group itself is
-- started once the nodes it schedules against exist.
local introduction

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

-- A forever ping-pong behavior: the target is written once and the clock
-- alternates on its own, so the pulse costs nothing per frame in Lua.
local pulse = ui.Rect {
  x = 60,
  y = 96,
  width = 14,
  height = 14,
  radius = 7,
  color = "#f87171",
  opacity = 0.15,
  behavior = {
    opacity = {
      duration = 900,
      easing = "in_out_sine",
      loops = "ping_pong",
    },
  },
}

-- A parametric Polymorpher shape. `star:<points>:<inner ratio>:<rounding>`
-- reaches outlines the built-in name table does not enumerate, and the point
-- count can change while the morph is mid-flight.
local badge = ui.Shape {
  x = 60,
  y = 168,
  width = 132,
  height = 132,
  morph_from = "circle:12",
  morph_to = function() return string.format("star:%d:0.52:0.24", points:get()) end,
  morph_progress = function() return extended:get() and 1 or 0 end,
  fill_color = function() return extended:get() and "#38bdf8" or "#a78bfa" end,
  rotation = function() return extended:get() and 72 or 0 end,
  behavior = {
    morph_progress = {
      duration = 620,
      easing = "in_out_cubic",
      on_finished = function()
        if extended:get() then write(laps, laps:get() + 1) end
      end,
    },
    fill_color = { duration = 260, easing = "out_quad" },
    rotation = {
      duration = 620,
      easing = "out_back",
      rotation_direction = "shortest",
    },
  },
}

-- The cached distance field is converted once; weight, softness, and the
-- outline band are sampled per frame, so animating them never re-runs the
-- CPU distance transform.
local glyph = ui.Image {
  x = 268,
  y = 176,
  width = 116,
  height = 116,
  source = "examples/assets/sdf-star.svg",
  fill_mode = "preserve_aspect_fit",
  distance_field = true,
  distance_field_spread = 12,
  color_overlay = "#fbbf24",
  distance_field_weight = function() return bold:get() and 0.34 or 0.52 end,
  distance_field_softness = function() return bold:get() and 0.4 or 1.6 end,
  distance_field_outline_width = function() return bold:get() and 3.2 or 0 end,
  distance_field_outline_color = "#f8fafc",
  behavior = {
    distance_field_weight = { duration = 420, easing = "in_out_quad" },
    distance_field_softness = { duration = 420, easing = "in_out_quad" },
    distance_field_outline_width = {
      kind = "spring",
      mass = 1,
      damping = 14,
      stiffness = 210,
      epsilon = 0.001,
    },
  },
}

-- A delayed, repeating sweep. Pausing it holds the bar exactly where it is;
-- resuming picks the clock back up rather than restarting it.
local sweep = ui.Rect {
  x = 468,
  y = 250,
  width = 40,
  height = 8,
  radius = 4,
  color = "#2dd4bf",
  translate_x = 0,
  behavior = {
    translate_x = {
      duration = 1400,
      easing = "in_out_cubic",
      delay = 200,
      loops = "ping_pong",
    },
  },
}

local function label(x, y, width, text, color, size, weight)
  return ui.Text {
    x = x,
    y = y,
    width = width,
    text = text,
    color = color,
    font_size = size,
    font_weight = weight or 400,
  }
end

local function button(x, y, width, text, on_clicked)
  return ui.Item {
    x = x,
    y = y,
    width = width,
    height = 34,
    ui.Rect {
      anchors = { fill = true },
      radius = 8,
      color = "#1d293b",
      border_width = 1,
      border_color = "#334155",
      behavior = { color = { duration = 140, easing = "out_quad" } },
    },
    ui.Text {
      anchors = { fill = true },
      y = 8,
      text = text,
      horizontal_alignment = "center",
      color = "#e2e8f0",
      font_size = 13,
      font_weight = 600,
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_clicked = on_clicked,
    },
  }
end

ui.Item {
  width = 860,
  height = 420,
  ui.Rect {
    anchors = { fill = true },
    gradient_type = "linear",
    gradient_start_color = "#0b1220",
    gradient_end_color = "#131f32",
    gradient_start_x = 0,
    gradient_start_y = 0,
    gradient_end_x = 1,
    gradient_end_y = 1,
  },
  ui.MouseArea { anchors = { fill = true } },

  label(60, 32, 460, "Motion lab", "#f8fafc", 24, 700),
  label(
    60,
    64,
    620,
    "loops, delay, lifecycle handlers, parametric shapes, live field edges",
    "#64748b",
    13
  ),
  pulse,
  label(88, 94, 300, "ping-pong opacity, forever", "#94a3b8", 13),

  badge,
  label(60, 314, 200, function()
    return string.format("%d points  ·  %d laps", points:get(), laps:get())
  end, "#94a3b8", 13),

  glyph,
  label(268, 314, 200, "signed distance field", "#94a3b8", 13),

  ui.Rect {
    x = 468,
    y = 168,
    width = 332,
    height = 116,
    radius = 14,
    color = "#131f32",
    border_width = 1,
    border_color = "#26364d",
  },
  label(488, 188, 300, "delayed ping-pong sweep", "#94a3b8", 13),
  sweep,

  button(60, 356, 132, "morph", function()
    write(extended, not extended:get())
  end),
  button(204, 356, 132, "add point", function()
    write(points, points:get() % 9 + 3)
  end),
  button(348, 356, 132, "field weight", function()
    write(bold, not bold:get())
  end),
  button(492, 356, 132, function()
    return running:get() and "pause sweep" or "resume sweep"
  end, function()
    if running:get() then
      mold.animation.pause(sweep, "translate_x")
    else
      mold.animation.resume(sweep, "translate_x")
    end
    write(running, not running:get())
  end),
  button(636, 356, 132, "skip intro", function()
    if introduction then introduction:finish() end
  end),
  label(636, 396, 200, function() return "intro: " .. intro:get() end, "#475569", 12),
}

-- One schedule across three properties on two nodes: the badge fades and lifts,
-- then the glyph thickens while the badge settles back. The group owns only the
-- ordering; each step is an ordinary property animation once it starts.
introduction = mold.animation.play {
  on_finished = function(reason) write(intro, reason) end,
  { node = badge, property = "opacity", from = 0, to = 1, duration = 260, easing = "out_quad" },
  { pause = 80 },
  { parallel = {
    { node = badge, property = "translate_y", from = 24, to = 0, duration = 420, easing = "out_back" },
    { node = glyph, property = "opacity", from = 0, to = 1, duration = 420, easing = "out_quad" },
  }},
}
write(intro, "playing")

-- Written after construction so the sweep has a target to travel toward. The
-- behavior above turns this single write into an endless alternating pass.
sweep.translate_x = 252

