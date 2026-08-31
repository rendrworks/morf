-- A lava lamp held together by nothing but its own gravity.
--
-- Nothing here writes a position. Each blob is thrown once and then coasts,
-- and the only thing Lua does afterwards is work out which way each blob would
-- like to be heading and hand the difference to the engine as an impulse.
-- Forces come from the configuration, integration stays in the engine: the
-- steering is recomputed about thirty times a second, and every frame in
-- between is moved by morf.
--
-- There is no gravity in it, downwards or between the blobs. Attraction is a
-- force with no upper bound, and a handful of blobs pulling on each other only
-- ever slingshot apart or fall together into one lump. They *steer* instead:
-- each turns towards where it wants to be at a bounded rate, and what it wants
-- is to go round the middle, keep clear of its neighbours, and drift a little.
-- That is Reynolds' flocking, and it is why the swarm can neither collapse nor
-- escape however its parts happen to line up.
--
-- `MORF_BLOB_LAYER` picks where it sits: `bottom` (the default) puts it above
-- the wallpaper and beneath the windows; `overlay` floats it over everything.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080
local SHORT = math.min(W, H)

-- The whole output. The surface paints nothing but the fluid, so everywhere it
-- does not reach stays transparent and the desktop shows through; and with no
-- interactive node in the tree the input region derives empty, so every click
-- passes straight to whatever is underneath.
local SURFACE_W = W
local SURFACE_H = H
morf.surface.width = SURFACE_W
morf.surface.height = SURFACE_H
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = core.env("MORF_BLOB_LAYER") or "bottom"
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1

--- Sizes are fractions of the short side, so the lamp fills any output alike.
local function s(fraction) return SHORT * fraction end

-- The field spans the screen, but what it costs is the area its layers cover,
-- not the node: the quad is sized to the blobs and everything outside them is
-- never asked about.
local FIELD_W = SURFACE_W
local FIELD_H = SURFACE_H

-- Small, and spread wide below. This is the whole difference between a lamp
-- and a puddle: six blobs whose radii add up to more than the ring they orbit
-- on cannot be six blobs, whatever the physics does, because they are always
-- touching. They have to be smaller than the room they move in.
-- Small, and spread wide below. Six blobs whose radii add up to more than the
-- space they move in cannot look like six blobs, whatever the forces do,
-- because they are always touching.
local BLOBS = {
  { radius = 0.255, color = "#f0b47a" },
  { radius = 0.210, color = "#e8735a" },
  { radius = 0.180, color = "#b4e1ea" },
  { radius = 0.156, color = "#7fb7c9" },
  { radius = 0.135, color = "#f5d98b" },
  { radius = 0.114, color = "#c98fd1" },
}

-- Where the swarm hangs, and how far out it is allowed to wander before
-- anything asks it to come back.
local CENTER_X = FIELD_W / 2
local CENTER_Y = FIELD_H / 2
-- As wide as the screen allows once the biggest blob has to fit inside it.
-- Bigger blobs need more room, not the same room; deriving this rather than
-- picking it is what lets the sizes above be changed without the swarm turning
-- back into a single lump.
local BOWL = math.min(FIELD_W, FIELD_H) / 2 - s(BLOBS[1].radius) / 2

-- How fast a blob wants to be going: about two pixels a frame, which is the
-- speed that reads as drifting rather than as thrown.
local CRUISE = s(0.125)
-- How sharply it may change its mind, per second. This is the number that
-- keeps the whole thing calm: no force here can turn a blob faster than this
-- however close it gets to another one, which is exactly what an attraction
-- between them could not promise.
local AGILITY = CRUISE * 1.6

-- What the blobs are trying to do, in the proportions they are trying to do
-- it. Each is a direction; they are added up and the result decides where a
-- blob would like to be heading, never how fast — the speed is always CRUISE.
local SWIRL = 1.0 -- round the middle: the spin
local HOLD = 0.7 -- in or out, onto the ring this blob belongs on
-- Deliberately the loudest of them. Everything else here is a preference;
-- this one is the difference between six blobs and one, so when a blob is
-- being crowded it has to out-argue the ring it is supposed to be holding.
local KEEP_APART = 3.2
local FOLLOW = 0.12 -- along with the neighbours, which is what makes it fluid
local WANDER = 0.45 -- and a little of nothing in particular

-- How far a blob's own pace may wander in one step. Small, so that a blob
-- speeds up and slows down over seconds rather than flickering between the
-- two — the eye reads the first as a current in the fluid and the second as a
-- fault in the animation.
local DRIFT = 0.035

-- Away from the edge of the screen, and loud enough to be obeyed. Blobs are
-- pushed outwards by their neighbours as well as held on their rings, and
-- without this the crowding wins near the rim and a blob ends up sliced flat
-- against the edge of the surface.
local EDGE = 4.0

--- A number near one, drawn once. `spread` is how far either side it may fall.
local function varied(spread) return 1 + (math.random() * 2 - 1) * spread end

-- Everything below is drawn rather than decided, because every output runs
-- this file in its own runtime with its own random stream. Deciding it — one
-- fixed size per blob, one fixed ring, one fixed direction — is what made
-- three monitors show the same lamp three times.
local SPIN = math.random() < 0.5 and 1 or -1 -- which way round this one turns
local PHASE = math.random() * 2 * math.pi -- and where it starts from

local blobs = {}
local size_of = {}
for index, spec in ipairs(BLOBS) do
  size_of[index] = s(spec.radius) * varied(0.16)
end

--- How close two blobs' centres are when their edges touch.
local function touching(a, b) return size_of[a] / 2 + size_of[b] / 2 end

--- How close is too close.
---
--- Only a little beyond touching, so that two blobs on a close pass do reach
--- each other and draw a neck between them before they ease apart. Held any
--- further off they would never touch at all, and the whole point of drawing
--- them as one field would be lost.
local function comfort(a, b) return touching(a, b) * 1.25 end

-- Where each starts, and how it is first thrown: evenly round the bowl, each
-- one across its own radius, so the swarm is already turning on frame one.
local start_x, start_y, start_vx, start_vy, ring_of, pace = {}, {}, {}, {}, {}, {}
for index = 1, #BLOBS do
  local angle = PHASE + ((index - 1) / #BLOBS + math.random() * 0.12) * 2 * math.pi
  -- Short of the bowl, not out to it: blobs this size shoulder each other
  -- outwards as well, and a ring drawn at the limit leaves that nowhere to go
  -- but over the edge of the screen.
  local reach = BOWL * (0.22 + 0.58 * (index - 1) / (#BLOBS - 1)) * varied(0.18)
  -- How fast this one likes to go. It drifts from here as the lamp runs, so
  -- the blob that is currently the quickest is not always the same blob.
  pace[index] = varied(0.3)
  start_x[index] = CENTER_X + math.cos(angle) * reach
  start_y[index] = CENTER_Y + math.sin(angle) * reach
  start_vx[index] = -math.sin(angle) * CRUISE * SPIN * pace[index]
  start_vy[index] = math.cos(angle) * CRUISE * SPIN * pace[index]
  -- The ring it belongs on. Giving every blob its own is what spreads the
  -- swarm out and keeps it spread: a flock that is only told to stay near the
  -- middle bunches up there, and one that is only told to stay inside a bowl
  -- drifts out to the edge of it and sits on the rim.
  ring_of[index] = reach
end

--- Throws one blob, into a world with nothing in it but walls.
---
--- Everything that moves it afterwards arrives as an impulse. `min` and `max`
--- should never come up — the bowl turns the swarm back long before — but a
--- bound that returns what it takes makes an escape recoverable.
local function throw(index)
  local size = size_of[index]
  morf.animation.fling {
    node = blobs[index],
    property = "x",
    velocity = start_vx[index],
    friction = 0,
    min_velocity = 0,
    bounce = 1.0,
    min = 0,
    max = FIELD_W - size,
  }
  morf.animation.fling {
    node = blobs[index],
    property = "y",
    velocity = start_vy[index],
    friction = 0,
    min_velocity = 0,
    bounce = 1.0,
    min = 0,
    max = FIELD_H - size,
  }
end

local was_x, was_y = {}, {}
for index = 1, #BLOBS do
  was_x[index], was_y[index] = start_x[index], start_y[index]
end

--- Steers every blob once.
---
--- Not attraction. Two blobs pulling on each other is a force with no upper
--- bound: the closer they get the harder they pull, so they slingshot, or they
--- fall together and stay there, and no amount of tuning fixes either because
--- the tuning is a fight with an infinity. What each blob does here instead is
--- work out the direction it would *like* to be going and turn towards it at a
--- bounded rate. Nothing can exceed `AGILITY`, nothing settles at a speed but
--- `CRUISE`, and the swarm cannot collapse or fly apart however the pieces
--- happen to line up. This is Reynolds' flocking, which is what everything that
--- looks like a swarm on screen has been doing since 1986.
---
--- Reading `node.x` reads the live animated position, so this sees exactly
--- where the engine has moved things since the last step, and the impulse it
--- hands back is that steering over `dt`. Between two of these the engine is
--- moving them on its own, which is why a step this coarse still looks smooth.
local function steer(dt)
  local n = #blobs
  local cx, cy, vx, vy = {}, {}, {}, {}
  local flock_vx, flock_vy = 0, 0
  for index = 1, n do
    local half = size_of[index] / 2
    cx[index] = blobs[index].x + half
    cy[index] = blobs[index].y + half
    vx[index] = (cx[index] - was_x[index]) / dt
    vy[index] = (cy[index] - was_y[index]) / dt
    was_x[index], was_y[index] = cx[index], cy[index]
    flock_vx = flock_vx + vx[index]
    flock_vy = flock_vy + vy[index]
  end
  flock_vx, flock_vy = flock_vx / n, flock_vy / n

  for index = 1, n do
    local px, py = cx[index], cy[index]
    local wish_x, wish_y = 0, 0

    -- A slow walk rather than a fresh number: a speed redrawn every step is
    -- noise and averages out to nothing, but one that wanders from where it
    -- was means a blob is quick for a while and then is not.
    pace[index] = math.max(math.min(pace[index] + (math.random() * 2 - 1) * DRIFT, 1.55), 0.55)

    -- Round the middle. Perpendicular to the line out from the centre, which
    -- is a circle's own direction of travel — this alone is the spin.
    local out_x, out_y = px - CENTER_X, py - CENTER_Y
    local out = math.max(math.sqrt(out_x * out_x + out_y * out_y), 0.001)
    wish_x = wish_x + SWIRL * SPIN * -out_y / out
    wish_y = wish_y + SWIRL * SPIN * out_x / out

    -- In or out, onto its own ring. Not a wall it is kept inside but a circle
    -- it is held on from both sides, which is what makes this an orbit rather
    -- than a swarm rattling around in a bowl — and it is why the lamp neither
    -- inflates nor wanders off, without anything having to conserve momentum.
    local stray = math.max(math.min((out - ring_of[index]) / ring_of[index], 1.0), -1.0)
    wish_x = wish_x - HOLD * stray * out_x / out
    wish_y = wish_y - HOLD * stray * out_y / out

    -- Away from anyone too close, weighted by how close, and averaged over
    -- all of them rather than taken from the nearest. The average is what
    -- makes this smooth: a blob crowded from two sides eases out between them
    -- instead of flinching away from whichever one is momentarily nearer.
    local apart_x, apart_y, crowd = 0, 0, 0
    for other = 1, n do
      if other ~= index then
        local dx, dy = px - cx[other], py - cy[other]
        local dist = math.max(math.sqrt(dx * dx + dy * dy), 0.001)
        local room = comfort(index, other)
        if dist < room then
          local urgency = (room - dist) / room
          apart_x = apart_x + urgency * dx / dist
          apart_y = apart_y + urgency * dy / dist
          crowd = crowd + 1
        end
      end
    end
    if crowd > 0 then
      wish_x = wish_x + KEEP_APART * apart_x / crowd
      wish_y = wish_y + KEEP_APART * apart_y / crowd
    end

    -- Off the edge of the screen. Nothing else here knows the surface exists:
    -- the rings are about the middle, and being crowded is about neighbours.
    local margin = size_of[index] / 2 + s(0.015)
    if px < margin then
      wish_x = wish_x + EDGE * (margin - px) / margin
    elseif px > FIELD_W - margin then
      wish_x = wish_x - EDGE * (px - (FIELD_W - margin)) / margin
    end
    if py < margin then
      wish_y = wish_y + EDGE * (margin - py) / margin
    elseif py > FIELD_H - margin then
      wish_y = wish_y - EDGE * (py - (FIELD_H - margin)) / margin
    end

    -- Along with everyone else. This is the one that reads as viscosity: a
    -- blob shoved aside by a neighbour drags a little of the neighbour's
    -- motion with it rather than simply bouncing off.
    local speed = math.max(math.sqrt(flock_vx * flock_vx + flock_vy * flock_vy), 0.001)
    wish_x = wish_x + FOLLOW * flock_vx / speed
    wish_y = wish_y + FOLLOW * flock_vy / speed

    -- And a little of nothing in particular, or the swarm finds an arrangement
    -- it likes and the lamp turns into clockwork.
    wish_x = wish_x + WANDER * (math.random() * 2 - 1)
    wish_y = wish_y + WANDER * (math.random() * 2 - 1)

    -- The steer: the difference between the speed it wants and the speed it
    -- has, capped. Capping *this* rather than the speed is what keeps the
    -- motion continuous — a blob is always turning towards what it wants at a
    -- rate an eye can follow, never being snapped onto it.
    local wish = math.sqrt(wish_x * wish_x + wish_y * wish_y)
    if wish > 0.001 then
      local want = CRUISE * pace[index]
      local turn_x = wish_x / wish * want - vx[index]
      local turn_y = wish_y / wish * want - vy[index]
      local turn = math.sqrt(turn_x * turn_x + turn_y * turn_y)
      local cap = math.min(AGILITY / math.max(turn, 0.001), 1.0)
      morf.animation.impulse(blobs[index], "x", turn_x * cap * dt)
      morf.animation.impulse(blobs[index], "y", turn_y * cap * dt)
    end
  end
end

local field = { x = 0, y = 0, width = FIELD_W, height = FIELD_H }
-- Only the fallback: every layer below names its own fill.
field.fill_color = "#f0b47a"
field.stroke_color = "#8a4a17"
field.stroke_width = math.max(2, SHORT * 0.0028)
for index, spec in ipairs(BLOBS) do
  local size = size_of[index]
  blobs[index] = ui.SdfShape {
    x = start_x[index] - size / 2,
    y = start_y[index] - size / 2,
    width = size,
    height = size,
    shape = "circle",
    fill_color = spec.color,
    operation = index == 1 and "union" or "smooth_union",
    -- A third of the smallest blob: enough that two passing close draw out a
    -- neck between them, not so much that the whole swarm reads as one skin.
    blend = s(0.038),
  }
  field[#field + 1] = blobs[index]
end

local STEP = 32

ui.Item {
  width = SURFACE_W,
  height = SURFACE_H,
  -- No background: what is not painted stays transparent.
  ui.Sdf(field),

  ui.Timer {
    interval = STEP,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      steer(STEP / 1000)
      for index, node in ipairs(blobs) do
        -- A blob only stops if it somehow lost all its speed at a wall. It
        -- should not happen; if it does, put it back in the swarm rather than
        -- leave a dead lump in the corner.
        if not morf.animation.active(node, "x") and not morf.animation.active(node, "y") then
          throw(index)
        end
      end
    end,
  },
}

for index = 1, #BLOBS do
  throw(index)
end
