-- Volume and brightness panels, ported from `~/.config/quickshell/settings/`.
--
-- `Settings.qml` hangs two pills off the shared ribbon and morphs a panel out
-- from under the bar when either is hovered. Hovering grows a round icon bubble
-- out of the pill and slides it clear of the bar; clicking the bubble expands it
-- into a card stack with a volume slider, a mute button, and one row per
-- `pactl` sink input. Losing the pointer retracts it after 300ms, and half a
-- minute without an interaction collapses the wide panel back to the bubble.
--
-- Three things about the original do not survive the move, and each is
-- re-expressed rather than dropped:
--
-- * Each panel is its own `wlr-layer-shell` surface whose horizontal MARGIN is
--   animated to slide it out from under the bar. mold hosts one layer surface
--   per process and fixes its geometry at startup, so the slide is node
--   geometry inside the shared overlay instead. The surfaces were transparent,
--   claimed no exclusive zone and took no keyboard focus, so nothing about them
--   was visible except the pixels they drew: the substitution is exact.
--
-- * Ten QML `Timer`s become one repeating `ui.Timer` with Lua deadlines. Each
--   `ui.Timer` costs an OS thread (`crates/mold-io/src/timer.rs:17-23`), and
--   nine of the ten were only ever counting down to a single assignment.
--
-- * The two morphs are driven per frame from that tick rather than declared as
--   property behaviors. A behavior interpolates linearly between the endpoints
--   of whatever the binding returned, so `width = pill + (button - pill) * t*t`
--   would render as a straight line and the squared ramp would be lost. Driving
--   the progress signal per frame renders the curve the original draws.
--
-- The pills sit below the workspace track on the same bar, because in the
-- original they live on the opposite edge of a second monitor and there is only
-- one bar here.

local mold = require("mold")
local ui = require("mold.ui")
local io = require("mold.io")
local core = require("mold.core")
local theme = require("theme")

local settings = {}

local WIDTH, HEIGHT = theme.reference()
local SHORT = math.min(WIDTH, HEIGHT)

-- The ten workspace rows `line.lua` draws, so the settings pills can be placed
-- under them rather than through them.
local ROW_COUNT = 10
-- `Ribbon.qml`'s `pillWidthFactor`.
local PILL_WIDTH_FACTOR = 0.55
-- `Settings.qml` slices `sinkInputs` to five rows.
local MAX_APPS = 5
-- One tick drives every animation, deadline, and poll in the module.
local TICK_MS = 16

local BRIGHT_COMMAND = (core.env("HOME") or "") .. "/.local/sbin/bright"

-- `pactl`, `pamixer` and `bright` are system binaries. mold is launched through
-- a nixGL-style wrapper that rewrites LD_LIBRARY_PATH to nix store paths, and a
-- child that inherits those fails to find its own libc and exits before writing
-- a line, so every child clears it.
local CHILD_ENVIRONMENT = { LD_LIBRARY_PATH = "" }

-- ─── Colour ───

--- Blends `fg` over `bg` at `amount`, in sRGB.
---
--- mold composites alpha in linear light and Qt composites it in sRGB, so a
--- translucent fill that must match the original is blended here and handed
--- over opaque. Only the panel's own outline keeps its alpha, because what sits
--- behind it is whatever window the panel is over.
local function mix(fg, bg, amount)
  local function channel(hex, at)
    return tonumber(hex:sub(at, at + 1), 16) or 0
  end
  local front = tostring(fg):gsub("#", "")
  local back = tostring(bg):gsub("#", "")
  if #front < 6 or #back < 6 then return "#ff00ff" end
  local a = math.max(0, math.min(1, amount))
  local out = "#"
  for _, at in ipairs { 1, 3, 5 } do
    local value = channel(front, at) * a + channel(back, at) * (1 - a)
    out = out .. string.format("%02x", math.floor(value + 0.5))
  end
  return out
end

-- ─── Signals ───

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

local clock = core.elapsed_timer()

-- Every animated progress in the module. `advance` walks this list once a tick.
local morphs = {}

--- A 0..1 progress driven by hand, so its easing curve is actually rendered.
local function morph(name)
  local entry = {
    signal = mold.signal(name, 0),
    from = 0,
    to = 0,
    start = 0,
    duration = 0,
    curve = "linear",
  }
  -- Reads like the signal it wraps, so a binding on a morph looks the same as
  -- a binding on any other piece of state.
  function entry:get() return self.signal:get() end
  morphs[#morphs + 1] = entry
  return entry
end

--- Retargets a morph from wherever it currently is.
local function drive(entry, to, duration, curve, now)
  entry.from = entry.signal:get()
  entry.to = to
  entry.start = now
  entry.duration = duration
  entry.curve = curve
end

--- Steps one morph, writing only when the value actually moves.
local function advance(entry, now)
  if entry.duration <= 0 then return end
  local progress = (now - entry.start) / entry.duration
  if progress >= 1 then
    progress = 1
    entry.duration = 0
  end
  local value = entry.from + (entry.to - entry.from) * mold.easing.value(entry.curve, progress)
  if value ~= entry.signal:get() then write(entry.signal, value) end
end

--- Smoothstep, matching `morphEase` in `Settings.qml`.
local function smooth(t)
  return t * t * (3 - 2 * t)
end

-- Shared device state.
local volume = mold.signal("quickshell.settings.volume", 0.5)
local muted = mold.signal("quickshell.settings.muted", false)
local headphone = mold.signal("quickshell.settings.headphone", false)
local brightness = mold.signal("quickshell.settings.brightness", 0.7)
local app_count = mold.signal("quickshell.settings.app_count", 0)
local app_interacting = mold.signal("quickshell.settings.app_interacting", false)

local apps = {}
for index = 1, MAX_APPS do
  apps[index] = {
    -- The `pactl` sink input id, or 0 when the row is empty.
    id = mold.signal("quickshell.settings.app." .. index .. ".id", 0),
    name = mold.signal("quickshell.settings.app." .. index .. ".name", ""),
    value = mold.signal("quickshell.settings.app." .. index .. ".value", 0),
    dragging = false,
    pending = nil,
  }
end

--- One hover-driven panel's worth of state.
local function channel(prefix)
  return {
    prefix = prefix,
    bubble = morph("quickshell.settings." .. prefix .. ".bubble"),
    expand = morph("quickshell.settings." .. prefix .. ".expand"),
    pill_hover = mold.signal("quickshell.settings." .. prefix .. ".pill_hover", false),
    ext_hover = mold.signal("quickshell.settings." .. prefix .. ".ext_hover", false),
    interacting = mold.signal("quickshell.settings." .. prefix .. ".interacting", false),
    expanded = mold.signal("quickshell.settings." .. prefix .. ".expanded", false),
    shown = mold.signal("quickshell.settings." .. prefix .. ".shown", false),
    wide = mold.signal("quickshell.settings." .. prefix .. ".wide", false),
    -- Deadlines, in `clock` milliseconds. Nothing binds to them, so they stay
    -- plain Lua rather than becoming three more signals.
    dismiss_at = nil,
    shrink_at = nil,
    autohide_at = nil,
    pending = nil,
  }
end

local vol = channel("vol")
local bright = channel("bright")

-- ─── Panel state machine ───

--- Whether anything is holding this panel open, matching `updateVolPanel`.
local function held(ch)
  if ch.pill_hover:get() or ch.ext_hover:get() or ch.interacting:get() then return true end
  return ch == vol and app_interacting:get()
end

local function set_shown(ch, shown, now)
  if ch.shown:get() == shown then return end
  write(ch.shown, shown)
  if shown then
    drive(ch.bubble, 1, 240, "out_cubic", now)
  else
    drive(ch.bubble, 0, 180, "in_cubic", now)
  end
end

local function update_panel(ch, now)
  if held(ch) then
    ch.dismiss_at = nil
    set_shown(ch, true, now)
  else
    ch.dismiss_at = now + 300
  end
end

local function set_expanded(ch, expanded, now)
  if ch.expanded:get() == expanded then return end
  write(ch.expanded, expanded)
  update_panel(ch, now)
  if expanded then
    ch.shrink_at = nil
    write(ch.wide, true)
    drive(ch.expand, 1, 260, "out_cubic", now)
  else
    drive(ch.expand, 0, 220, "in_cubic", now)
    -- The wide flag outlives the fade, so the panel is still there to fade.
    ch.shrink_at = now + 260
  end
end

--- Re-arms the 30s collapse. The original only ever calls `restart()` on this
--- timer from an interaction, so an untouched panel stays open until the
--- pointer leaves; that is reproduced rather than corrected.
local function touch(ch, now)
  ch.autohide_at = now + 30000
end

-- ─── Device IO ───

local runners = {}

--- A reusable child process. Reassigning the command is what makes a finished
--- view runnable again, so one view covers every invocation of one command.
local function runner(command, on_output)
  local entry = {
    view = io.process_view { command = command, environment = CHILD_ENVIRONMENT },
    command = command,
    on_output = on_output,
    buffer = "",
    busy = false,
    next_at = 0,
  }
  runners[#runners + 1] = entry
  return entry
end

--- Starts one query, if it is not already running.
---
--- Spawning raises when the binary is not on PATH, so `busy` is only set once
--- a child actually exists. Marking it busy first would strand the entry
--- forever: the next drain would call `next` on a view with no child, which
--- raises in turn, and because the drain is the first thing the tick does the
--- whole timer would stop.
local function launch(entry, command)
  if entry.busy then return false end
  entry.buffer = ""
  local ok = pcall(function()
    entry.view:set_command(command or entry.command)
    entry.view:start()
  end)
  entry.busy = ok
  return ok
end

-- A drain waits on a child for at most this long, and one tick spends at most
-- this many waits across every running child. Output arrives while `next` waits,
-- so a purely non-blocking drain would never advance; the budget keeps that
-- from turning into a stall when several queries are in flight.
local DRAIN_SLICE_MS = 1
local DRAIN_BUDGET = 6

local function drain()
  local budget = DRAIN_BUDGET
  for _, entry in ipairs(runners) do
    while entry.busy and budget > 0 do
      budget = budget - 1
      local event = entry.view:next(DRAIN_SLICE_MS)
      if not event then break end
      if event.kind == "stdout" then
        entry.buffer = entry.buffer .. (event.data or "")
      elseif event.kind == "exit" then
        entry.busy = false
        if event.success and entry.on_output then entry.on_output(entry.buffer) end
      end
    end
  end
end

-- Queries.

local poll_mute = runner({ "pamixer", "--get-mute" }, function(out)
  local word = (out:match("(%S+)") or ""):lower()
  write(muted, word == "true" or word == "yes")
end)

local poll_volume = runner({ "pamixer", "--get-volume" }, function(out)
  local percent = tonumber(out:match("(%d+)"))
  if percent and not vol.interacting:get() then write(volume, percent / 100) end
end)

local poll_brightness = runner({ BRIGHT_COMMAND, "get" }, function(out)
  local percent = tonumber(out:match("([%d%.]+)"))
  if percent and not bright.interacting:get() then write(brightness, percent / 100) end
end)

-- The original's one-liner: read the default sink's active port and decide
-- whether it is a headset. `pactl` has no direct query for it, and
-- `mold.pipewire` reports no port information at all
-- (`crates/mold-services/src/pipewire/runtime.rs:282,287-310`), so the shell
-- pipeline is kept as-is.
local PORT_SCRIPT = "default_sink=$(pactl get-default-sink | tr -d '\\n'); "
  .. "pactl list sinks | awk -v target=\"$default_sink\" "
  .. "'/^\\s*Name: /{enabled = ($2 == target)} /^\\s*Active Port:/ {if (enabled){print; exit}}' "
  .. "| tr 'A-Z' 'a-z' | grep -qiE 'headphone|headset|bluez' && echo true || echo false"

local poll_port = runner({ "bash", "-c", PORT_SCRIPT }, function(out)
  write(headphone, (out:match("(%S+)") or "") == "true")
end)

--- Reads `pactl list sink-inputs` into the five fixed rows.
local function parse_sink_inputs(text)
  local list = {}
  local current = nil
  local function flush()
    if current and current.name and current.volume then list[#list + 1] = current end
    current = nil
  end
  for line in text:gmatch("[^\n]+") do
    local id = line:match("^Sink Input #(%d+)")
    if id then
      flush()
      current = { index = tonumber(id) }
    elseif current then
      if not current.volume and line:match("^%s*Volume:") then
        local percent = tonumber(line:match("(%d+)%%"))
        if percent then current.volume = percent / 100 end
      end
      local name = line:match('application%.name = "([^"]+)"')
      if name then current.name = name end
    end
  end
  flush()
  return list
end

local poll_apps = runner({ "pactl", "list", "sink-inputs" }, function(out)
  local list = parse_sink_inputs(out)
  local count = math.min(#list, MAX_APPS)
  for index = 1, MAX_APPS do
    local row = apps[index]
    local entry = index <= count and list[index] or nil
    local id = entry and entry.index or 0
    local name = entry and entry.name or ""
    if row.id:get() ~= id then write(row.id, id) end
    if row.name:get() ~= name then write(row.name, name) end
    -- A row being dragged keeps the value the pointer put there; letting the
    -- poll win would fight the drag.
    if entry and not row.dragging and row.value:get() ~= entry.volume then
      write(row.value, entry.volume)
    elseif not entry and row.value:get() ~= 0 then
      write(row.value, 0)
    end
  end
  if app_count:get() ~= count then write(app_count, count) end
end)

-- Writes. These are fire and forget, but still have to be reaped or the view
-- stays occupied by a zombie and refuses to start again, so they go through the
-- same drain with no output handler.

local set_volume = runner({ "pamixer", "--set-volume", "50" }, nil)
local toggle_mute = runner({ "pamixer", "-t" }, nil)
local set_brightness = runner({ BRIGHT_COMMAND, "50" }, nil)
local set_app_volume = runner({ "pactl", "set-sink-input-volume", "0", "50%" }, nil)

local function percent(value)
  return tostring(math.floor(math.max(0, math.min(1, value)) * 100 + 0.5))
end

--- Sends whatever the last seek asked for, one command at a time. The original
--- debounces its writes by 50ms; gating on the running child does the same job
--- without a second timer, and never queues a stale position behind a fresh one.
local function flush_writes()
  if vol.pending and launch(set_volume, { "pamixer", "--set-volume", percent(vol.pending) }) then
    vol.pending = nil
  end
  if bright.pending and launch(set_brightness, { BRIGHT_COMMAND, percent(bright.pending) }) then
    bright.pending = nil
  end
  for _, row in ipairs(apps) do
    if row.pending and row.id:get() > 0 then
      local command = {
        "pactl",
        "set-sink-input-volume",
        tostring(row.id:get()),
        percent(row.pending) .. "%",
      }
      if launch(set_app_volume, command) then row.pending = nil end
    end
  end
end

--- Starts whichever queries are due, matching the four polling `Timer`s.
local function schedule(now)
  if now >= poll_mute.next_at and launch(poll_mute) then
    poll_mute.next_at = now + 1000
  end
  if now >= poll_port.next_at and launch(poll_port) then
    poll_port.next_at = now + 3000
  end
  if vol.expanded:get() and not vol.interacting:get() and now >= poll_volume.next_at then
    if launch(poll_volume) then poll_volume.next_at = now + 750 end
  end
  if bright.expanded:get() and not bright.interacting:get() and now >= poll_brightness.next_at then
    if launch(poll_brightness) then poll_brightness.next_at = now + 750 end
  end
  if vol.expanded:get() and not app_interacting:get() and now >= poll_apps.next_at then
    if launch(poll_apps) then poll_apps.next_at = now + 1500 end
  end
end

--- Runs the deadlines one channel has come due for.
local function expire(ch, now)
  if ch.dismiss_at and now >= ch.dismiss_at then
    ch.dismiss_at = nil
    set_shown(ch, false, now)
    set_expanded(ch, false, now)
  end
  if ch.shrink_at and now >= ch.shrink_at then
    ch.shrink_at = nil
    write(ch.wide, false)
  end
  if ch.autohide_at and now >= ch.autohide_at then
    ch.autohide_at = nil
    set_expanded(ch, false, now)
  end
end

-- ─── Geometry and tree ───

--- The whole module, as one `ui.Item` subtree.
---
--- The four arguments are the ribbon geometry `line.lua` already computes, so
--- the settings pills land on the same bar at the same pitch.
function settings.build(bar_width, track_top, item_height, pill_gap)
  -- Dimensions, from the readonly block at the top of `Settings.qml`.
  local pill_width = bar_width * PILL_WIDTH_FACTOR
  local pill_h = item_height
  local button_size = item_height
  local border_padding = math.floor(SHORT * (6 / 2160) + 0.5)
  local reserved = bar_width + border_padding

  local panel_width = WIDTH * 0.2
  local panel_pad = theme.scaled(12)
  local card_pad = theme.scaled(8)
  local row_h = pill_h * 0.65
  local app_row_h = pill_h * 0.5
  local icon_size = row_h * 0.85
  local app_spacing = theme.scaled(6)
  local popup_gap = theme.scaled(24)
  local popup_slide = theme.scaled(46)
  local line_h = HEIGHT * 0.005
  local slash_thickness = theme.scaled(3.3)

  -- The radii and stroke widths from `Theme.qml`. They are measured against the
  -- output rather than the palette, so unlike a colour they are fixed for the
  -- run and are read once here instead of from inside a binding.
  local pill_radius = theme.pill_radius()
  local panel_radius = theme.panel_radius()
  local corner_radius = theme.corner_radius()
  local tiny_radius = theme.tiny_radius()
  local hairline = theme.border_width()
  local heavy_border = theme.heavy_border_width()
  local border_growth = theme.morph_border_growth()

  local main_card_h = card_pad * 2 + row_h
  local bright_panel_h = panel_pad * 2 + main_card_h

  -- `RibbonPopup.qml`'s two insets. The compact one tucks the panel a bar's
  -- width off the edge; the expanded one clears the bar and its padding.
  local compact_inset = -bar_width
  local expanded_inset = reserved + popup_gap
  local compact_w = button_size + popup_slide + (popup_gap * 0.33)
  local compact_h = pill_h

  -- The workspace track occupies the middle half of the output, so the two
  -- settings pills go directly beneath it at the same pitch.
  local track_bottom = track_top + (ROW_COUNT * item_height) + ((ROW_COUNT - 1) * pill_gap)
  local settings_top = track_bottom + (pill_gap * 2)

  local function app_card_h()
    local count = app_count:get()
    if count <= 0 then return 0 end
    return card_pad * 2 + count * app_row_h + math.max(0, count - 1) * app_spacing
  end

  local function vol_panel_h()
    local extra = app_count:get() > 0 and (panel_pad + app_card_h()) or 0
    return panel_pad + main_card_h + extra + panel_pad
  end

  vol.top = settings_top
  vol.height = vol_panel_h
  bright.top = settings_top + item_height + pill_gap
  bright.height = function() return bright_panel_h end

  -- The animated geometry of one panel, all of it a function of its two
  -- morphs. These are read from bindings, so each registers its caller on the
  -- morph signals and nothing else has to say what depends on what.
  local function ease(ch) return smooth(ch.expand:get()) end
  local function inset(ch)
    return compact_inset + (expanded_inset - compact_inset) * ch.expand:get()
  end
  local function surface_w(ch) return compact_w + (panel_width - compact_w) * ease(ch) end
  local function surface_h(ch) return compact_h + (ch.height() - compact_h) * ease(ch) end
  local function panel_x(ch) return inset(ch) + popup_slide * (1 - ease(ch)) end
  local function panel_w(ch) return button_size + (surface_w(ch) - button_size) * ease(ch) end
  local function panel_h(ch) return button_size + (surface_h(ch) - button_size) * ease(ch) end
  local function content_w(ch) return math.max(0, panel_w(ch) - panel_pad * 2) end

  -- The bubble's width uses the square of the progress, as the original does,
  -- so it stays pill-thin through the first half of the morph and then opens
  -- out. Its height never changes: `buttonSize` and `pillH` are the same number.
  local function bubble_w(ch)
    local t = ch.bubble:get()
    return pill_width + (button_size - pill_width) * (t * t)
  end
  local function bubble_x(ch) return inset(ch) + popup_slide * ch.bubble:get() end
  -- The original centres the bubble inside its own popup surface, whose
  -- `margins.top` already carries the channel's absolute position. That surface
  -- is dissolved into the shared overlay here, so the channel's own top has to
  -- be added back or every bubble stacks at the top of the screen.
  local function bubble_y(ch) return ch.top + (surface_h(ch) - pill_h) / 2 end

  --- Hover handlers shared by every mouse area inside a panel.
  ---
  --- The original stacks one `NoButton` MouseArea on top of the panel to track
  --- the pointer. mold delivers an event to a single node, the topmost hit, so
  --- an area on top would swallow every click beneath it; the tracker goes to
  --- the bottom of the stack instead and each interactive area keeps the flag
  --- up on its own. Exit is dispatched before enter
  --- (`crates/mold-cli/src/surface_events.rs:85-88`), so moving between two of
  --- them never reads as a gap.
  local function hover(ch)
    return function() write(ch.ext_hover, true) update_panel(ch, clock:elapsed_ms()) end,
      function() write(ch.ext_hover, false) update_panel(ch, clock:elapsed_ms()) end
  end

  -- ─── ProgressBar.qml ───

  --- A three-piece slider: a filled past track, a rounded indicator, and an
  --- empty future track, with a gap punched either side of the indicator.
  ---
  --- `origin` reports the bar's absolute left edge. Pointer events carry surface
  --- coordinates rather than node-local ones
  --- (`crates/mold-cli/src/surface_events.rs:94`), so the fraction has to be
  --- worked out against it.
  local function progress_bar(cfg)
    local gap = line_h * 0.8
    local indicator_w = line_h * 0.3
    local indicator_h = line_h * 2
    local function position() return cfg.width() * math.max(0, math.min(1, cfg.value())) end

    local past = ui.Rect {
      x = 0,
      y = function() return (cfg.height() - line_h) / 2 end,
      width = function() return math.max(0, position() - gap) end,
      height = line_h,
      radius = line_h * 0.2,
      color = cfg.fill,
      behavior = { width = { duration = 200, easing = "out_cubic" } },
    }
    local indicator = ui.Rect {
      x = function() return position() - indicator_w / 2 end,
      y = function() return (cfg.height() - indicator_h) / 2 end,
      width = indicator_w,
      height = indicator_h,
      radius = indicator_w * 0.5,
      color = cfg.fill,
      behavior = { x = { duration = 200, easing = "out_cubic" } },
    }
    local future = ui.Rect {
      x = function() return position() + gap end,
      y = function() return (cfg.height() - line_h) / 2 end,
      -- Both of these are affine in the position, so animating them with the
      -- same curve keeps the track's right edge pinned while its left edge
      -- slides, which is what binding width to the animating x does in QML.
      width = function() return math.max(0, cfg.width() - position() - gap) end,
      height = line_h,
      radius = line_h * 0.2,
      color = cfg.track,
      behavior = {
        x = { duration = 200, easing = "out_cubic" },
        width = { duration = 200, easing = "out_cubic" },
      },
    }

    local last_x = 0
    local pressed = false

    --- Turns a surface x into a 0..1 position along the bar.
    local function fraction(x)
      local width = cfg.width()
      if width <= 0 then return 0 end
      return math.max(0, math.min(1, (x - cfg.origin()) / width))
    end

    --- Suppresses the smoothing while the pointer is driving the value, as the
    --- original's `Behavior { enabled: !mouseArea.pressed }` does.
    local function smoothing(enabled)
      mold.animation.set_enabled(past, "width", enabled)
      mold.animation.set_enabled(indicator, "x", enabled)
      mold.animation.set_enabled(future, "x", enabled)
      mold.animation.set_enabled(future, "width", enabled)
    end

    return ui.Item {
      x = cfg.x,
      y = cfg.y,
      width = cfg.width,
      height = cfg.height,
      visible = cfg.visible,
      past,
      indicator,
      future,
      ui.MouseArea {
        anchors = { fill = true },
        on_entered = cfg.on_entered,
        on_exited = cfg.on_exited,
        on_position_changed = function(x)
          last_x = x
          if pressed then cfg.on_seek(fraction(x)) end
        end,
        on_dragged = function(x)
          last_x = x
          cfg.on_seek(fraction(x))
        end,
        on_pressed = function()
          pressed = true
          smoothing(false)
          cfg.on_seek(fraction(last_x))
        end,
        on_released = function()
          pressed = false
          smoothing(true)
          cfg.on_commit(fraction(last_x))
        end,
      },
    }
  end

  -- ─── The morphing icon bubble ───

  local function bubble(ch, icon, fill, show_slash)
    local enter, leave = hover(ch)
    local children = {
      x = function() return bubble_x(ch) end,
      y = function() return bubble_y(ch) end,
      width = function() return bubble_w(ch) end,
      height = pill_h,
      radius = function()
        return pill_radius + (button_size * 0.5 - pill_radius) * ch.bubble:get()
      end,
      color = fill,
      border_color = function() return theme.color0() end,
      border_width = function()
        return heavy_border + border_growth * ch.bubble:get()
      end,
      opacity = function() return 1 - ch.expand:get() end,
      -- Stays up through the fade-out, and steps aside once the wide panel has
      -- all but replaced it.
      visible = function()
        return (ch.shown:get() or ch.bubble:get() > 0) and ch.expand:get() < 0.98
      end,
      behavior = { color = { duration = 200, easing = "out_cubic" } },

      ui.Text {
        x = 0,
        y = 0,
        width = function() return bubble_w(ch) end,
        height = pill_h,
        text = icon,
        color = function() return theme.color0() end,
        font_family = theme.font,
        font_source = theme.font_source,
        font_size = function() return math.min(bubble_w(ch), pill_h) * 0.495 end,
        font_weight = 900,
        horizontal_alignment = "center",
        vertical_alignment = "center",
      },

      ui.MouseArea {
        anchors = { fill = true },
        on_entered = enter,
        on_exited = leave,
        on_clicked = function()
          local now = clock:elapsed_ms()
          if not ch.expanded:get() then set_expanded(ch, true, now) end
        end,
      },
    }
    if show_slash then
      -- The mute slash: a rounded bar across the diagonal of the bubble. Only
      -- the volume bubble has one, so it is spliced in rather than always
      -- present and hidden.
      table.insert(children, #children, ui.Rect {
        x = function() return (bubble_w(ch) - math.sqrt(bubble_w(ch) ^ 2 + pill_h ^ 2) * 0.5) / 2 end,
        y = (pill_h - slash_thickness) / 2,
        width = function() return math.sqrt(bubble_w(ch) ^ 2 + pill_h ^ 2) * 0.5 end,
        height = slash_thickness,
        radius = tiny_radius,
        rotation = -45,
        color = function() return theme.color0() end,
        opacity = 0.9,
        visible = show_slash,
      })
    end
    return ui.Rect(children)
  end

  -- ─── The expanded card stack ───

  --- The circular icon at the head of a card, with its optional mute slash.
  local function card_icon(ch, icon, tint, on_click, mute_aware)
    local enter, leave = hover(ch)
    local children = {
      ui.Text {
        x = 0,
        y = 0,
        width = icon_size,
        height = icon_size,
        text = icon,
        color = function()
          if mute_aware and muted:get() then return theme.color244() end
          return theme.color1()
        end,
        font_family = theme.font,
        font_source = theme.font_source,
        font_size = icon_size * 0.5,
        horizontal_alignment = "center",
        vertical_alignment = "center",
      },
    }
    if mute_aware then
      local diagonal = math.sqrt(icon_size ^ 2 + icon_size ^ 2) * 0.5
      children[#children + 1] = ui.Rect {
        x = (icon_size - diagonal) / 2,
        y = (icon_size - slash_thickness) / 2,
        width = diagonal,
        height = slash_thickness,
        radius = tiny_radius,
        rotation = -45,
        color = function() return theme.color0() end,
        opacity = 0.95,
        visible = function() return muted:get() end,
      }
    end
    children[#children + 1] = ui.MouseArea {
      anchors = { fill = true },
      on_entered = enter,
      on_exited = leave,
      on_clicked = on_click,
    }
    children.x = card_pad
    children.y = card_pad + (row_h - icon_size) / 2
    children.width = icon_size
    children.height = icon_size
    children.radius = icon_size / 2
    children.color = tint
    children.behavior = { color = { duration = 200 } }
    return ui.Rect(children)
  end

  --- The main volume or brightness card: icon, then slider.
  local function main_card(ch, icon, tint, on_icon_click, value, on_seek, on_commit, mute_aware)
    local enter, leave = hover(ch)
    local function bar_x() return card_pad + icon_size + card_pad end
    return ui.Rect {
      x = panel_pad,
      y = panel_pad,
      width = function() return content_w(ch) end,
      height = main_card_h,
      radius = corner_radius,
      color = function() return theme.color236() end,
      border_width = hairline,
      border_color = function() return mix(theme.color244(), theme.color238(), 0.08) end,

      card_icon(ch, icon, tint, on_icon_click, mute_aware),

      progress_bar {
        x = bar_x,
        y = function() return card_pad end,
        width = function() return math.max(0, content_w(ch) - card_pad * 3 - icon_size) end,
        height = function() return row_h end,
        origin = function() return panel_x(ch) + panel_pad + bar_x() end,
        value = value,
        fill = function() return theme.color1() end,
        track = function() return mix(theme.color244(), theme.color236(), 0.15) end,
        on_entered = enter,
        on_exited = leave,
        on_seek = on_seek,
        on_commit = on_commit,
      },
    }
  end

  --- One per-application volume row. Five are built and the surplus is hidden,
  --- because the tree is fixed at startup and the sink input list is not.
  local function app_row(index)
    local row = apps[index]
    local enter, leave = hover(vol)
    local function row_w() return math.max(0, content_w(vol) - card_pad * 2) end
    local function visible() return row.id:get() > 0 end
    local function seek(position)
      write(row.value, position)
      row.dragging = true
      write(app_interacting, true)
      row.pending = position
      touch(vol, clock:elapsed_ms())
    end
    return ui.Item {
      x = card_pad,
      y = card_pad + (index - 1) * (app_row_h + app_spacing),
      width = row_w,
      height = app_row_h,
      visible = visible,

      ui.Text {
        x = 0,
        y = 0,
        width = function() return row_w() * 0.25 end,
        height = app_row_h,
        text = function() return row.name:get() end,
        elide = "right",
        color = function() return mix(theme.color1(), theme.color236(), 0.7) end,
        font_family = theme.font,
        font_source = theme.font_source,
        font_size = app_row_h * 0.45,
        vertical_alignment = "center",
      },

      progress_bar {
        x = function() return row_w() * 0.25 + card_pad end,
        y = 0,
        width = function() return math.max(0, row_w() * 0.75 - card_pad) end,
        height = function() return app_row_h end,
        origin = function()
          return panel_x(vol) + panel_pad + card_pad + row_w() * 0.25 + card_pad
        end,
        value = function() return row.value:get() end,
        fill = function() return theme.color1() end,
        track = function() return mix(theme.color244(), theme.color236(), 0.15) end,
        visible = visible,
        on_entered = enter,
        on_exited = leave,
        on_seek = seek,
        on_commit = function(position)
          seek(position)
          row.dragging = false
          local any = false
          for _, other in ipairs(apps) do any = any or other.dragging end
          write(app_interacting, any)
        end,
      },
    }
  end

  --- The card holding the per-application rows.
  local function app_card()
    local children = {
      x = panel_pad,
      y = function() return panel_pad + main_card_h + panel_pad end,
      width = function() return content_w(vol) end,
      height = app_card_h,
      radius = corner_radius,
      color = function() return theme.color236() end,
      border_width = hairline,
      border_color = function() return mix(theme.color244(), theme.color238(), 0.08) end,
      visible = function() return app_count:get() > 0 end,
    }
    for index = 1, MAX_APPS do
      children[#children + 1] = app_row(index)
    end
    return ui.Rect(children)
  end

  --- The board-style surface the bubble grows into.
  local function panel(ch, cards)
    local children = {
      x = function() return panel_x(ch) end,
      y = ch.top,
      width = function() return panel_w(ch) end,
      height = function() return panel_h(ch) end,
      color = function() return theme.color238() end,
      radius = function()
        return button_size * 0.5 + (panel_radius - button_size * 0.5) * ease(ch)
      end,
      opacity = function() return ch.expand:get() end,
      border_width = hairline,
      -- The only alpha left unresolved: what is behind the panel is whatever
      -- window it happens to be over, so there is nothing to blend against.
      border_color = function() return theme.alpha("color244", 0.08 * ch.expand:get()) end,
      visible = function() return ch.wide:get() or ch.expand:get() > 0 end,
    }
    for _, card in ipairs(cards) do children[#children + 1] = card end
    return ui.Rect(children)
  end

  --- The pointer tracker, sized to the surface the original would have opened.
  --- Being invisible when the panel is down keeps it out of the input region,
  --- which mold re-derives from live MouseArea geometry every paint
  --- (`crates/mold-cli/src/paint.rs:26-42`).
  local function tracker(ch)
    local enter, leave = hover(ch)
    return ui.MouseArea {
      x = function() return inset(ch) end,
      y = ch.top,
      width = function() return surface_w(ch) end,
      height = function() return surface_h(ch) end,
      visible = function()
        return ch.shown:get() or ch.bubble:get() > 0 or ch.expand:get() > 0
      end,
      -- `Qt.NoButton`: it watches the pointer and takes no clicks.
      accepted_buttons = {},
      on_entered = enter,
      on_exited = leave,
    }
  end

  --- One ribbon pill. The compact/expanded/hovered machine is three discrete
  --- looks with one 200ms crossfade between any two of them, which is what
  --- mold's states and transitions are
  --- (`crates/mold-lua/src/configure.rs:160-252`); the morphs above are not,
  --- because their curves are not linear in the progress.
  local function pill(ch, y)
    local hovered = mold.signal("quickshell.settings." .. ch.prefix .. ".hovered", false)
    return ui.Rect {
      x = (bar_width - pill_width) / 2,
      y = y,
      width = pill_width,
      height = pill_h,
      radius = pill_radius,
      color = function() return theme.color240() end,
      opacity = 0.6,
      state = function()
        if ch.expanded:get() then return "expanded" end
        return hovered:get() and "hovered" or "idle"
      end,
      states = {
        idle = {
          property_changes = {
            color = function() return theme.color240() end,
            opacity = 0.6,
          },
        },
        hovered = {
          property_changes = {
            color = function() return theme.color240() end,
            opacity = 0.9,
          },
        },
        expanded = {
          property_changes = {
            color = function() return theme.color1() end,
            opacity = 1.0,
          },
        },
      },
      transitions = { { from = "*", to = "*", duration = 200 } },

      ui.MouseArea {
        anchors = { fill = true },
        on_entered = function()
          write(hovered, true)
          write(ch.pill_hover, true)
          update_panel(ch, clock:elapsed_ms())
        end,
        on_exited = function()
          write(hovered, false)
          write(ch.pill_hover, false)
          update_panel(ch, clock:elapsed_ms())
        end,
        on_clicked = function()
          local now = clock:elapsed_ms()
          if ch.expanded:get() then set_expanded(ch, false, now) end
        end,
      },
    }
  end

  -- Icons, from the `volumeIcon` and `brightnessIcon` bindings.
  local function volume_icon()
    return headphone:get() and "\u{f02cb}" or "\u{f057e}"
  end
  local BRIGHTNESS_ICON = "\u{f00e0}"

  local vol_seek = function(position)
    write(vol.interacting, true)
    write(volume, position)
    vol.pending = position
    touch(vol, clock:elapsed_ms())
  end
  local bright_seek = function(position)
    write(bright.interacting, true)
    write(brightness, position)
    bright.pending = position
    touch(bright, clock:elapsed_ms())
  end

  local vol_panel = panel(vol, {
    main_card(
      vol,
      volume_icon,
      function()
        return mix(theme.color1(), theme.color236(), muted:get() and 0.05 or 0.15)
      end,
      function()
        write(muted, not muted:get())
        launch(toggle_mute)
        touch(vol, clock:elapsed_ms())
      end,
      function() return volume:get() end,
      vol_seek,
      function(position)
        write(volume, position)
        write(vol.interacting, false)
        vol.pending = position
        touch(vol, clock:elapsed_ms())
      end,
      true
    ),
    app_card(),
  })

  local bright_panel = panel(bright, {
    main_card(
      bright,
      BRIGHTNESS_ICON,
      function() return mix(theme.color1(), theme.color236(), 0.15) end,
      function() end,
      function() return brightness:get() end,
      bright_seek,
      function(position)
        write(brightness, position)
        write(bright.interacting, false)
        bright.pending = position
        touch(bright, clock:elapsed_ms())
      end
    ),
  })

  -- Order is paint order and hit order both: mold's renderer and its hit test
  -- walk children in declaration order and ignore `z`, so the bubble is
  -- declared after the panel it overlaps (the original gives it `z: 1`), the
  -- tracker before everything it sits behind, and the pills last so they stay
  -- hoverable where a panel reaches back under the bar.
  return ui.Item {
    width = math.ceil(expanded_inset + panel_width),
    height = HEIGHT,

    tracker(vol),
    vol_panel,
    bubble(vol, volume_icon, function()
      return muted:get() and theme.color240() or theme.color1()
    end, function() return muted:get() end),

    tracker(bright),
    bright_panel,
    bubble(bright, BRIGHTNESS_ICON, function() return theme.color1() end),

    pill(vol, settings_top),
    pill(bright, settings_top + item_height + pill_gap),

    -- The one timer. Ten in the original: three dismiss and shrink deadlines
    -- per channel, and four polls.
    ui.Timer {
      interval = TICK_MS,
      ["repeat"] = true,
      running = true,
      on_triggered = function()
        local now = clock:elapsed_ms()
        drain()
        expire(vol, now)
        expire(bright, now)
        for _, entry in ipairs(morphs) do advance(entry, now) end
        flush_writes()
        schedule(now)
      end,
    },
  }
end

--- The horizontal extent the panels can reach, for a caller that has to declare
--- an input region by hand instead of letting the paint derive one.
function settings.input_region(bar_width, track_top, item_height, pill_gap)
  local border_padding = math.floor(SHORT * (6 / 2160) + 0.5)
  local popup_gap = theme.scaled(24)
  local panel_pad = theme.scaled(12)
  local card_pad = theme.scaled(8)
  local app_spacing = theme.scaled(6)
  local row_h = item_height * 0.65
  local app_row_h = item_height * 0.5
  local main_card_h = card_pad * 2 + row_h
  local app_card = card_pad * 2 + MAX_APPS * app_row_h + (MAX_APPS - 1) * app_spacing
  local tallest = panel_pad * 3 + main_card_h + app_card
  local track_bottom = track_top + (ROW_COUNT * item_height) + ((ROW_COUNT - 1) * pill_gap)
  local top = track_bottom + (pill_gap * 2)
  return {
    x = 0,
    y = math.floor(top),
    width = math.ceil(bar_width + border_padding + popup_gap + WIDTH * 0.2),
    height = math.ceil((item_height + pill_gap) + tallest),
  }
end

return settings
