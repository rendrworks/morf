-- Pseudo-physics: motion decided by forces rather than by a curve.
--
-- A behavior answers "this property was assigned a value — how does it travel
-- there". Every lane but the last is one of those, and every one of them knows
-- its destination before it starts moving.
--
-- A fling does not. It is thrown at a speed and stops where friction leaves it,
-- or where a bound catches it. Nothing in the configuration chooses the landing
-- point; it is a consequence of how hard the throw was. That is why it is a
-- verb — `morf.animation.fling` — rather than another `kind` in a behavior
-- table: there is no target for an assignment to set.

local morf = require("morf")
local ui = require("morf.ui")

local W, H = 900, 420
morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local INK = "#0e1213"
local PANEL = "#141a1c"
local ACCENT = "#b4e1ea"
local WARM = "#f0b47a"
local MUTED = "#6a8389"

local LANE_X, LANE_W = 250, 560
local ROW_H = 62

local target = morf.signal("physics.target", 0)

--- The puck for a lane driven by a behavior, which knows where it is going.
local function driven(y, behavior, tint)
  return ui.Sdf {
    x = LANE_X,
    y = y,
    width = 46,
    height = 46,
    fill_color = tint,
    x = function() return LANE_X + target:get() * LANE_W end,
    behavior = { x = behavior },
    ui.SdfShape { width = 46, height = 46, shape = "circle" },
  }
end

-- The fling's puck is not bound to anything: its position is written by the
-- physics, so the configuration never says where it should be.
local thrown = ui.Sdf {
  x = LANE_X,
  y = 0,
  width = 46,
  height = 46,
  fill_color = WARM,
  ui.SdfShape { width = 46, height = 46, shape = "circle" },
}

local lanes = {
  {
    "Tween",
    "a curve over a fixed time",
    driven(0, { duration = 900, easing = "in_out_cubic" }, ACCENT),
  },
  {
    "Spring",
    "a force; overshoots, then settles",
    driven(0, { kind = "spring", mass = 1, damping = 12, stiffness = 140, epsilon = 0.05 }, ACCENT),
  },
  {
    "Smoothed",
    "constant speed, however far it is",
    driven(0, { kind = "smoothed", velocity = 620 }, ACCENT),
  },
  { "Fling", "thrown; friction decides where it lands", thrown },
}

local rows = { width = W, height = H }
rows[#rows + 1] = ui.Rect { width = W, height = H, color = INK }
for index, lane in ipairs(lanes) do
  local top = 30 + (index - 1) * (ROW_H + 30)
  rows[#rows + 1] = ui.Rect {
    x = 20, y = top - 12, width = W - 40, height = ROW_H + 14, radius = 14, color = PANEL,
  }
  rows[#rows + 1] = ui.Text {
    x = 44, y = top + 6, width = 190, text = lane[1], font_size = 16, color = ACCENT,
  }
  rows[#rows + 1] = ui.Text {
    x = 44, y = top + 30, width = 200, text = lane[2], font_size = 11, wrap = true, color = MUTED,
  }
  rows[#rows + 1] = ui.Rect {
    x = LANE_X, y = top + 30, width = LANE_W + 46, height = 2, color = "#25353a",
  }
  -- The lane's own puck, moved to this row.
  lane[3].y = top + 8
  rows[#rows + 1] = lane[3]
end

rows[#rows + 1] = ui.Text {
  x = 44, y = H - 34, width = W - 88,
  text = "the first three are handed a destination; the fling is handed a speed",
  font_size = 12,
  color = MUTED,
}

local throws = { 900, 1500, 2400, 1200 }
local next_throw = 1

rows[#rows + 1] = ui.Timer {
  interval = 2000,
  ["repeat"] = true,
  running = true,
  on_triggered = function()
    local ok, error = target:set(target:get() > 0.5 and 0 or 1)
    assert(ok, error)
    -- Thrown from wherever it happens to be, at a speed that varies, and
    -- caught by the ends of its own lane.
    morf.animation.fling {
      node = thrown,
      property = "x",
      velocity = (target:get() > 0.5 and 1 or -1) * throws[next_throw],
      preset = "smooth",
      min = LANE_X,
      max = LANE_X + LANE_W,
    }
    next_throw = next_throw % #throws + 1
  end,
}

ui.Item(rows)
