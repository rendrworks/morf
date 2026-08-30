-- Screen border, ported from `~/.config/quickshell/border/modules/border/Border.qml`.
--
-- The original composes twelve layer surfaces: four edge strips, four corner
-- windows that punch a quarter-disc out of a filled square with a Canvas, and
-- four zero-size windows whose only job is to claim an exclusive zone.
--
-- mold hosts one layer surface per process, so the whole frame is drawn once
-- instead. That is not a workaround: a frame with rounded inner corners is one
-- path — the output rectangle, then the inset rounded rectangle wound as a
-- hole — and an even-odd fill leaves exactly the border. The corners come out
-- of the same tessellation as the edges, so there is no seam to line up.

local ui = require("mold.ui")
local theme = require("theme")

local border = {}

--- The frame as a single even-odd path: the output, then the inset rounded
--- rectangle wound as a hole.
local function frame_path(WIDTH, HEIGHT, thickness, inner_radius)
  local left, top = thickness, thickness
  local right, bottom = WIDTH - thickness, HEIGHT - thickness
  local r = inner_radius
  return table.concat({
    -- Outer contour: the whole output.
    string.format("M0 0 L%d 0 L%d %d L0 %d Z", WIDTH, WIDTH, HEIGHT, HEIGHT),
    -- Inner contour: the visible area, with the same rounded corners the
    -- Canvas cut out of each corner window.
    string.format("M%d %d", left + r, top),
    string.format("L%d %d", right - r, top),
    string.format("A%d %d 0 0 1 %d %d", r, r, right, top + r),
    string.format("L%d %d", right, bottom - r),
    string.format("A%d %d 0 0 1 %d %d", r, r, right - r, bottom),
    string.format("L%d %d", left + r, bottom),
    string.format("A%d %d 0 0 1 %d %d", r, r, left, bottom - r),
    string.format("L%d %d", left, top + r),
    string.format("A%d %d 0 0 1 %d %d", r, r, left + r, top),
    "Z",
  }, " ")
end

--- Builds the frame for one output.
function border.build(width, height)
  local short_side = math.min(width, height)
  -- Exactly the arithmetic in Border.qml.
  local thickness = math.floor(width * 0.005 + 0.5)
  local inner_radius = math.floor(short_side * (24 / 2160) + 0.5)
  return ui.Shape {
    width = width,
    height = height,
    path = frame_path(width, height, thickness, inner_radius),
    fill_rule = "even_odd",
    fill_color = function() return theme.color0() end,
    behavior = { fill_color = { duration = 200, easing = "out_cubic" } },
  }
end

--- The space the original reserves on every edge with its `Reserve` windows.
function border.reserved(width, height)
  local short_side = math.min(width, height)
  return math.floor(width * 0.005 + 0.5) + math.floor(short_side * (6 / 2160) + 0.5)
end

return border
