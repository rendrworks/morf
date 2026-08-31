-- The same fluid as `sdf-blobs.lua`, with the flocking taken out and a charge
-- put in. Every blob carries one, slowly swinging between positive and
-- negative on its own clock; like charges push apart and opposite ones pull
-- together, so the swarm gathers into a knot, breaks up, drifts, and gathers
-- somewhere else, and it never does it the same way twice.
--
-- Where `sdf-blobs.lua` decides a direction and turns towards it, this decides
-- a *force* — but it is still handed over as a steer with a cap on it, which is
-- the only reason it can be gravity without being a disaster. Attraction has no
-- upper bound: two blobs pulling on each other directly get a harder pull the
-- closer they are, so they slingshot away or fall together and stay there.
-- Bounding the turn instead means the pull can point wherever the charges say
-- while the motion it produces stays something an eye can follow.
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

local SURFACE_W = W
local SURFACE_H = H
mold.surface.width = SURFACE_W
mold.surface.height = SURFACE_H
mold.surface.anchors = { top = true, left = true, right = true, bottom = true }
mold.surface.layer = core.env("MOLD_BLOB_LAYER") or "bottom"
mold.surface.keyboard_focus = "none"
mold.surface.exclusive_zone = -1

--- Sizes are fractions of the short side, so it fills any output alike.
local function s(fraction) return SHORT * fraction end

local FIELD_W = SURFACE_W
local FIELD_H = SURFACE_H

-- Two colours each: what it looks like pulling, and what it looks like pushing.
-- The charge is otherwise invisible, and a lamp whose blobs suddenly refuse
-- each other for no visible reason reads as a bug rather than as a force.
local BLOBS = {
  { radius = 0.255, pulling = "#f0b47a", pushing = "#8f6bd6" },
  { radius = 0.210, pulling = "#e8735a", pushing = "#5a7fe8" },
  { radius = 0.180, pulling = "#f5d98b", pushing = "#6fd6c4" },
  { radius = 0.156, pulling = "#ef9a6a", pushing = "#8f7ae0" },
  { radius = 0.135, pulling = "#f2c07a", pushing = "#5fb4e0" },
  { radius = 0.114, pulling = "#e88a72", pushing = "#7a9ee8" },
}

local CENTER_X = FIELD_W / 2
local CENTER_Y = FIELD_H / 2
local BOWL = math.min(FIELD_W, FIELD_H) / 2 - s(BLOBS[1].radius) / 2

-- Slower than the flocking lamp. There the speed is the point; here the point
-- is watching a knot decide to come apart, which wants time to read.
local CRUISE = s(0.10)
local AGILITY = CRUISE * 1.5

local CHARGE = 2.4 -- pull together, or push apart, as the charges say
local KEEP_APART = 3.4 -- and never actually stack, whatever the charges say
local HOME = 0.7 -- back towards the middle
local EDGE = 4.0 -- and never off the screen
local WANDER = 0.3

-- How long a blob takes to swing from pulling to pushing and back, in seconds.
-- Different per blob and drawn per output, so the swarm is never all of one
-- mind and no two screens are in step.
local CYCLE_LOW = 7.0
local CYCLE_HIGH = 19.0

--- A number near one, drawn once. `spread` is how far either side it may fall.
local function varied(spread) return 1 + (math.random() * 2 - 1) * spread end

local blobs = {}
local size_of = {}
for index, spec in ipairs(BLOBS) do
  size_of[index] = s(spec.radius) * varied(0.16)
end

--- How close two blobs' centres are when their edges touch.
local function touching(a, b) return size_of[a] / 2 + size_of[b] / 2 end

-- The charge itself: a phase that advances, read as a sine. Continuous, so a
-- blob does not flip from pulling to pushing between one step and the next —
-- it eases through neutral, and near neutral it barely takes part at all.
local phase, rate, showing = {}, {}, {}

local start_x, start_y, start_vx, start_vy, pace = {}, {}, {}, {}, {}
local PHASE = math.random() * 2 * math.pi
for index = 1, #BLOBS do
  local angle = PHASE + ((index - 1) / #BLOBS + math.random() * 0.12) * 2 * math.pi
  local reach = BOWL * (0.35 + 0.65 * (index - 1) / (#BLOBS - 1)) * varied(0.18)
  pace[index] = varied(0.25)
  phase[index] = math.random() * 2 * math.pi
  rate[index] = 2 * math.pi / (CYCLE_LOW + math.random() * (CYCLE_HIGH - CYCLE_LOW))
  showing[index] = nil
  start_x[index] = CENTER_X + math.cos(angle) * reach
  start_y[index] = CENTER_Y + math.sin(angle) * reach
  start_vx[index] = -math.sin(angle) * CRUISE * pace[index]
  start_vy[index] = math.cos(angle) * CRUISE * pace[index]
end

--- Throws one blob. Everything after this arrives as an impulse.
local function throw(index)
  local size = size_of[index]
  mold.animation.fling {
    node = blobs[index],
    property = "x",
    velocity = start_vx[index],
    friction = 0,
    min_velocity = 0,
    bounce = 1.0,
    min = 0,
    max = FIELD_W - size,
  }
  mold.animation.fling {
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

--- Advances every charge, and recolours anything that has changed its mind.
---
--- The colour is assigned, not animated here: the shapes carry a behavior on
--- `fill_color`, so the engine walks them across over a second and a half. A
--- blob is well past neutral before it changes colour, which keeps one sitting
--- near zero from flickering between the two.
local function recharge(dt)
  for index = 1, #blobs do
    phase[index] = (phase[index] + rate[index] * dt) % (2 * math.pi)
    local charge = math.sin(phase[index])
    local sign = charge > 0.15 and "pulling" or charge < -0.15 and "pushing" or showing[index]
    if sign and sign ~= showing[index] then
      showing[index] = sign
      blobs[index].fill_color = BLOBS[index][sign]
    end
  end
end

--- Steers every blob once, under whatever the charges currently say.
local function steer(dt)
  local n = #blobs
  local cx, cy, vx, vy = {}, {}, {}, {}
  for index = 1, n do
    local half = size_of[index] / 2
    cx[index] = blobs[index].x + half
    cy[index] = blobs[index].y + half
    vx[index] = (cx[index] - was_x[index]) / dt
    vy[index] = (cy[index] - was_y[index]) / dt
    was_x[index], was_y[index] = cx[index], cy[index]
  end

  for index = 1, n do
    local px, py = cx[index], cy[index]
    local wish_x, wish_y = 0, 0

    pace[index] = math.max(math.min(pace[index] + (math.random() * 2 - 1) * 0.03, 1.5), 0.6)

    -- What the other blobs are asking of it. Like charges repel, opposite
    -- attract, and either fades with distance — the falloff is one at contact
    -- and heads for zero across the screen, so a blob is mostly answering to
    -- whoever is nearest without ever being yanked by them.
    local mine = math.sin(phase[index])
    local charge_x, charge_y = 0, 0
    for other = 1, n do
      if other ~= index then
        local dx, dy = px - cx[other], py - cy[other]
        local dist = math.max(math.sqrt(dx * dx + dy * dy), 0.001)
        local reach = touching(index, other)
        local falloff = reach * reach / (dist * dist + reach * reach)
        -- Negative product means opposite charges, which pull; positive means
        -- alike, which push. Multiplying the two is what makes a blob's own
        -- swing change how it feels about every other blob at once.
        local along = -mine * math.sin(phase[other])
        charge_x = charge_x + along * falloff * dx / dist
        charge_y = charge_y + along * falloff * dy / dist
      end
    end
    -- Averaged over the others, not summed over them, so that being pulled at
    -- by five blobs is not five times the argument. Summed, the pull scales
    -- with how many neighbours a blob has and always beats the one force that
    -- has to win in the end, which is the one keeping them from stacking.
    wish_x = wish_x + CHARGE * charge_x / (n - 1)
    wish_y = wish_y + CHARGE * charge_y / (n - 1)

    -- Whatever the charges want, they do not get to put two blobs in the same
    -- place. Attraction that strong is how a lamp becomes a single lump.
    local apart_x, apart_y, crowd = 0, 0, 0
    for other = 1, n do
      if other ~= index then
        local dx, dy = px - cx[other], py - cy[other]
        local dist = math.max(math.sqrt(dx * dx + dy * dy), 0.001)
        local room = touching(index, other) * 1.1
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

    -- Back towards the middle from outside the bowl. A swarm that is all
    -- pushing apart at once has nothing else to turn it round.
    local out_x, out_y = px - CENTER_X, py - CENTER_Y
    local out = math.max(math.sqrt(out_x * out_x + out_y * out_y), 0.001)
    if out > BOWL * 0.85 then
      local excess = math.min((out - BOWL * 0.85) / BOWL, 1.0)
      wish_x = wish_x - HOME * excess * out_x / out
      wish_y = wish_y - HOME * excess * out_y / out
    end

    -- And off the edge of the screen, which nothing above knows about.
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

    wish_x = wish_x + WANDER * (math.random() * 2 - 1)
    wish_y = wish_y + WANDER * (math.random() * 2 - 1)

    -- The cap. This is the whole difference between a force that looks like
    -- gravity and one that behaves like it: the direction is as unbounded as
    -- the charges make it, and the turn it produces never is.
    local wish = math.sqrt(wish_x * wish_x + wish_y * wish_y)
    if wish > 0.001 then
      local want = CRUISE * pace[index]
      local turn_x = wish_x / wish * want - vx[index]
      local turn_y = wish_y / wish * want - vy[index]
      local turn = math.sqrt(turn_x * turn_x + turn_y * turn_y)
      local cap = math.min(AGILITY / math.max(turn, 0.001), 1.0)
      mold.animation.impulse(blobs[index], "x", turn_x * cap * dt)
      mold.animation.impulse(blobs[index], "y", turn_y * cap * dt)
    end
  end
end

local field = { x = 0, y = 0, width = FIELD_W, height = FIELD_H }
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
    fill_color = spec.pulling,
    operation = index == 1 and "union" or "smooth_union",
    blend = s(0.038),
    -- Slow, so the colour reads as a mood the blob is in rather than as a
    -- switch being thrown.
    behavior = {
      fill_color = { duration = 1500, easing = "in_out_cubic" },
    },
  }
  field[#field + 1] = blobs[index]
end

local STEP = 32

ui.Item {
  width = SURFACE_W,
  height = SURFACE_H,
  ui.Sdf(field),

  ui.Timer {
    interval = STEP,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      recharge(STEP / 1000)
      steer(STEP / 1000)
      for index, node in ipairs(blobs) do
        if not mold.animation.active(node, "x") and not mold.animation.active(node, "y") then
          throw(index)
        end
      end
    end,
  },
}

for index = 1, #BLOBS do
  throw(index)
end
