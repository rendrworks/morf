-- Lava made of blurred desktop.
--
--     oslo make run --example examples/sdf-blobs-blur.lua
--
-- The blobs are barely coloured at all. What you see through them is whatever
-- happens to be behind the surface — your windows, your wallpaper — blurred by
-- the compositor, with just enough tint left to say which blob is which.
--
-- Each blob asks for the blur itself, so the region follows the swarm as it
-- drifts. A region is rectangles, but at pixel granularity, and a circle is
-- simply a rectangle whose corner radii are half its size — so what the
-- compositor blurs is a circle per blob, not a box.
--
-- Needs a compositor that implements `ext-background-effect-v1` (KDE 6.7+,
-- niri, GNOME 51, COSMIC, Hyprland) *and* has blur switched on — Hyprland gates
-- it behind `decoration:blur:enabled`, and one pass at the default size barely
-- reads as a blur at all. Three passes is where it starts looking like glass.
-- Without any of that the blobs are simply translucent, which is why they carry
-- their own faint colour rather than relying entirely on what is underneath.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080
local SHORT = math.min(W, H)

morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = core.env("MORF_BLOB_LAYER") or "overlay"
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1

local function s(fraction) return SHORT * fraction end

-- A twentieth of full alpha — barely a tint. The blur is the subject and the
-- colour only says which blob is which; nineteen parts of what shows through a
-- blob is the desktop behind it. Set these to `00` and the glass goes
-- completely invisible, which is worth trying once: the rim alone is enough to
-- read the shape, and nothing else on screen changes.
local BLOBS = {
  { radius = 0.150, orbit = 0.185, speed = 0.00040, phase = 0.0, wobble = 0.052, color = "#f0b47a0d" },
  { radius = 0.122, orbit = 0.225, speed = -0.00057, phase = 1.9, wobble = 0.066, color = "#e8735a0d" },
  { radius = 0.104, orbit = 0.152, speed = 0.00079, phase = 3.4, wobble = 0.038, color = "#b4e1ea0d" },
  { radius = 0.088, orbit = 0.248, speed = -0.00098, phase = 5.0, wobble = 0.074, color = "#7fb7c90d" },
  { radius = 0.074, orbit = 0.118, speed = 0.00121, phase = 2.4, wobble = 0.044, color = "#f5d98b0d" },
  { radius = 0.062, orbit = 0.272, speed = -0.00142, phase = 0.7, wobble = 0.058, color = "#c98fd10d" },
}

local BLEND = s(0.038)

local elapsed = core.elapsed_timer()
local blobs = {}

local function advance()
  local now = elapsed:elapsed_ms()
  for index, spec in ipairs(BLOBS) do
    local node = blobs[index]
    if node then
      local size = s(spec.radius)
      local angle = spec.phase + now * spec.speed
      -- The orbit breathes, so the blobs do not circle at a fixed distance and
      -- never touch.
      local orbit = s(spec.orbit) + math.sin(now * 0.00037 + spec.phase) * s(spec.wobble)
      node.x = W / 2 + math.cos(angle) * orbit - size / 2
      node.y = H / 2 + math.sin(angle) * orbit * 0.68 - size / 2
    end
  end
end

-- One field. One draw. The layers melt into each other exactly as in
-- `sdf-blobs.lua` — the blur does not change what the shape is.
local field = { x = 0, y = 0, width = W, height = H }
field.fill_color = "#f0b47a0d"
-- A thin bright edge, and it matters more the fainter the fill gets: at a tenth
-- of alpha the interior is barely there, so the rim is what says a blob is a
-- blob rather than a soft patch of nothing.
field.stroke_color = "#ffffff4a"
field.stroke_width = math.max(1.5, SHORT * 0.0016)

for index, spec in ipairs(BLOBS) do
  local size = s(spec.radius)
  blobs[index] = ui.SdfShape {
    x = W / 2,
    y = H / 2,
    width = size,
    height = size,
    shape = "circle",
    fill_color = spec.color,
    -- The first layer establishes the field; the rest melt into it.
    operation = index == 1 and "union" or "smooth_union",
    blend = BLEND,

    -- Blur the desktop behind this blob. The radii make the region a circle
    -- rather than the square the layer occupies.
    backdrop_blur = true,
    radius = size / 2,
  }
  field[#field + 1] = blobs[index]
end

advance()

ui.Item {
  width = W,
  height = H,
  ui.Sdf(field),

  ui.Timer {
    interval = 16,
    ["repeat"] = true,
    running = true,
    on_triggered = advance,
  },
}
