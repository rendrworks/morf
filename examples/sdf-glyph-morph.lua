-- Letters and shapes are the same kind of thing.
--
-- A glyph is an outline, and a distance-field composition takes outlines, so a
-- letter is a shape in it: it unions, subtracts and morphs by the arithmetic a
-- circle does. Nothing here is text drawn over a picture of a shape — the
-- figure in the middle is a *hole*, and both the hole and the thing it is cut
-- from are moving at once.
--
-- Click anywhere. Each click picks a new shape for the body and a new thing to
-- cut out of it, and the two travel together. Sometimes the hole is a letter
-- turning into another letter; sometimes it is a letter turning into a shape,
-- or a shape into a letter, because the composition does not distinguish them.
--
-- Each click also changes the face. Correspondence is geometry — contours
-- paired by position, resampled, rotated onto each other — and knows nothing
-- about which font either outline came out of, so a grotesque `8` walks onto a
-- blackletter `W` exactly as it walks onto its own. Watch a pixel face arrive:
-- its corners are all right angles, and they stay right angles the whole way.
--
-- The two morphs are not the same underneath, and the difference is worth
-- watching for. Two letters correspond: their contours are matched, resampled
-- and rotated onto each other, so every point walks to its opposite number and
-- the shape between them is a real letterform. A letter and a star have no
-- correspondence to find, so those two interpolate as fields — which passes
-- through shapes neither describes and can change topology on the way, a thing
-- outlines cannot do at all.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

-- A window, not a screen. This used to cover the output on the overlay layer
-- with a click target across the whole of it, which left nothing above it to
-- click on and no way to reach anything underneath — including whatever you
-- would have used to close it.
-- Sized around the stage rather than the other way about: the glyph is the
-- thing being looked at, so it gets its measurement first and the window is
-- whatever holds it with room for two lines of label.
local STAGE = 680
local W, H = STAGE + 220, STAGE + 300
morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local function s(n) return n end

local TEXT = "#e9edf5"
local MUTED = "#78849a"

-- Every family the field can draw. A `Pie` and a `Ring` are as much a shape to
-- morph through as a circle is.
local SHAPES = {
  "circle", "rect", "capsule", "triangle", "hexagon",
  "star", "ring", "pie", "cross", "ellipse",
}

-- Letters, digits and symbols, so the hole is not always a numeral.
local GLYPHS = {
  "A", "B", "G", "K", "R", "S", "W", "@", "&", "?",
  "0", "3", "5", "8", "9", "#", "%", "+", "£", "€",
}

-- Faces to cut the letters from. The four generic names always resolve to
-- something; the rest are asked for by name and fall back if the machine has
-- not got them — which is why the caption says which face was *asked* for
-- rather than claiming to know which one arrived.
local FACES = {
  "sans-serif", "serif", "monospace", "cursive",
  "Blacksword", "basis33", "Grape Nuts", "Departure Mono",
  "Bahnschrift", "Iosevka", "Liberation Serif", "Roboto",
}

local TINTS = {
  "#7fc3dd", "#c98fd0", "#8fd0a4", "#e0b56a", "#e0847a",
  "#8f9fe0", "#6fd0c8", "#d0c06f",
}

local MORPH = 900

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--------------------------------------------------------------------------------
-- What is on screen, and what it is turning into.
--------------------------------------------------------------------------------

local travel = morf.signal("morph.travel", 0)
local tint = morf.signal("morph.tint", 1)
local seed = morf.signal("morph.seed", 0)

--- The composition is described twice: what it is now, and what it is becoming.
--- A morph is those two plus a number between them, and landing it is copying
--- the second over the first — at which point the number means nothing and can
--- go back to zero without anything moving.
local now = { body = "circle", hole = "8", hole_is_glyph = true, face = "sans-serif" }
local next_up = now

-- One journey at a time. A plain flag rather than the travel signal: the signal
-- returns to zero the moment the swap lands, while the property is still easing
-- back to it, so a click in that window started a second morph on top of the
-- first and two things moved at once.
local morphing = false

local body_node
local hole_node
local caption
local swap

local function pick(list, avoid)
  local choice = avoid
  for _ = 1, 8 do
    choice = list[math.random(#list)]
    if choice ~= avoid then break end
  end
  return choice
end

local function describe(state)
  if not state.hole_is_glyph then
    return state.body .. "  ·  " .. state.hole
  end
  return state.body .. "  ·  " .. state.hole .. "  ·  " .. state.face
end

local function advance()
  if morphing then return end
  morphing = true
  next_up = {
    body = pick(SHAPES, now.body),
    -- One in five is a shape rather than a letter, so a run of clicks still
    -- shows letter-to-shape and shape-to-shape without being asked — but the
    -- letters are what the faces are for, so they are most of it.
    hole_is_glyph = math.random(5) > 1,
    face = pick(FACES, now.face),
  }
  next_up.hole = next_up.hole_is_glyph and pick(GLYPHS, now.hole)
    or pick(SHAPES, now.hole)

  body_node.morph_to = next_up.body
  if now.hole_is_glyph and next_up.hole_is_glyph then
    -- Both letters: they correspond, so this is an outline morph and the
    -- shape in between is a letterform — including when the two are cut from
    -- different faces, which is a morph and not a swap.
    hole_node.shape = "glyph"
    hole_node.glyph = now.hole
    hole_node.font_family = now.face
    hole_node.glyph_morph_to = next_up.hole
    hole_node.font_family_morph_to = next_up.face
    hole_node.morph_to = "glyph"
  else
    -- One of them is not a letter. There is no correspondence to find between
    -- an `S` and a hexagon, so the two are interpolated as fields instead.
    hole_node.glyph = now.hole_is_glyph and now.hole or ""
    hole_node.font_family = now.hole_is_glyph and now.face or next_up.face
    hole_node.font_family_morph_to = ""
    hole_node.shape = now.hole_is_glyph and "glyph" or now.hole
    if next_up.hole_is_glyph then
      hole_node.glyph = next_up.hole
      hole_node.font_family = next_up.face
      hole_node.morph_to = "glyph"
    else
      hole_node.morph_to = next_up.hole
    end
  end

  write(tint, (tint:get() % #TINTS) + 1)
  write(travel, 1)
  swap.running = true
end

--------------------------------------------------------------------------------
-- Drawing it.
--------------------------------------------------------------------------------

local STAGE_X = math.floor((W - STAGE) / 2)
local STAGE_Y = s(96)

body_node = ui.SdfShape {
  width = STAGE,
  height = STAGE,
  shape = now.body,
  morph_to = now.body,
  radius = math.floor(STAGE * 0.18),
  points = 5,
  inner_radius = 0.45,
  thickness = math.floor(STAGE * 0.16),
  angle = 260,
  morph_progress = function() return travel:get() end,
  behavior = { morph_progress = { duration = MORPH, easing = "in_out_cubic" } },
}

-- Subtracted, so it is a hole in the body rather than a second thing standing
-- in front of it. Its own morph runs on the same number as the body's, which is
-- why the two never fall out of step: there is one animation, not two.
hole_node = ui.SdfShape {
  x = math.floor(STAGE * 0.26),
  y = math.floor(STAGE * 0.24),
  width = math.floor(STAGE * 0.48),
  height = math.floor(STAGE * 0.52),
  shape = "glyph",
  glyph = now.hole,
  glyph_morph_to = now.hole,
  font_family = now.face,
  morph_to = "glyph",
  radius = math.floor(STAGE * 0.06),
  points = 6,
  inner_radius = 0.5,
  thickness = math.floor(STAGE * 0.07),
  angle = 300,
  operation = "subtract",
  morph_progress = function() return travel:get() end,
  behavior = { morph_progress = { duration = MORPH, easing = "in_out_cubic" } },
}

caption = ui.Text {
  y = STAGE_Y + STAGE + s(58),
  width = W,
  text = describe(now),
  font_size = s(20),
  font_weight = 500,
  horizontal_alignment = "center",
  color = TEXT,
}

--- Lands the new composition and drops the progress.
swap = ui.Timer {
  interval = MORPH,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    now = next_up
    body_node.shape = now.body
    body_node.morph_to = now.body
    if now.hole_is_glyph then
      hole_node.shape = "glyph"
      hole_node.glyph = now.hole
      hole_node.glyph_morph_to = now.hole
      -- Both ends the same face now, so the letter stops travelling where it
      -- arrived rather than snapping back to the face it set out from.
      hole_node.font_family = now.face
      hole_node.font_family_morph_to = ""
      hole_node.morph_to = "glyph"
    else
      hole_node.glyph = ""
      hole_node.shape = now.hole
      hole_node.morph_to = now.hole
    end
    write(travel, 0)
    caption.text = describe(now)
    morphing = false
  end,
}

-- No background. The surface paints only the shape and its labels, so
-- everywhere else stays transparent and the desktop shows through — which is
-- also the honest way to look at an edge, with something behind it rather than
-- a flat colour chosen to flatter it.
ui.Item {
  width = W,
  height = H,

  ui.Sdf {
    x = STAGE_X,
    y = STAGE_Y,
    width = STAGE,
    height = STAGE,
    fill_color = function() return TINTS[tint:get()] end,
    -- A seam radius, so where the hole meets the body's edge the two draw a
    -- neck between them instead of a corner.
    blend = s(10),
    behavior = { fill_color = { duration = MORPH, easing = "in_out_cubic" } },
    body_node,
    hole_node,
  },

  caption,

  ui.Text {
    y = STAGE_Y + STAGE + s(92),
    width = W,
    text = "click anywhere",
    font_size = s(14),
    horizontal_alignment = "center",
    color = MUTED,
  },

  ui.Text {
    y = s(40),
    width = W,
    text = "a letter is a shape: it unions, subtracts and morphs like one",
    font_size = s(15),
    horizontal_alignment = "center",
    color = MUTED,
  },

  -- The surface's own area, which is all it has any business claiming.
  ui.MouseArea {
    width = W,
    height = H,
    on_clicked = advance,
  },

  -- In the tree, because a timer that is not in it never runs.
  swap,
}

-- Different every run, so a demonstration is not the same demonstration twice.
math.randomseed(core.launch_time_ms + seed:get())
