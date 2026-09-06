-- Metaballs: six fields orbiting inside one composition.
--
-- Each blob is a layer with a `smooth_union` and a wide blend radius, so where
-- two of them come close their surfaces bulge into one another and the seam
-- disappears entirely. When they separate the surface pinches and breaks. The
-- number of separate pieces on screen changes continuously, and there is
-- nothing anywhere in the scene that knows how many there are — the count is a
-- consequence of the field, not a thing the configuration tracks.
--
-- This is the case that cannot be written with outlines at all. Interpolating
-- two closed paths needs a correspondence between their points; a splitting or
-- merging outline has none.

local morf = require("morf")
local ui = require("morf.ui")

local WIDTH, HEIGHT = 680, 420

morf.surface.width = WIDTH
morf.surface.height = HEIGHT
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local theme = morf.theme {
  ink = "#0e1213",
}

-- One clock, read every frame. The orbits are plain trigonometry; the engine
-- only ever sees `x` and `y` being assigned.
local clock = morf.core and nil
local core = require("morf.core")
local elapsed = core.elapsed_timer()

-- Each blob carries its own fill. A composition is one surface but not one
-- colour: the fills cross-fade with exactly the weight the smooth union uses,
-- so the colour bleeds through the neck as two blobs draw together and
-- separates again as they part.
local BLOBS = {
  { radius = 96, speed = 0.00042, orbit = 118, phase = 0.0, wobble = 26, color = "#f0b47a" },
  { radius = 74, speed = -0.00061, orbit = 142, phase = 1.9, wobble = 34, color = "#e8735a" },
  { radius = 60, speed = 0.00083, orbit = 96, phase = 3.4, wobble = 18, color = "#b4e1ea" },
  { radius = 52, speed = -0.00104, orbit = 158, phase = 5.0, wobble = 40, color = "#7fb7c9" },
  { radius = 44, speed = 0.00127, orbit = 74, phase = 2.4, wobble = 22, color = "#f5d98b" },
  { radius = 38, speed = -0.00149, orbit = 176, phase = 0.7, wobble = 30, color = "#c98fd1" },
}

local blobs = {}

--- Places every blob for the moment `now`.
local function advance()
  local now = elapsed:elapsed_ms()
  for index, spec in ipairs(BLOBS) do
    local node = blobs[index]
    if node then
      local angle = spec.phase + now * spec.speed
      -- The orbit itself breathes, so the blobs do not simply circle at a
      -- fixed distance and never touch.
      local orbit = spec.orbit + math.sin(now * 0.00037 + spec.phase) * spec.wobble
      node.x = WIDTH / 2 + math.cos(angle) * orbit - spec.radius / 2
      node.y = HEIGHT / 2 + math.sin(angle) * orbit * 0.62 - spec.radius / 2
    end
  end
end

local field = { x = 0, y = 0, width = WIDTH, height = HEIGHT }
-- The field's own fill is only the fallback for a layer that names none.
field.fill_color = "#f0b47a"
field.stroke_color = "#8a4a17"
field.stroke_width = 3
for index, spec in ipairs(BLOBS) do
  blobs[index] = ui.SdfShape {
    x = WIDTH / 2,
    y = HEIGHT / 2,
    width = spec.radius,
    height = spec.radius,
    shape = "circle",
    fill_color = spec.color,
    -- The first layer establishes the field; the rest melt into it.
    operation = index == 1 and "union" or "smooth_union",
    blend = 46,
  }
  field[#field + 1] = blobs[index]
end

ui.Item {
  width = WIDTH,
  height = HEIGHT,
  ui.Rect { width = WIDTH, height = HEIGHT, color = theme.ink },
  ui.Sdf(field),

  -- 16ms is a frame; the positions are written directly rather than eased,
  -- because the motion is the orbit itself and not a transition between states.
  ui.Timer {
    interval = 16,
    ["repeat"] = true,
    running = true,
    on_triggered = advance,
  },
}
