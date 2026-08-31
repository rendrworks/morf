-- Screen border, ported from `~/.config/quickshell/border/modules/border/Border.qml`.
--
-- The original composes twelve layer surfaces: four edge strips, four corner
-- windows that punch a quarter-disc out of a filled square with a Canvas, and
-- four zero-size windows whose only job is to claim an exclusive zone.
--
-- morf hosts one layer surface per process, so the whole frame is drawn once
-- instead. And a frame is not really a path: it is the output rectangle with a
-- rounded rectangle taken out of the middle, which is one subtraction between
-- two distance fields. Written that way there is nothing to tessellate, no
-- arc commands to get right, and the inset and the corner radius are ordinary
-- animatable numbers rather than text baked into an SVG string.

local ui = require("morf.ui")
local theme = require("theme")

local border = {}

--- Builds the frame for one output.
function border.build(width, height)
  local short_side = math.min(width, height)
  -- Exactly the arithmetic in Border.qml.
  local thickness = math.floor(width * 0.005 + 0.5)
  local inner_radius = math.floor(short_side * (24 / 2160) + 0.5)
  return ui.Sdf {
    width = width,
    height = height,
    fill_color = function() return theme.color0() end,
    behavior = { fill_color = { duration = 200, easing = "out_cubic" } },
    -- The output, square-cornered, because the compositor's own edge is.
    ui.SdfShape {
      x = 0,
      y = 0,
      width = width,
      height = height,
      shape = "box",
      operation = "union",
    },
    -- And the visible area taken back out of it, with the same rounded corners
    -- the original cut out of each corner window with a Canvas. The corners
    -- come out of the same field as the edges, so there is no seam to line up.
    ui.SdfShape {
      x = thickness,
      y = thickness,
      width = width - thickness * 2,
      height = height - thickness * 2,
      shape = "box",
      radius = inner_radius,
      operation = "subtract",
    },
  }
end

--- The space the original reserves on every edge with its `Reserve` windows.
function border.reserved(width, height)
  local short_side = math.min(width, height)
  return math.floor(width * 0.005 + 0.5) + math.floor(short_side * (6 / 2160) + 0.5)
end

return border
