-- Compound morphing: several shapes and a label moving as one thing.
--
-- A compound is not a new mechanism. It is a field whose `morph_progress`
-- drives every layer that does not name its own, so the whole composition is
-- one animatable number — and a `Text` that is not part of the field at all,
-- following the same number with its own opacity and size.
--
-- The last panel is the case a workspace badge needs: a disc with a numeral in
-- it collapsing into a small pill with the numeral gone. Nothing there is
-- special-cased; it is the shape morph, the layer rectangle and the label's
-- opacity all reading one signal.

local mold = require("mold")
local ui = require("mold.ui")

local W, H = 940, 320
mold.surface.width = W
mold.surface.height = H
mold.surface.anchors = { top = true, left = true }
mold.surface.keyboard_focus = "none"

local INK = "#0e1213"
local PANEL = "#141a1c"
local ACCENT = "#b4e1ea"
local WARM = "#f0b47a"
local MUTED = "#6a8389"

-- One number for every panel, so they move in step and the comparison is fair.
local open = mold.signal("compound.open", 0)
local SWING = { duration = 900, easing = "in_out_cubic" }

local function t() return open:get() end
local function lerp(a, b) return function() return a + (b - a) * t() end end

local function panel(x, title, caption, body)
  return ui.Item {
    x = x,
    y = 0,
    width = 220,
    height = H,
    ui.Rect { x = 6, y = 6, width = 208, height = 244, radius = 16, color = PANEL },
    body,
    ui.Text { x = 24, y = 214, width = 180, text = title, font_size = 16, color = ACCENT },
    ui.Text {
      x = 24,
      y = 240,
      width = 180,
      text = caption,
      font_size = 11,
      wrap = true,
      color = MUTED,
    },
  }
end

-- ── One number, three layers ────────────────────────────────────────────────
--
-- The field's `morph_progress` reaches every layer, so a disc, a ring and a bar
-- all change family together without any of them naming a position.
local together = ui.Sdf {
  x = 6, y = 20, width = 208, height = 180,
  fill_color = ACCENT,
  blend = 10,
  morph_progress = function() return t() end,
  behavior = { morph_progress = SWING },
  ui.SdfShape { x = 54, y = 20, width = 100, height = 100, shape = "circle", morph_to = "hexagon" },
  ui.SdfShape {
    x = 74, y = 40, width = 60, height = 60,
    shape = "ring",
    morph_to = "cross",
    thickness = 16,
    operation = "subtract",
  },
  ui.SdfShape {
    x = 44, y = 118, width = 120, height = 26,
    shape = "capsule",
    morph_to = "box",
    operation = "smooth_union",
  },
}

-- ── A layer that opts out ───────────────────────────────────────────────────
--
-- Saying nothing joins the compound; naming a position leaves it. The small
-- star holds still while everything around it moves.
local partial = ui.Sdf {
  x = 6, y = 20, width = 208, height = 180,
  fill_color = WARM,
  blend = 14,
  morph_progress = function() return t() end,
  behavior = { morph_progress = SWING },
  ui.SdfShape { x = 34, y = 30, width = 140, height = 90, shape = "capsule", morph_to = "triangle" },
  ui.SdfShape {
    x = 84, y = 46, width = 40, height = 40,
    shape = "star",
    points = 6,
    morph_progress = 0,
    operation = "subtract",
  },
}

-- ── The badge ───────────────────────────────────────────────────────────────
--
-- A disc with a numeral collapsing to a pill. Three things read the same
-- number: the shape family, the layer's own rectangle, and the label — which is
-- not part of the field, because text has no distance field and paints over the
-- composition like anything else.
local badge_shape = ui.SdfShape {
  x = lerp(50, 84),
  y = lerp(20, 62),
  width = lerp(108, 40),
  height = lerp(108, 18),
  shape = "circle",
  morph_to = "capsule",
  behavior = { x = SWING, y = SWING, width = SWING, height = SWING },
}

local badge_label = ui.Text {
  x = lerp(50, 84),
  y = lerp(50, 66),
  width = lerp(108, 40),
  text = "7",
  font_size = lerp(52, 8),
  font_weight = 900,
  horizontal_alignment = "center",
  color = INK,
  -- Gone well before the pill has finished closing, so the numeral never looks
  -- squeezed by the shape shrinking around it.
  opacity = function() return math.max(0, 1 - t() * 2.2) end,
  behavior = { x = SWING, y = SWING, width = SWING, font_size = SWING, opacity = SWING },
}

local badge = ui.Item {
  x = 6, y = 20, width = 208, height = 180,
  ui.Sdf {
    x = 0, y = 0, width = 208, height = 180,
    fill_color = ACCENT,
    morph_progress = function() return t() end,
    behavior = { morph_progress = SWING },
    badge_shape,
  },
  badge_label,
}

ui.Item {
  width = W, height = H,
  ui.Rect { width = W, height = H, color = INK },

  panel(0, "Together", "one number drives every layer", together),
  panel(235, "Opt out", "the star names its own position", partial),
  panel(470, "Badge", "disc with a numeral to a bare pill", badge),

  ui.Text {
    x = 715, y = 40, width = 210,
    text = "A compound is one animatable number. The label is not part of the field — text has no distance field, so it paints over the composition and follows the same signal with its own opacity.",
    font_size = 12,
    wrap = true,
    color = MUTED,
  },

  ui.Timer {
    interval = 1600,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      local ok, error = open:set(t() > 0.5 and 0 or 1)
      assert(ok, error)
    end,
  },
}
