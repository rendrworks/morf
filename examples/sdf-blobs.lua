-- A lava lamp for the desktop: the metaball composition with nothing behind it.
--
-- Same field as `sdf-metaballs.lua` — six circles joined by a smooth union,
-- each carrying its own fill so the colours bleed through the necks — but the
-- surface paints no background, so the blobs float over whatever is beneath.
--
-- Two things make that work and both are the engine's defaults rather than
-- anything written here. A surface only shows what it paints, so leaving out a
-- background rectangle leaves it transparent. And the input region is derived
-- from live `MouseArea` geometry every paint, so a configuration with nothing
-- interactive claims nothing and every click goes straight through to the
-- desktop.
--
-- `MOLD_BLOB_LAYER` picks where it sits: `bottom` (the default) puts it above
-- the wallpaper and beneath the windows; `overlay` floats it over everything.

local mold = require("mold")
local ui = require("mold.ui")
local core = require("mold.core")

local screen = mold.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080
local SHORT = math.min(W, H)

mold.surface.width = W
mold.surface.height = H
mold.surface.anchors = { top = true, left = true, right = true, bottom = true }
mold.surface.layer = core.env("MOLD_BLOB_LAYER") or "bottom"
mold.surface.keyboard_focus = "none"
-- Never reserve space; this is decoration and must not move a single window.
mold.surface.exclusive_zone = -1

local elapsed = core.elapsed_timer()

-- Sizes are fractions of the short side, so the lamp fills any output the same
-- way instead of being tuned to one monitor.
local function s(fraction) return SHORT * fraction end

local BLOBS = {
  { radius = 0.20, speed = 0.00042, orbit = 0.135, phase = 0.0, wobble = 0.030, color = "#f0b47a" },
  { radius = 0.155, speed = -0.00061, orbit = 0.165, phase = 1.9, wobble = 0.039, color = "#e8735a" },
  { radius = 0.125, speed = 0.00083, orbit = 0.110, phase = 3.4, wobble = 0.021, color = "#b4e1ea" },
  { radius = 0.108, speed = -0.00104, orbit = 0.182, phase = 5.0, wobble = 0.046, color = "#7fb7c9" },
  { radius = 0.092, speed = 0.00127, orbit = 0.085, phase = 2.4, wobble = 0.025, color = "#f5d98b" },
  { radius = 0.079, speed = -0.00149, orbit = 0.203, phase = 0.7, wobble = 0.034, color = "#c98fd1" },
}

-- A field is evaluated over its own rectangle, every layer at every pixel of
-- it. Making that rectangle the whole screen would ask the GPU for 8.3 million
-- fragments times six layers per frame, per output, for a composition that
-- never leaves the middle of the screen — enough to miss the frame deadline
-- outright. So the node is only as big as the orbits reach, and sits centred.
local REACH = 0.203 + 0.046 + 0.10
local FIELD_W = math.min(W, SHORT * (REACH * 2 + 0.06))
local FIELD_H = math.min(H, SHORT * (REACH * 2 * 0.62 + 0.06))
local FIELD_X = (W - FIELD_W) / 2
local FIELD_Y = (H - FIELD_H) / 2

local blobs = {}

--- Places every blob for the moment the clock is at.
local function advance()
  local now = elapsed:elapsed_ms()
  for index, spec in ipairs(BLOBS) do
    local node = blobs[index]
    if node then
      local angle = spec.phase + now * spec.speed
      -- The orbit breathes, so the blobs do not simply circle at a fixed
      -- distance and never touch.
      local orbit = s(spec.orbit) + math.sin(now * 0.00037 + spec.phase) * s(spec.wobble)
      node.x = FIELD_W / 2 + math.cos(angle) * orbit - s(spec.radius) / 2
      node.y = FIELD_H / 2 + math.sin(angle) * orbit * 0.62 - s(spec.radius) / 2
    end
  end
end

local field = { x = FIELD_X, y = FIELD_Y, width = FIELD_W, height = FIELD_H }
-- Only the fallback: every layer below names its own fill.
field.fill_color = "#f0b47a"
field.stroke_color = "#8a4a17"
field.stroke_width = math.max(2, SHORT * 0.0028)
for index, spec in ipairs(BLOBS) do
  blobs[index] = ui.SdfShape {
    x = FIELD_W / 2,
    y = FIELD_H / 2,
    width = s(spec.radius),
    height = s(spec.radius),
    shape = "circle",
    fill_color = spec.color,
    -- The first layer establishes the field; the rest melt into it.
    operation = index == 1 and "union" or "smooth_union",
    blend = s(0.096),
  }
  field[#field + 1] = blobs[index]
end

ui.Item {
  width = W,
  height = H,
  -- No background rectangle: what is not painted stays transparent.
  ui.Sdf(field),

  ui.Timer {
    interval = 16,
    ["repeat"] = true,
    running = true,
    on_triggered = advance,
  },
}
