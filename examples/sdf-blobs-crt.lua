-- One merged field. Every blob its own tube.
--
--     oslo make run --example examples/sdf-blobs-crt.lua
--
-- The blobs melt into one another exactly as in `sdf-blobs.lua`: one `ui.Sdf`,
-- `smooth_union`, one surface, one draw. That is not negotiable and nothing
-- here gives it up.
--
-- The tube is still per-blob, and the way that works is the interesting part. A
-- field is one draw, so it can only carry one shader — but the shader is *told
-- where the blobs are*, through a data block the configuration refills each
-- tick. For any pixel it works out how much each blob owns it, and shades in
-- that blob's own local coordinates: its own curved glass, its own scanlines,
-- its own vignette, its own colour. Through a neck where two blobs have fused,
-- the ownership crosses over and one tube becomes the other, which is a thing
-- separate nodes could not do at all.
--
-- So the per-blob look does not come from splitting the field up. It comes from
-- the shader knowing what the field is made of.
--
-- And a blob only wears a tube while it is *its own* blob. Two that have drawn
-- together are one surface, and one surface is not two screens — so as they
-- fuse the tube dissolves back to plain lava, and re-forms as they part. That
-- decision is made here in Lua rather than in the shader, because whether two
-- blobs are touching is a fact about where this file put them: `advance` has
-- the centres and the radii in hand, and the shader would only be working out
-- again what was already known.

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

local COUNT = 6
-- Seven numbers a blob: centre x, centre y, radius, its colour, and how
-- alone it is.
local STRIDE = 7

--------------------------------------------------------------------------------
-- The tube.
--------------------------------------------------------------------------------

morf.shader("crt", {
  -- Parameters arrive alphabetically whatever order they are declared in, so
  -- the entry point lists them that way: lines, mask, roll, scan.
  params = {
    -- Scanlines across one blob, not across the screen: a bigger blob is a
    -- bigger tube and gets bigger lines.
    lines = 52.0,
    -- Strength of the red/green/blue phosphor stripes.
    mask = 0.18,
    -- Speed of the bright band rolling down a face.
    roll = 0.22,
    -- Depth of the scanline gaps.
    scan = 0.30,
  },
  -- Where the blobs are, what colour they are, and how alone each one is,
  -- refilled every tick by the configuration. This is the whole mechanism:
  -- without it the shader knows only the merged silhouette, which by
  -- construction no longer remembers what it was made of.
  data = { blobs = 42 },
  fragment = [[
    function fragment(uv, time, resolution, coverage, lines, mask, roll, scan)
      -- `uv` is the surface, which is not square, so distances have to be
      -- measured in one consistent unit or every blob comes out an ellipse.
      local aspect = resolution.x / resolution.y
      local here = vec2(uv.x * aspect, uv.y)

      -- How much each blob owns this pixel, what colour it is, how alone it is,
      -- and what its own coordinates are here. All accumulated as a weighted
      -- sum, so where two blobs have fused the answer is genuinely between the
      -- two of them rather than one or the other with a seam down the middle.
      local weight = 0.0
      local base = vec3(0.0, 0.0, 0.0)
      local solo = 0.0
      local mine = vec2(0.0, 0.0)
      local index = i32(0)
      while index < i32(6) do
        local at = index * i32(7)
        local centre = vec2(blobs[at] * aspect, blobs[at + 1])
        local radius = max(blobs[at + 2], 0.0001)
        local away = here - centre
        -- Reach a little past the blob's own edge, so ownership is handed over
        -- by the time the surface gets there — but only a little. Reaching far
        -- means a blob still has a say in the coordinates used on its
        -- neighbour, and then neither is shaded about its own centre.
        local reach = radius * 1.35
        local near = clamp(1.0 - dot(away, away) / (reach * reach), 0.0, 1.0)
        -- Cubed rather than squared: whichever blob a pixel is actually in
        -- should win outright, and only genuinely shared ground should blend.
        --
        -- Plus a floor that falls off with distance but never quite reaches
        -- zero. The merged surface bulges out past every blob's reach in a deep
        -- neck, and a pixel there would otherwise have no owner at all — which
        -- is not a slightly wrong answer but a division by nothing.
        local pull = 0.0015 / (dot(away, away) / (radius * radius) + 0.2)
        local w = near * near * near + pull
        weight = weight + w
        base = base + vec3(blobs[at + 3], blobs[at + 4], blobs[at + 5]) * w
        solo = solo + blobs[at + 6] * w
        -- This blob's own square, centred on it: the tube's coordinates.
        mine = mine + (away / (radius * 2.0) + vec2(0.5, 0.5)) * w
        index = index + i32(1)
      end
      local total = max(weight, 0.0001)
      base = base / total
      solo = solo / total
      mine = mine / total

      -- From here on it is one tube, in the coordinates of whichever blob owns
      -- this pixel. Its glass bows around its own centre.
      local p = mine * 2.0 - vec2(1.0, 1.0)
      local bent = (p + p * vec2(p.y * p.y, p.x * p.x) * 0.18) * 0.5 + vec2(0.5, 0.5)

      -- The blob's own colour, and then the tube laid over it. Nothing here
      -- invents a colour: a lamp that came out in primaries would not be the
      -- lamp any more, whatever else was right about it.
      local lit = base

      local beam = sin(bent.y * lines) * 0.5 + 0.5
      lit = lit * (1.0 - scan * beam)

      -- The shadow mask is the one thing measured on the real screen rather
      -- than on the blob: it is the physical phosphor, and it does not move
      -- when a blob does.
      local stripe = i32(floor(uv.x * resolution.x)) % 3
      local tint = vec3(1.0, 1.0, 1.0)
      if stripe == 0 then
        tint = vec3(1.0, 1.0 - mask, 1.0 - mask)
      elseif stripe == 1 then
        tint = vec3(1.0 - mask, 1.0, 1.0 - mask)
      else
        tint = vec3(1.0 - mask, 1.0 - mask, 1.0)
      end
      lit = lit * tint

      -- A bright band drifting down this blob's face alone.
      local band = fract(bent.y - time * roll)
      lit = lit + base * smoothstep(0.93, 1.0, band) * 0.30

      -- Its own corners, further from its own gun. Gentle: a vignette deep
      -- enough to notice on its own is deep enough to look like dirt.
      local off = bent - vec2(0.5, 0.5)
      lit = lit * clamp(1.12 - dot(off, off) * 1.0, 0.0, 1.0)

      -- Where blobs have drawn together there is no tube, only lava. Because
      -- solitude was blended by the same ownership weights as everything else,
      -- this fades across a neck rather than switching at a line — the glass
      -- thins towards the join and is gone by the middle of it.
      lit = mix(base, lit, solo)

      -- The merged field still decides where any of this lands. A material
      -- shader colours inside the coverage it was handed and nowhere else, so
      -- the silhouette is the fused one, necks and all.
      return vec4(lit, 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- The lamp, merged exactly as before.
--------------------------------------------------------------------------------

local BLOBS = {
  { radius = 0.150, orbit = 0.185, speed = 0.00040, phase = 0.0, wobble = 0.052, color = "#f0b47a" },
  { radius = 0.122, orbit = 0.225, speed = -0.00057, phase = 1.9, wobble = 0.066, color = "#e8735a" },
  { radius = 0.104, orbit = 0.152, speed = 0.00079, phase = 3.4, wobble = 0.038, color = "#b4e1ea" },
  { radius = 0.088, orbit = 0.248, speed = -0.00098, phase = 5.0, wobble = 0.074, color = "#7fb7c9" },
  { radius = 0.074, orbit = 0.118, speed = 0.00121, phase = 2.4, wobble = 0.044, color = "#f5d98b" },
  { radius = 0.062, orbit = 0.272, speed = -0.00142, phase = 0.7, wobble = 0.058, color = "#c98fd1" },
}

local elapsed = core.elapsed_timer()
local blobs = {}
local field_node
local levels = {}
local centres = {}

-- The seam radius the field melts its layers with, and the distance over which
-- a blob counts as losing its own identity. Fusion is not an event with an
-- instant: the field starts bulging one surface towards another well before
-- they meet, so the tube has to start going before they meet too, or it would
-- still be there on a shape that has already stopped being one blob.
local BLEND = s(0.038)
local NECK = BLEND * 2.5

--- Moves every blob, works out how alone each one is, and tells the shader.
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

  -- How alone each one is: the gap to its nearest neighbour's *surface*, not
  -- its centre, so a big blob and a small one are judged the same way. Below
  -- zero they overlap; past the neck distance the field is not drawing
  -- anything between them at all. Six blobs is thirty comparisons a tick,
  -- which is nothing — and the alternative is the shader rediscovering it for
  -- every pixel on the screen, sixty times a second.
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
      local solitude = math.max(0, math.min(1, nearest / NECK))

      -- In the units the shader measures in: `uv` for the centres, and the
      -- height for the radius, because that is the axis the aspect correction
      -- leaves alone.
      local base = (index - 1) * STRIDE
      levels[base + 1] = mine.x / W
      levels[base + 2] = mine.y / H
      levels[base + 3] = mine.r / H
      -- A shader is handed raw floats and converts nothing, so the colour
      -- goes in as linear light or every blob comes out washed out.
      local linear = morf.color(BLOBS[index].color):linear()
      local red, green, blue = linear.r, linear.g, linear.b
      levels[base + 4] = red
      levels[base + 5] = green
      levels[base + 6] = blue
      levels[base + 7] = solitude
    end
  end

  if field_node then
    morf.shader_data(field_node, "blobs", levels)
  end
end

-- One field. One draw. The layers melt into each other.
local field = { x = 0, y = 0, width = W, height = H, shader = "crt" }
-- Only the fallback: the shader decides the colour, but a field still wants one.
field.fill_color = "#f0b47a"
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

field_node = ui.Sdf(field)

-- Fill the block before the first frame, so nothing is ever drawn against a
-- table of zeroes.
advance()

ui.Item {
  width = W,
  height = H,
  field_node,

  ui.Timer {
    interval = 16,
    ["repeat"] = true,
    running = true,
    on_triggered = advance,
  },
}
