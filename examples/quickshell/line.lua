-- Workspace ribbon, ported from `~/.config/quickshell/line/modules/line/`.
--
-- Three pieces share one edge of the output. `Ribbon.qml` is the strip itself:
-- ten pills down the middle half of the screen, coloured by whether a
-- workspace is active, has windows, or is empty, and a wheel over the strip
-- steps through workspaces. `Numbers.qml` watches the active workspace and
-- morphs a badge out of the corresponding pill: it grows from the pill's width
-- to a square, slides out from under the bar, shows the workspace number, and
-- fades back after 800ms. `RibbonPopup.qml` is the window the badge lives in,
-- which sits tucked under the bar until something expands it.
--
-- The original gives each piece its own layer surface, one per monitor, and
-- mirrors the whole thing to the right edge on monitors left of the main one.
-- morf hosts one layer surface per process, so all three live on a single
-- strip wide enough to hold the badge at full extension, positioned against
-- whichever edge `Workspace.qml` would have chosen.
--
-- The morph is driven a frame at a time rather than declared as a behavior.
-- `Numbers.qml` grows the badge with `startWidth + (endSize - startWidth) *
-- (morphProgress * morphProgress)` — a quadratic that keeps the badge
-- pill-thin through the first half of the transition and then opens out. A
-- property bound to that expression would only ever be evaluated at the two
-- ends of the morph, because the signal it reads jumps straight from 0 to 1,
-- and the engine would tween linearly between those two values: the shape of
-- the animation, which is the whole point of it, would be lost. So an
-- invisible carrier node is animated instead, and its geometry change wakes a
-- transform watcher once per painted frame; the badge's own geometry is
-- written from there, off a clock, with the curve evaluated at the value of
-- `morphProgress` that frame actually has.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local theme = require("theme")
local hypr = require("hypr")

local line = {}

local WIDTH, HEIGHT = theme.reference()
local SHORT_SIDE = math.min(WIDTH, HEIGHT)

-- Geometry, straight from Workspace.qml, Line.qml and Numbers.qml.
local bar_width = math.floor(WIDTH * 0.005 + 0.5)
local track_height = HEIGHT * 0.5
local pill_spacing = SHORT_SIDE * (10 / 2160)
local pill_width_factor = 0.55
local pill_width = bar_width * pill_width_factor
local item_height = (track_height - pill_spacing * 9) / 10
local pill_radius = bar_width * (4 / 8)
local track_top = (HEIGHT - track_height) / 2

local popup_gap = math.floor(SHORT_SIDE * (6 / 2160) + 0.5)
local popup_slide = bar_width * (1 + ((1 + pill_width_factor) / 2)) + popup_gap
local popup_border_width = math.max(1, SHORT_SIDE * (2 / 2160))
local popup_border_growth = SHORT_SIDE * (4 / 2160)

-- The badge is square at full morph and starts at the pill's width; its corner
-- starts at the shared pill radius rather than the bar's, as Numbers.qml does.
local badge_size = item_height
local badge_radius = theme.pill_radius()

-- RibbonPopup's own geometry. The compact inset parks the popup a bar's width
-- off the edge, so the badge is completely hidden at rest and emerges as it
-- grows; the expanded inset clears the bar and its gap.
local reserved_thickness = math.floor(bar_width + 6 + 0.5)
local compact_inset = -bar_width
local expanded_inset = reserved_thickness + popup_gap

-- Wide enough for the badge at full extension out of an expanded popup, so
-- every piece fits the one surface.
local strip_width = math.ceil(expanded_inset + popup_slide + badge_size + popup_gap)

-- Numbers.qml: morph in over 400ms out-cubic, hold 800ms, morph out over 300ms
-- in-cubic, with the vertical slide a separate 250ms in-out-quad.
local MORPH_IN_MS = 400
local MORPH_OUT_MS = 300
local HOLD_MS = 800
local SLIDE_MS = 250

-- The carrier that keeps frames coming while the morph runs. `translate_x` is
-- a pure transform, so animating it moves nothing and lays out nothing, but it
-- is part of the transform signature the watcher hashes, so every frame of it
-- is a callback. The span is arbitrary; only the fact that it changes matters.
-- The duration outlasts the longest morph so the final frame always lands.
local PUMP_MS = 700
local PUMP_SPAN = 1000

--- The y offset of a pill within the track, by its index in the block of ten.
local function row_offset(index)
  return (item_height + pill_spacing) * (index - 1)
end

-- Badge state. `Numbers.qml` keeps this in `shouldShowOSD`, `morphProgress`
-- and a hide timer; the same four values, held in Lua because the curve is
-- evaluated here rather than by the engine.
local morph_mode = "idle" -- idle | in | hold | out
local morph_value = 0
local morph_clock = core.elapsed_timer()
local hold_clock = core.elapsed_timer()
local last_shown = -1
local last_shown_id = -1
local bar_on_right = false
local expand_progress = 0

local badge_workspace = morf.signal("quickshell.line.badge_workspace", 1)

-- Nodes, filled in by `build`.
local strip, column, frame, badge, anchor, field, label, pump

-- ------------------------------------------------------------------ layout --

--- RibbonPopup's `horizontalInset`, between the compact and expanded states.
local function frame_inset()
  return compact_inset + (expanded_inset - compact_inset) * expand_progress
end

--- Places the popup frame. Mirrored, its x is the popup's right edge, so the
--- badge inside it grows leftwards from the same inset.
local function apply_frame(animated)
  if not frame then return end
  local inset = frame_inset()
  local x = bar_on_right and (strip_width - inset) or inset
  if not animated then morf.animation.set_enabled(frame, "x", false) end
  frame.x = x
  if not animated then morf.animation.set_enabled(frame, "x", true) end
end

--- Writes every dimension the morph owns for one value of `morphProgress`.
local function apply_morph(value)
  morph_value = value
  if not badge then return end
  -- The quadratic from Numbers.qml, evaluated at this frame's progress.
  local width = pill_width + (badge_size - pill_width) * (value * value)
  local slide = popup_slide * value
  badge.width = width
  badge.radius = badge_radius + (badge_size * 0.5 - badge_radius) * value
  badge.x = bar_on_right and (-slide - width) or slide
  -- The seam radius closes as the badge pulls away. The two overlap for most
  -- of the slide, so the neck holds nearly to the end and thins out rather
  -- than snapping: that easing of the join is the whole effect.
  badge.blend = pill_width * 1.9 * (1.0 - value)
  field.stroke_width = popup_border_width + popup_border_growth * value
  field.visible = value > 0
  label.width = width
  label.x = badge.x
  label.y = badge.y
  label.font_size = math.max(1, math.floor(math.min(width, badge_size) * 0.495 + 0.5))
  label.opacity = value
end

--- Moves everything to the edge `Workspace.qml` would have chosen.
local function apply_side()
  if not strip then return end
  strip.x = bar_on_right and (WIDTH - strip_width) or 0
  column.x = bar_on_right and (strip_width - bar_width) or 0
  apply_frame(false)
  apply_morph(morph_value)
end

-- ------------------------------------------------------------------- morph --

--- Restarts the carrier, which is what keeps painted frames coming. The target
--- alternates so a restart mid-flight is always a real change of target.
local pump_high = false
local function kick()
  if not pump then return end
  pump_high = not pump_high
  pump.translate_x = pump_high and PUMP_SPAN or 0
end

--- Sets the badge's row without sliding to it, as `displayY = ...` does.
local function place_row(index)
  local row = track_top + row_offset(index)
  morf.animation.set_enabled(badge, "y", false)
  badge.y = row
  morf.animation.set_enabled(badge, "y", true)
  anchor.y = row + (item_height - badge_size) / 2
end

--- Slides the badge to a row, taking the pill it grows from with it.
local function slide_to(row)
  badge.y = row
  anchor.y = row + (item_height - badge_size) / 2
end

--- `showWorkspaceId` from Numbers.qml.
local function show_badge(index, id, force)
  if not badge then return end
  -- Numbers.qml keys this on the workspace, not the row: a block of ten that
  -- has scrolled under the badge is a different workspace at the same offset.
  if not force and morph_mode == "idle" and last_shown_id == id then return end
  badge_workspace:set(id)
  local target = track_top + row_offset(index)
  if morph_mode == "idle" then
    -- Fresh show: start where the previous badge was, so the slide reads as
    -- movement between workspaces rather than an appearance out of nowhere.
    if last_shown >= 1 and last_shown ~= index then
      place_row(last_shown)
      slide_to(target)
    else
      place_row(index)
    end
    morph_mode = "in"
    morph_clock:restart()
    apply_morph(0)
  elseif morph_mode == "out" then
    -- Was fading out: cancel and stay.
    morph_mode = "hold"
    apply_morph(1)
    if last_shown ~= index then slide_to(target) end
  else
    -- Already up: slide if the workspace changed.
    if last_shown ~= index then slide_to(target) end
  end
  hold_clock:restart()
  last_shown = index
  last_shown_id = id
  kick()
end

--- Advances the morph to the value this frame should show. Called once per
--- painted frame from the transform watcher, and again on the tick so a
--- stalled frame clock cannot leave the badge stranded mid-morph.
local function advance()
  if morph_mode == "in" then
    local progress = math.min(1, morph_clock:elapsed_ms() / MORPH_IN_MS)
    apply_morph(morf.easing.value("out_cubic", progress))
    if progress >= 1 then morph_mode = "hold" end
  elseif morph_mode == "out" then
    local progress = math.min(1, morph_clock:elapsed_ms() / MORPH_OUT_MS)
    apply_morph(1 - morf.easing.value("in_cubic", progress))
    if progress >= 1 then morph_mode = "idle" end
  end
end

-- ------------------------------------------------------------------- pills --

--- One workspace pill.
local function pill(index)
  local hovered = morf.signal("quickshell.line.hover." .. index, false)
  return ui.Rect {
    x = (bar_width - pill_width) / 2,
    y = track_top + row_offset(index),
    width = pill_width,
    height = item_height,
    radius = pill_radius,
    color = function()
      local row = hypr.row(index)
      if row.active then return theme.palette.color1 end
      return row.windows > 0 and theme.palette.color244 or theme.palette.color240
    end,
    opacity = function()
      local row = hypr.row(index)
      if row.active then return 1.0 end
      return hovered:get() and 0.9 or 0.6
    end,
    behavior = {
      color = { duration = 200 },
      opacity = { duration = 200 },
    },
    ui.MouseArea {
      cursor = "pointer",
      anchors = { fill = true },
      on_entered = function() hovered:set(true) end,
      on_exited = function() hovered:set(false) end,
      on_clicked = function() hypr.dispatch(hypr.row(index).id) end,
      -- The wheel reaches the topmost area under the pointer and no further,
      -- so a pill has to answer it too.
      on_wheel = function(_, _, _, vertical, _, steps)
        line.wheel(steps ~= 0 and steps or vertical)
      end,
    },
  }
end

--- `onWheel` from Ribbon.qml. Wayland's vertical axis points down, where Qt's
--- `angleDelta.y` points up, so the comparison is the other way round.
function line.wheel(delta)
  if delta == 0 then return end
  hypr.dispatch(delta < 0 and "r-1" or "r+1")
end

-- ------------------------------------------------------------------- ticks --

--- Everything that is not per-frame: sockets, the hide timer, the edge.
local function tick()
  hypr.poll()
  local request = hypr.take_badge()
  if request then
    show_badge(hypr.index_of(request.id) or hypr.active_index(), request.id, request.force)
  end
  if morph_mode ~= "idle" and morph_mode ~= "out" and hold_clock:elapsed_ms() >= HOLD_MS then
    morph_mode = "out"
    morph_clock:restart()
    kick()
  end
  advance()
  local side = hypr.bar_on_right()
  if side ~= bar_on_right then
    bar_on_right = side
    apply_side()
  end
end

-- ------------------------------------------------------------------- build --

--- The ribbon, its badge and the popup that carries it, as one subtree.
function line.build()
  local pills = {}
  for index = 1, hypr.ROW_COUNT do
    pills[index] = pill(index)
  end

  column = ui.Item(
    (function(values)
      -- Under the pills, so a click still reaches them, and only as wide as
      -- the bar, matching the Ribbon window the wheel handler lives on.
      values[#values + 1] = ui.MouseArea {
        width = bar_width,
        height = HEIGHT,
        on_wheel = function(_, _, _, vertical, _, steps)
          line.wheel(steps ~= 0 and steps or vertical)
        end,
      }
      for _, child in ipairs(pills) do values[#values + 1] = child end
      return values
    end) { x = 0, width = bar_width, height = HEIGHT }
  )

  label = ui.Text {
    x = 0,
    y = 0,
    width = pill_width,
    height = badge_size,
    text = function() return tostring(badge_workspace:get()) end,
    color = function() return theme.palette.color1:text_color() end,
    font_family = theme.font,
    font_source = theme.font_source,
    font_size = math.max(1, math.floor(math.min(pill_width, badge_size) * 0.495 + 0.5)),
    font_weight = 900,
    horizontal_alignment = "center",
    vertical_alignment = "center",
    opacity = 0,
  }

  -- Width, x, radius, border width, the label's size and its opacity are all
  -- written per frame by `apply_morph`, so none of them carries a behavior:
  -- a behavior would tween towards each frame's value instead of showing it.
  -- The row slide is the one thing the engine still owns, because it really
  -- is a plain interpolation between two positions.
  -- `y` is left at zero here on purpose: a behavior is installed before the
  -- constructor's own values are assigned, so setting the row here would
  -- animate the badge in from the top of the screen. `place_row` sets it.
  -- The badge and the pill it comes from are one composition, not two
  -- rectangles that happen to overlap. `Numbers.qml` grows the badge *out of*
  -- its pill; with a smooth union that is literally what happens — the two
  -- surfaces bulge into each other while they are close and the neck thins and
  -- parts as the badge slides clear, with nothing tracking the transition.
  --
  -- The anchor is the pill, redrawn inside the field in the same colour, so
  -- the fused shape sits exactly over the real one.
  anchor = ui.SdfShape {
    x = (bar_width - pill_width) / 2,
    y = 0,
    width = pill_width,
    height = item_height,
    shape = "box",
    radius = pill_radius,
  }

  badge = ui.SdfShape {
    x = 0,
    y = 0,
    width = pill_width,
    height = badge_size,
    shape = "box",
    radius = badge_radius,
    operation = "smooth_union",
    blend = 0,
    behavior = { y = { duration = SLIDE_MS, easing = "in_out_quad" } },
  }

  field = ui.Sdf {
    x = 0,
    y = 0,
    width = strip_width,
    height = HEIGHT,
    fill_color = function() return theme.palette.color1 end,
    stroke_color = function() return theme.palette.color0 end,
    stroke_width = popup_border_width,
    visible = false,
    anchor,
    badge,
    label,
  }

  -- Same as the badge's row: the compact inset is applied by `apply_side`,
  -- with the behavior off, rather than animated in from zero at startup.
  frame = ui.Item {
    y = 0,
    width = 0,
    height = HEIGHT,
    behavior = { x = { duration = theme.short_duration, easing = "in_out_quad" } },
    field,
  }

  pump = ui.Item {
    x = 0,
    y = 0,
    width = 1,
    height = 1,
    visible = false,
    translate_x = 0,
    behavior = { translate_x = { duration = PUMP_MS, easing = "linear" } },
  }

  strip = ui.Item {
    x = 0,
    y = 0,
    width = strip_width,
    height = HEIGHT,

    column,
    frame,
    pump,

    -- Sockets, the hide timer and the edge check. Two frames' worth: fast
    -- enough that a workspace change is on screen within a frame or two, slow
    -- enough that the blocking socket reads cost nothing measurable.
    ui.Timer {
      interval = 32,
      ["repeat"] = true,
      running = true,
      on_triggered = tick,
    },

    -- A reconciliation in case an event was ever missed.
    ui.Timer {
      interval = 1000,
      ["repeat"] = true,
      running = true,
      on_triggered = function() hypr.refresh() end,
    },
  }

  -- One callback per painted frame for as long as the carrier is moving. The
  -- watcher only reports a change once both nodes have been laid out, so the
  -- first morph starts from the first frame after this.
  core.transform_watcher {
    a = pump,
    b = badge,
    common_parent = strip,
    on_changed = advance,
  }

  bar_on_right = hypr.bar_on_right()
  place_row(1)
  apply_side()
  apply_morph(0)
  return strip
end

--- The width of the strip the ribbon and badge occupy.
function line.width() return strip_width end

--- The bar itself, which is the only part that should take a click.
function line.bar_width() return bar_width end

--- RibbonPopup's expanded state: the popup slides clear of the bar and stays
--- there. Nothing in the ported shell expands it yet; it is the hook a panel
--- alongside the ribbon would use.
function line.set_expanded(expanded)
  local target = expanded and 1 or 0
  if target == expand_progress then return end
  expand_progress = target
  apply_frame(true)
end

function line.expanded()
  return expand_progress > 0
end

return line
