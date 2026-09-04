-- Keyframe tracks: one property through several stops, each with its own curve.
--
-- A behavior takes a property from where it is to where it is going, with one
-- curve. That is the right primitive for a reaction, and the wrong one for a
-- path: an arc, an overshoot-and-settle, a bounce with a pause in it are all
-- several segments, and writing them as nested timers is how configurations
-- turn into frame runtimes.
--
-- A track names the property, one duration, and stops at normalized offsets. It
-- is not a second animation runtime — each pair of neighbouring stops expands
-- into an ordinary property animation with an explicit `from` and `to`, so
-- retargeting, damage and completion events apply exactly as they do anywhere
-- else. Offsets are fractions of the whole, which is what makes a track
-- editable: move a stop and the segments after it follow.

local morf = require("morf")
local ui = require("morf.ui")

local W, H = 820, 420
morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local theme = morf.theme {
  ink = "#0e1213",
  panel = "#141a1c",
  accent = "#b4e1ea",
  warm = "#f0b47a",
  muted = "#6a8389",
}

local LANE_X, LANE_W = 200, 560
local ROW_Y, ROW_H = 40, 84

-- Each lane is one track. The stops are written as fractions, so the shape of
-- the motion is legible without doing arithmetic in your head.
local lanes = {
  {
    title = "Arc",
    caption = "up, across, down — three segments",
    property = "y",
    frames = {
      { at = 0.0, value = 0 },
      { at = 0.35, value = -42, easing = "out_cubic" },
      { at = 0.65, value = -42, easing = "linear" },
      { at = 1.0, value = 0, easing = "in_cubic" },
    },
  },
  {
    title = "Overshoot",
    caption = "past the mark, then settle back",
    property = "x",
    frames = {
      { at = 0.0, value = 0 },
      { at = 0.55, value = LANE_W + 40, easing = "out_quint" },
      { at = 0.75, value = LANE_W - 26, easing = "in_out_quad" },
      { at = 1.0, value = LANE_W, easing = "out_back" },
    },
  },
  {
    title = "Hold",
    caption = "a stop that waits, then a jump",
    property = "x",
    frames = {
      { at = 0.0, value = 0 },
      { at = 0.3, value = LANE_W * 0.42, easing = "in_out_cubic" },
      -- Two stops at the same offset are a deliberate jump: the value changes
      -- with no time to interpolate over.
      { at = 0.62, value = LANE_W * 0.42, easing = "linear" },
      { at = 0.62, value = LANE_W * 0.66, easing = "linear" },
      { at = 1.0, value = LANE_W, easing = "out_cubic" },
    },
  },
}

local pucks = {}
local rows = { width = W, height = H }
rows[#rows + 1] = ui.Rect { width = W, height = H, color = theme.ink }

for index, lane in ipairs(lanes) do
  local top = ROW_Y + (index - 1) * (ROW_H + 34)
  rows[#rows + 1] = ui.Rect {
    x = 20, y = top - 14, width = W - 40, height = ROW_H + 16, radius = 14, color = theme.panel,
  }
  rows[#rows + 1] = ui.Text {
    x = 40, y = top + 4, width = 150, text = lane.title, font_size = 16, color = theme.accent,
  }
  rows[#rows + 1] = ui.Text {
    x = 40, y = top + 28, width = 150, text = lane.caption, font_size = 11, wrap = true, line_height = 1.4, color = theme.muted,
  }
  -- The rail the puck travels along, so the path is readable at rest.
  rows[#rows + 1] = ui.Rect {
    x = LANE_X, y = top + 40, width = LANE_W + 30, height = 2, color = "#25353a",
  }

  -- The puck is a field, so the two new primitives are visible together: a
  -- keyframe track driving a shape that is itself a composition.
  local puck = ui.Sdf {
    x = LANE_X, y = top + 8, width = 44, height = 66,
    fill_color = index == 2 and theme.warm or theme.accent,
    stroke_color = theme.ink,
    stroke_width = 2.5,
    ui.SdfShape { x = 4, y = 12, width = 36, height = 36, shape = "circle" },
    ui.SdfShape {
      x = 12, y = 30, width = 20, height = 26,
      shape = "circle",
      operation = "smooth_union",
      blend = 14,
    },
  }
  pucks[index] = { node = puck, lane = lane, home_x = LANE_X, home_y = top + 8 }
  rows[#rows + 1] = puck
end

--- Plays every track from the top.
local function play()
  for _, puck in ipairs(pucks) do
    local lane = puck.lane
    -- The stops are relative to where the puck lives, so a track can be written
    -- once and reused at any position on screen.
    local origin = lane.property == "x" and puck.home_x or puck.home_y
    local frames = {}
    for stop, frame in ipairs(lane.frames) do
      frames[stop] = {
        at = frame.at,
        value = origin + frame.value,
        easing = frame.easing,
      }
    end
    morf.animation.play {
      {
        node = puck.node,
        property = lane.property,
        duration = 2200,
        keyframes = frames,
      },
    }
  end
end

rows[#rows + 1] = ui.Text {
  x = 40, y = H - 44, width = W - 80,
  text = "each lane is one morf.animation.play track, replayed every 2.6s",
  font_size = 12,
  color = theme.muted,
}
rows[#rows + 1] = ui.Timer {
  interval = 2600,
  ["repeat"] = true,
  running = true,
  on_triggered = play,
}

ui.Item(rows)

-- The Timer only reports after its first interval, so the opening pass is
-- started here; every later one comes from the timer.
play()
