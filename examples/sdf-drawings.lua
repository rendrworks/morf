-- A drawing is a shape.
--
-- An SVG is a set of closed curves and so is a letter, so this engine treats
-- them as the same thing: name a file on an `SdfShape` and the layer *is* that
-- drawing's outline. It then unions, subtracts and morphs by the arithmetic a
-- circle does — including morphing into a letter, which is the row along the
-- bottom.
--
-- Nothing here is rasterised. `resvg` can turn a document into pixels and for a
-- photograph that is the right answer, but a shape that has been through a
-- raster has thrown away what a field is measured from: its edge is only as
-- exact as the grid it was flattened onto, it cannot be scaled without being
-- flattened again, and it cannot be walked onto another shape at all, because a
-- picture has pixels and a walk needs points. So the document is read as the
-- curves it was written as, and never as an image of them.
--
-- Four files, deliberately unalike. `heart` is one filled path. `gear` is an
-- `evenodd` path with a hole, which is a different rule from the winding count
-- a field uses and has to be re-wound to agree. `bolt` has no fill at all — it
-- is a stroke, so the shape on screen is the *outline of the stroke*, widened
-- into a loop of its own before it can be measured. And `clipped` is a heart
-- inside a `clip-path`, which is an intersection: the drawing kept only where
-- the window allows, cut against the window's edges rather than drawn whole.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local W, H = 1180, 620
morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true }

local INK = "#141821"
local TEXT = "#e9edf5"
local MUTED = "#78849a"

-- Beside the configuration. `core.shell_path` is rooted at the directory the
-- configuration itself was loaded from, so this holds wherever it is run from
-- and whatever the working directory happens to be.
local function asset(name)
  return core.shell_path("assets/" .. name .. ".svg")
end

local HEART, GEAR = asset("sdf-heart"), asset("sdf-gear")
local BOLT, CLIPPED = asset("sdf-bolt"), asset("sdf-clipped")

local tree = { width = W, height = H }
local function place(node) tree[#tree + 1] = node end
place(ui.Rect { width = W, height = H, color = INK })

local function caption(x, y, width, text)
  place(ui.Text {
    x = x, y = y, width = width, text = text,
    font_size = 15, horizontal_alignment = "center", color = MUTED,
  })
end

-- Each drawing on its own, as a field.
for index, drawing in ipairs({
  { HEART, "one filled path" },
  { GEAR, "evenodd, re-wound" },
  { BOLT, "a stroke, widened" },
  { CLIPPED, "clipped to a circle" },
}) do
  local x = 40 + (index - 1) * 150
  place(ui.Sdf {
    x = x, y = 60, width = 130, height = 130, fill_color = "#e0b56a",
    ui.SdfShape { width = 130, height = 130, source = drawing[1] },
  })
  caption(x - 25, 214, 180, drawing[2])
end

-- Cut out of a shape, which is what being a field rather than a picture buys.
place(ui.Sdf {
  x = 640, y = 60, width = 130, height = 130, fill_color = "#7fc3dd",
  ui.SdfShape { width = 130, height = 130, shape = "rect", radius = 32 },
  ui.SdfShape { x = 16, y = 16, width = 98, height = 98,
                source = HEART, operation = "subtract" },
})
caption(615, 214, 180, "subtracted")

-- And fused with one, by the same smooth union two circles get.
place(ui.Sdf {
  x = 810, y = 60, width = 180, height = 140, fill_color = "#c98fd0", blend = 14,
  ui.SdfShape { x = 0, y = 30, width = 90, height = 90, shape = "circle" },
  ui.SdfShape { x = 55, y = 10, width = 120, height = 120,
                source = BOLT, operation = "smooth_union" },
})
caption(810, 214, 180, "fused with a circle")

-- One drawing walking onto another: a real outline morph, not a cross-fade.
for index = 1, 5 do
  local travel = (index - 1) / 4
  place(ui.Sdf {
    x = 40 + (index - 1) * 150, y = 290, width = 130, height = 130,
    fill_color = "#8fd0a4",
    ui.SdfShape { width = 130, height = 130,
                  source = HEART, source_morph_to = GEAR, morph_progress = travel },
  })
end
caption(40, 434, 730, "heart → gear: contours paired, resampled, and walked point by point")

-- And onto a letter, on exactly the same terms — because by the time the field
-- sees either one, neither knows what it was written in.
for index = 1, 5 do
  local travel = (index - 1) / 4
  place(ui.Sdf {
    x = 40 + (index - 1) * 150, y = 470, width = 130, height = 130,
    fill_color = "#e0847a",
    ui.SdfShape { width = 130, height = 130,
                  source = BOLT, glyph_morph_to = "S", morph_progress = travel },
  })
end
caption(40, 596, 730, "bolt → S: a drawing and a letter correspond like any two outlines")

ui.Item(tree)
