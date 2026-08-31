-- Composed signed-distance fields.
--
-- Every panel here is one `ui.Sdf` node whose `ui.SdfShape` children are
-- combined in a single fragment shader. A layer is an ordinary scene node, so
-- its numbers animate through the same behaviors as any other property — none
-- of the motion below is a mechanism of its own, it is `morph_progress` and
-- `blend` easing between two values.
--
-- Two things here cannot be expressed by interpolating outlines, which is the
-- reason the engine resolves fields rather than tessellating them:
--
--   * a morph passes through shapes neither end describes, and survives the
--     outline splitting or merging on the way;
--   * a smooth union has no seam at all — the surfaces bulge into each other
--     over a radius, the way two drops of liquid meet.

local morf = require("morf")
local ui = require("morf.ui")

morf.surface.width = 900
morf.surface.height = 340
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local INK = "#0e1213"
local ACCENT = "#b4e1ea"
local WARM = "#f0b47a"

-- One phase drives everything, so the panels stay in step.
local phase = morf.signal("sdf.phase", 0)
local merged = morf.signal("sdf.merged", false)

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--- A titled panel with a field in it.
local function panel(x, title, caption, field)
  return ui.Item {
    x = x,
    y = 0,
    width = 280,
    height = 340,
    ui.Rect {
      x = 0,
      y = 0,
      width = 280,
      height = 340,
      radius = 18,
      color = "#141a1c",
    },
    field,
    ui.Text {
      x = 20,
      y = 250,
      width = 240,
      text = title,
      font_size = 19,
      color = ACCENT,
    },
    ui.Text {
      x = 20,
      y = 278,
      width = 240,
      text = caption,
      font_size = 13,
      wrap = true,
      color = "#6a8389",
    },
  }
end

-- ── A morph that changes what the outline is ────────────────────────────────
--
-- `shape` and `morph_to` name the two ends; `morph_progress` is an ordinary
-- number between them, so one behavior is the whole animation.
local morphing = ui.Sdf {
  x = 0,
  y = 0,
  width = 280,
  height = 240,
  fill_color = ACCENT,
  stroke_color = INK,
  stroke_width = 3,
  ui.SdfShape {
    x = 70,
    y = 40,
    width = 140,
    height = 140,
    shape = "circle",
    morph_to = "star",
    points = 6,
    inner_radius = 0.45,
    morph_progress = function() return phase:get() end,
    behavior = {
      morph_progress = { duration = 1400, easing = "in_out_cubic" },
    },
  },
}

-- ── Two fields meeting without a seam ───────────────────────────────────────
--
-- The blend radius is what decides whether these are two circles or one shape.
-- Animating it is a topology change: the count of separate pieces goes from two
-- to one, mid-flight, with nothing to interpolate between.
local merging = ui.Sdf {
  x = 0,
  y = 0,
  width = 280,
  height = 240,
  fill_color = WARM,
  stroke_color = INK,
  stroke_width = 3,
  ui.SdfShape {
    x = 40,
    y = 70,
    width = 90,
    height = 90,
    shape = "circle",
  },
  ui.SdfShape {
    x = 150,
    y = 70,
    width = 90,
    height = 90,
    shape = "circle",
    operation = "smooth_union",
    blend = function() return merged:get() and 55 or 0 end,
    behavior = { blend = { duration = 900, easing = "in_out_quad" } },
  },
}

-- ── Boolean composition ─────────────────────────────────────────────────────
--
-- A ring with a wedge cut out of it, the cut opening and closing. The hole is
-- exact at every scale because nothing here is a polygon: it is simply where
-- one field wins over another.
--
-- The cutter is a `pie`, not a bar, and it sweeps from a sliver to a right
-- angle. A bar would have been the obvious choice and the wrong one — it is
-- symmetric about its own centre, so rotating it half a turn leaves the cut
-- exactly where it started and the panel appears not to animate at all.
local carving = ui.Sdf {
  x = 0,
  y = 0,
  width = 280,
  height = 240,
  fill_color = ACCENT,
  stroke_color = INK,
  stroke_width = 3,
  ui.SdfShape {
    x = 65,
    y = 35,
    width = 150,
    height = 150,
    shape = "ring",
    thickness = 34,
  },
  ui.SdfShape {
    x = 55,
    y = 25,
    width = 170,
    height = 170,
    shape = "pie",
    operation = "subtract",
    angle = function() return 8 + phase:get() * 82 end,
    rotation = function() return 40 + phase:get() * 140 end,
    behavior = {
      angle = { duration = 1400, easing = "in_out_cubic" },
      rotation = { duration = 1400, easing = "in_out_cubic" },
    },
  },
}

ui.Item {
  width = 900,
  height = 340,
  ui.Rect { width = 900, height = 340, color = INK },
  panel(10, "Morph", "circle to six-pointed star, as one number", morphing),
  panel(310, "Merge", "two fields joining with no seam", merging),
  panel(610, "Carve", "a wedge cut out of a ring, opening", carving),

  -- The only driver: one value flipping back and forth.
  ui.Timer {
    interval = 2000,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      write(phase, phase:get() > 0.5 and 0 or 1)
      write(merged, not merged:get())
    end,
  },
}
