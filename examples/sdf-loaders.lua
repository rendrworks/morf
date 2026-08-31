-- Progress and busy indicators, built entirely out of distance fields.
--
-- Every one of these is normally a sprite sheet, an SVG with a rotate
-- transform, or a canvas redraw. Here each is a handful of analytic fields with
-- one number moving, which means they are resolution independent, they cost the
-- fragment shader and nothing else, and the shapes can do things a rotating
-- image cannot — a gap that closes, a bar whose ends melt together, dots that
-- fuse as they pass.

local mold = require("mold")
local ui = require("mold.ui")
local core = require("mold.core")

local W, H = 760, 260
mold.surface.width = W
mold.surface.height = H
mold.surface.anchors = { top = true, left = true }
mold.surface.keyboard_focus = "none"

local INK = "#0e1213"
local PANEL = "#141a1c"
local ACCENT = "#b4e1ea"
local WARM = "#f0b47a"
local MUTED = "#6a8389"

local elapsed = core.elapsed_timer()
-- Progress runs 0 to 1 and back, so the indicators have something to report.
local progress = mold.signal("loaders.progress", 0)

local function caption(x, text)
  return ui.Text { x = x + 18, y = 196, width = 150, text = text, font_size = 12, wrap = true, color = MUTED }
end

local function plinth(x, title)
  return ui.Item {
    x = x,
    y = 0,
    width = 180,
    height = H,
    ui.Rect { x = 6, y = 6, width = 168, height = 236, radius = 16, color = PANEL },
    ui.Text { x = 18, y = 168, width = 150, text = title, font_size = 15, color = ACCENT },
  }
end

-- ── A sweep ─────────────────────────────────────────────────────────────────
--
-- A progress arc: a ring *intersected* with a pie, so the filled part is the
-- overlap of the two fields. A separate muted ring underneath is the track.
-- Filling an arc this way keeps both ends square to the radius at every angle,
-- which is fiddly to do with a stroked path and free here.
-- The angle is bound to the progress and given a behavior, rather than being
-- assigned each frame: the report is a step function, and the behavior is what
-- turns it into a sweep. Writing it imperatively in the tick below would make
-- the arc jump from empty to full with nothing in between.
local sweep_pie = ui.SdfShape {
  x = 44, y = 20, width = 80, height = 80,
  shape = "pie",
  operation = "intersect",
  angle = function() return 4 + progress:get() * 352 end,
  behavior = { angle = { duration = 2000, easing = "in_out_cubic" } },
}
local sweep_track = ui.Sdf {
  x = 6, y = 20, width = 168, height = 140,
  fill_color = "#25353a",
  ui.SdfShape { x = 44, y = 20, width = 80, height = 80, shape = "ring", thickness = 11 },
}
local sweep = ui.Sdf {
  x = 6, y = 20, width = 168, height = 140,
  fill_color = ACCENT, stroke_color = INK, stroke_width = 2,
  ui.SdfShape { x = 44, y = 20, width = 80, height = 80, shape = "ring", thickness = 11 },
  sweep_pie,
}

-- ── A ring with a travelling gap ────────────────────────────────────────────
--
-- The classic spinner, except the gap is *cut* out of the ring rather than the
-- ring being a rotated bitmap, so it can widen and narrow while it travels.
local spinner_gap = ui.SdfShape {
  x = 34, y = 10, width = 100, height = 100,
  shape = "pie",
  operation = "subtract",
  angle = 40,
}
local spinner = ui.Sdf {
  x = 6, y = 20, width = 168, height = 140,
  fill_color = WARM, stroke_color = "#8a4a17", stroke_width = 2.5,
  ui.SdfShape { x = 44, y = 20, width = 80, height = 80, shape = "ring", thickness = 12 },
  spinner_gap,
}

-- ── Three dots that fuse as they pass ───────────────────────────────────────
--
-- A smooth union between neighbours, so the dots do not merely bob — they draw
-- out of one another and pinch back apart. Each one is *dropped* rather than
-- driven: gravity pulls it down, the floor gives most of the speed back, and
-- the three drift out of step because they were thrown at different strengths.
-- Nothing here writes a position; a dot that has come to rest is thrown again.
local dots = {}
local dots_field = {
  x = 6, y = 20, width = 168, height = 140,
  fill_color = ACCENT, stroke_color = INK, stroke_width = 2.5,
}
for index = 1, 3 do
  dots[index] = ui.SdfShape {
    x = 30 + (index - 1) * 40,
    y = 50,
    width = 30,
    height = 30,
    shape = "circle",
    operation = index == 1 and "union" or "smooth_union",
    blend = 22,
  }
  dots_field[#dots_field + 1] = dots[index]
end

-- ── A bar that fills ────────────────────────────────────────────────────────
--
-- The fill is its own capsule, so its leading end stays perfectly round at
-- every width instead of being a clipped rectangle.
local bar_fill = ui.SdfShape {
  x = 20, y = 56, width = 20, height = 26,
  shape = "capsule",
  width = function() return 20 + progress:get() * 108 end,
  behavior = { width = { duration = 2000, easing = "in_out_cubic" } },
}
local bar = ui.Sdf {
  x = 6, y = 20, width = 168, height = 140,
  fill_color = WARM, stroke_color = "#8a4a17", stroke_width = 2.5,
  ui.SdfShape { x = 20, y = 56, width = 128, height = 26, shape = "capsule" },
  bar_fill,
}

--- Drops one dot again, a little harder or softer than last time.
local function drop(index)
  mold.animation.fling {
    node = dots[index],
    property = "y",
    velocity = -150 - index * 22 - math.random() * 60,
    gravity = 900,
    friction = 0,
    min_velocity = 26,
    bounce = 0.72,
    min = 12,
    max = 88,
  }
end

--- Drives every indicator from one clock and one progress value.
local function advance()
  local now = elapsed:elapsed_ms()
  -- The sweep and the bar are bound to `progress` with behaviors of their own,
  -- so nothing here touches them. Only the two continuous motions are written
  -- per frame, because an orbit is not a transition between two states.
  -- The gap travels, and breathes as it goes.
  spinner_gap.rotation = (now * 0.22) % 360
  spinner_gap.angle = 28 + math.sin(now * 0.0022) * 18
  -- The dots are falling, not being positioned; each is thrown again once it
  -- has settled on the floor.
  for index = 1, 3 do
    if not mold.animation.active(dots[index], "y") then
      drop(index)
    end
  end
end

ui.Item {
  width = W, height = H,
  ui.Rect { width = W, height = H, color = INK },

  plinth(0, "Sweep"), sweep_track, sweep, caption(0, "a ring intersected with a pie"),
  plinth(190, "Spinner"), ui.Item { x = 190, width = 180, height = H, spinner },
  caption(190, "the gap is cut out, and breathes"),
  plinth(380, "Dots"), ui.Item { x = 380, width = 180, height = H, ui.Sdf(dots_field) },
  caption(380, "neighbours fuse as they pass"),
  plinth(570, "Bar"), ui.Item { x = 570, width = 180, height = H, bar },
  caption(570, "a capsule fill, round at any width"),

  ui.Timer { interval = 16, ["repeat"] = true, running = true, on_triggered = advance },
  ui.Timer {
    interval = 1,
    ["repeat"] = false,
    running = true,
    on_triggered = function()
      for index = 1, 3 do drop(index) end
    end,
  },
  ui.Timer {
    interval = 2400,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      local ok, error = progress:set(progress:get() > 0.5 and 0 or 1)
      assert(ok, error)
    end,
  },
}
