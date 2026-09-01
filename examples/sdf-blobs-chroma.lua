-- One merged field, with the colour split off to one side.
--
--     oslo make run --example examples/sdf-blobs-chroma.lua
--
-- The blobs melt into one another exactly as in `sdf-blobs.lua`: one `ui.Sdf`,
-- `smooth_union`, one surface, one draw. Nothing here gives that up.
--
-- A chromatic shift is one constant offset. The red channel is sampled a few
-- pixels one way and the blue a few pixels the other, and everything else is
-- left alone — so what you see is the same shape, the same size, with a
-- coloured ghost along one side of it and a complementary one along the other.
--
-- It is worth saying what this is *not*, because it is the mistake that looks
-- plausible: offsetting each channel radially, outward from a centre. That is a
-- real thing a real lens does, but it scales each channel's picture by a
-- different amount, so the shape appears to change size per channel and the
-- result reads as a blob that grew rather than a colour that shifted.
--
-- The offset is still per-blob: each carries its own direction, so the split
-- points a different way on each one and crosses over smoothly through a neck
-- where two have fused. And a blob only has a split while it is its own blob —
-- as two draw together it fades out, which is decided in Lua, because whether
-- two blobs are touching is a fact about where this file put them.

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

-- Six numbers a blob: centre x, centre y, radius, how alone it is, and the
-- direction its colour splits along. No colour here — the field keeps its own
-- layer fills, and a shift only moves what it is given.
local STRIDE = 6

--------------------------------------------------------------------------------
-- The lens.
--------------------------------------------------------------------------------

morf.shader("chromatic", {
  kind = "effect",
  -- Parameters arrive alphabetically whatever order they are declared in, so
  -- the entry point lists them that way: amount, jolt, pulse.
  params = {
    -- How far apart the red and blue are, in pixels. Everything inside the
    -- shader is in `uv`, where a plausible-looking number like 7 would mean
    -- seven times the whole screen, so the conversion happens in one place and
    -- this parameter means what a person reading it would think it means.
    amount = 9.0,
    -- Occasional horizontal tearing, in pixels: a torn signal, not a lens.
    jolt = 6.0,
    -- How much the separation breathes. 0 holds it still.
    pulse = 0.35,
  },
  data = { blobs = 36 },
  fragment = [[
    -- Enough bits for a believable stutter. An f32 has twenty-four of mantissa
    -- and these constants need thirty-two, so this is only writable at all
    -- because the shader language has real integers.
    function hash(seed)
      local h = seed * u32(747796405) + u32(2891336453)
      local word = ((h >> ((h >> u32(28)) + u32(4))) ~ h) * u32(277803737)
      return f32((word >> u32(22)) ~ word & u32(65535)) / 65535.0
    end

    function fragment(uv, time, resolution, amount, jolt, pulse)
      -- The surface is not square, so ownership has to be measured in one
      -- consistent unit or every blob's reach comes out an ellipse.
      local aspect = resolution.x / resolution.y
      local here = vec2(uv.x * aspect, uv.y)

      -- Which blob owns this pixel, and therefore which way its colour splits
      -- and whether it splits at all. Accumulated as a weighted sum, so through
      -- a neck the direction turns from one blob's to the other's instead of
      -- jumping.
      local weight = 0.0
      local solo = 0.0
      local dir = vec2(0.0, 0.0)
      local index = i32(0)
      while index < i32(6) do
        local at = index * i32(6)
        local centre = vec2(blobs[at] * aspect, blobs[at + 1])
        local radius = max(blobs[at + 2], 0.0001)
        local away = here - centre
        -- Reach a little past the blob's own edge, so ownership is handed over
        -- by the time the surface gets there — but only a little.
        local reach = radius * 1.35
        local near = clamp(1.0 - dot(away, away) / (reach * reach), 0.0, 1.0)
        -- Cubed rather than squared: whichever blob a pixel is actually in
        -- should win outright, and only genuinely shared ground should blend.
        -- Plus a floor that never quite reaches zero, because the merged
        -- surface bulges past every blob's reach in a deep neck and a pixel
        -- there would otherwise have no owner and no direction at all.
        local pull = 0.0015 / (dot(away, away) / (radius * radius) + 0.2)
        local w = near * near * near + pull
        weight = weight + w
        solo = solo + blobs[at + 3] * w
        dir = dir + vec2(blobs[at + 4], blobs[at + 5]) * w
        index = index + i32(1)
      end
      local total = max(weight, 0.0001)
      solo = solo / total
      dir = dir / total
      -- Back to a unit direction: the weights decided which way, not how far.
      dir = dir / max(length(dir), 0.0001)

      -- How far, in pixels, and then the same offset for every pixel of this
      -- blob. Constant is the whole point — a shift that grew with distance
      -- from some centre would scale the channels instead of displacing them,
      -- and the shape would appear to change size rather than the colour to
      -- come apart.
      local breathe = 1.0 + sin(time * 0.9) * pulse
      local pixels = amount * breathe * solo
      local shift = vec2(dir.x * pixels / resolution.x, dir.y * pixels / resolution.y)

      -- A few rows torn sideways for a moment. The band and the slide both come
      -- from the clock, so this is one more thing the configuration never has
      -- to drive.
      local band = floor(uv.y * 24.0)
      local tick = floor(time * 12.0)
      local tear = hash(u32(band) * u32(374761393) + u32(tick) * u32(668265263))
      -- Almost always zero: the point of a glitch is that it is rare. The
      -- surviving seven percent is stretched back to a full zero-to-one so
      -- `jolt` can be read as the pixels it actually moves.
      local torn = min(max(tear - 0.93, 0.0) * 14.3, 1.0)
      local slide = vec2(torn * jolt * solo / resolution.x, 0.0)

      -- Red one way, blue the other, green where it was. Three samples of the
      -- same picture at three places — nothing is scaled, nothing is added.
      local r = texture(uv + shift + slide)
      local g = texture(uv + slide)
      local b = texture(uv - shift + slide)

      -- Alpha from whichever sample found something, so the ghosts are visible
      -- past the edge of the shape rather than clipped to it. That overhang on
      -- either side is the effect.
      return vec4(r.x, g.y, b.z, max(r.w, max(g.w, b.w)))
    end
  ]],
})

--------------------------------------------------------------------------------
-- The lamp, merged exactly as before.
--------------------------------------------------------------------------------

local BLOBS = {
  { radius = 0.150, orbit = 0.185, speed = 0.00040, phase = 0.0, wobble = 0.052, tilt = 0.4, color = "#f0b47a" },
  { radius = 0.122, orbit = 0.225, speed = -0.00057, phase = 1.9, wobble = 0.066, tilt = 2.1, color = "#e8735a" },
  { radius = 0.104, orbit = 0.152, speed = 0.00079, phase = 3.4, wobble = 0.038, tilt = 3.9, color = "#b4e1ea" },
  { radius = 0.088, orbit = 0.248, speed = -0.00098, phase = 5.0, wobble = 0.074, tilt = 5.2, color = "#7fb7c9" },
  { radius = 0.074, orbit = 0.118, speed = 0.00121, phase = 2.4, wobble = 0.044, tilt = 1.2, color = "#f5d98b" },
  { radius = 0.062, orbit = 0.272, speed = -0.00142, phase = 0.7, wobble = 0.058, tilt = 4.6, color = "#c98fd1" },
}

local elapsed = core.elapsed_timer()
local blobs = {}
local lens
local places = {}
local centres = {}

-- The seam radius the field melts its layers with, and the distance over which
-- a blob counts as losing its own rim. Fusion has no instant: the field starts
-- bulging one surface towards another well before they meet, so the lens has to
-- start going before they meet too.
local BLEND = s(0.038)
local NECK = BLEND * 2.5

--- Moves every blob, works out how alone each one is, and tells the lens.
local function advance()
  local now = elapsed:elapsed_ms()

  -- Where everything is, before anything can be said about what it is near.
  for index, spec in ipairs(BLOBS) do
    local node = blobs[index]
    if node then
      local size = s(spec.radius)
      local angle = spec.phase + now * spec.speed
      -- The orbit breathes, so the blobs do not circle at a fixed distance and
      -- never touch.
      local orbit = s(spec.orbit) + math.sin(now * 0.00037 + spec.phase) * s(spec.wobble)
      local x = W / 2 + math.cos(angle) * orbit - size / 2
      local y = H / 2 + math.sin(angle) * orbit * 0.68 - size / 2
      node.x = x
      node.y = y

      local centre = centres[index] or {}
      centre.x = x + size / 2
      centre.y = y + size / 2
      centre.r = size / 2
      centres[index] = centre
    end
  end

  -- How alone each one is: the gap to its nearest neighbour's *surface* rather
  -- than its centre, so a big blob and a small one are judged the same way.
  -- Six blobs is thirty comparisons a tick, which is nothing — and the
  -- alternative is the shader rediscovering it for every pixel on the screen,
  -- sixty times a second.
  for index = 1, #BLOBS do
    local mine = centres[index]
    if mine then
      local nearest = math.huge
      for other = 1, #BLOBS do
        if other ~= index and centres[other] then
          local theirs = centres[other]
          local dx = mine.x - theirs.x
          local dy = mine.y - theirs.y
          local gap = math.sqrt(dx * dx + dy * dy) - (mine.r + theirs.r)
          if gap < nearest then
            nearest = gap
          end
        end
      end

      -- In the units the shader measures in: `uv` for the centres, and the
      -- height for the radius, because that is the axis the aspect correction
      -- leaves alone.
      local base = (index - 1) * STRIDE
      places[base + 1] = mine.x / W
      places[base + 2] = mine.y / H
      places[base + 3] = mine.r / H
      places[base + 4] = math.max(0, math.min(1, nearest / NECK))
      -- Which way this blob's colour comes apart. It turns slowly, so the
      -- split is not a fixed artefact of the file but something the lamp does.
      local tilt = BLOBS[index].tilt + now * 0.00009
      places[base + 5] = math.cos(tilt)
      places[base + 6] = math.sin(tilt)
    end
  end

  if lens then
    morf.shader_data(lens, "blobs", places)
  end
end

-- One field. One draw. The layers melt into each other, and keep their colours.
local field = { x = 0, y = 0, width = W, height = H }
field.fill_color = "#f0b47a"
field.stroke_color = "#8a4a17"
field.stroke_width = math.max(2, SHORT * 0.0028)
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
    -- About a third of the smallest blob: enough that two passing close draw a
    -- neck between them, not so much that the swarm reads as one skin.
    blend = BLEND,
  }
  field[#field + 1] = blobs[index]
end

-- The lens wraps the field rather than lying over it: an effect shader samples
-- the layer its own node became, and a layer holds that node's subtree.
lens = ui.Item {
  width = W,
  height = H,
  shader = "chromatic",
  -- A transparent rectangle the size of the surface, so the layer's bounds are
  -- the screen and not whatever the blobs happen to span this frame. Without
  -- it `uv` would be measured against a box that moves, and the blob positions
  -- the shader is handed would mean something different every frame.
  ui.Rect { width = W, height = H, color = "#00000000" },
  ui.Sdf(field),
}

-- Fill the block before the first frame, so nothing is drawn against zeroes.
advance()

ui.Item {
  width = W,
  height = H,
  lens,

  ui.Timer {
    interval = 16,
    ["repeat"] = true,
    running = true,
    on_triggered = advance,
  },
}
