-- Every shape family a distance field can take, with its own parameter moving.
--
-- Nine analytic fields, each evaluated per fragment. The point of the grid is
-- that the parameters are ordinary animatable properties: `points` on a star,
-- `angle` on a pie, `thickness` on a ring and a cross, `radius` on a box. None
-- of them rebuilds geometry — there is no geometry — so a parameter can be
-- driven every frame for nothing.

local morf = require("morf")
local ui = require("morf.ui")

local COLUMNS = 5
local CELL = 190
local LABEL = 46

morf.surface.width = COLUMNS * CELL
morf.surface.height = 2 * (CELL + LABEL)
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local INK = "#0e1213"
local PANEL = "#141a1c"
local ACCENT = "#b4e1ea"
local MUTED = "#6a8389"

-- One value swinging between zero and one drives every cell.
local phase = morf.signal("gallery.phase", 0)

local function lerp(a, b) return function() return a + (b - a) * phase:get() end end

local SWING = { duration = 1600, easing = "in_out_cubic" }

-- Each entry names the shape and the one property it animates, so the grid
-- reads as "here is the family, here is the knob it has".
local families = {
  -- A family with no parameter of its own still has its layer rectangle, which
  -- is an ordinary animatable pair of properties like any other.
  {
    "circle",
    "the layer box is the radius",
    {
      x = lerp(35, 10),
      y = lerp(25, 0),
      width = lerp(120, 170),
      height = lerp(120, 170),
      behavior = { x = SWING, y = SWING, width = SWING, height = SWING },
    },
  },
  {
    "box",
    "radius: square to fully rounded",
    { radius = lerp(0, 60), behavior = { radius = SWING } },
  },
  -- A stadium in a square box is a circle, so this one is given a wider box:
  -- the shape reads as its own family only when the sides differ, and
  -- stretching that box is the animation.
  {
    "capsule",
    "the short side rounds it",
    {
      x = lerp(55, 12),
      y = 55,
      width = lerp(80, 166),
      height = 62,
      behavior = { x = SWING, width = SWING },
    },
  },
  {
    "triangle",
    "equilateral; rotating in place",
    { rotation = lerp(0, 120), behavior = { rotation = SWING } },
  },
  {
    "hexagon",
    "a regular six-gon, turning",
    { rotation = lerp(0, 90), behavior = { rotation = SWING } },
  },
  {
    "star",
    "points: 4 to 9, waist held",
    {
      points = lerp(4, 9),
      inner_radius = 0.42,
      behavior = { points = SWING },
    },
  },
  {
    "ring",
    "thickness: hairline to solid",
    { thickness = lerp(6, 58), behavior = { thickness = SWING } },
  },
  {
    "pie",
    "angle: a sliver to a full turn",
    { angle = lerp(20, 355), behavior = { angle = SWING } },
  },
  {
    "cross",
    "thickness: thin arms to a block",
    { thickness = lerp(14, 96), behavior = { thickness = SWING } },
  },
}

--- One labelled cell holding a single field.
---
--- The cell knows its own size and nothing about where it goes: the grid
--- below places it, in five equal tracks.
local function cell(name, caption, extra)
  local layer = {
    x = 35,
    y = 25,
    width = 120,
    height = 120,
    shape = name,
  }
  for key, value in pairs(extra) do layer[key] = value end

  return ui.Item {
    width = CELL,
    height = CELL + LABEL,
    ui.Rect { x = 6, y = 6, width = CELL - 12, height = CELL + LABEL - 12, radius = 14, color = PANEL },
    ui.Sdf {
      x = 6,
      y = 6,
      width = CELL - 12,
      height = CELL - 12,
      fill_color = ACCENT,
      -- Three stops across the shape, mixed in OkLCh so the middle keeps its
      -- chroma; the field takes a gradient the same way a rectangle does.
      gradient = { angle = 135, space = "oklch", stops = { "#e6f7fa", { ACCENT, 0.5 }, "#5fa8d3" } },
      stroke_color = INK,
      stroke_width = 2.5,
      ui.SdfShape(layer),
    },
    ui.Text { x = 20, y = CELL - 14, width = CELL - 34, text = name, font_size = 15, color = ACCENT },
    ui.Text {
      x = 20,
      y = CELL + 6,
      width = CELL - 34,
      text = caption,
      font_size = 11,
      wrap = true,
      color = MUTED,
    },
  }
end

-- Five equal columns; the rows are as tall as the cells. Nine cells fill
-- the grid in order, wrapping after the fifth, and nobody computes an x.
local cells = {
  width = COLUMNS * CELL,
  height = 2 * (CELL + LABEL),
  template_columns = { "repeat(" .. COLUMNS .. ", 1fr)" },
}
for _, family in ipairs(families) do
  cells[#cells + 1] = cell(family[1], family[2], family[3])
end

ui.Item {
  width = COLUMNS * CELL,
  height = 2 * (CELL + LABEL),
  ui.Rect { width = COLUMNS * CELL, height = 2 * (CELL + LABEL), color = INK },
  ui.Grid(cells),
  ui.Timer {
    interval = 1800,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      local ok, error = phase:set(phase:get() > 0.5 and 0 or 1)
      assert(ok, error)
    end,
  },
}
