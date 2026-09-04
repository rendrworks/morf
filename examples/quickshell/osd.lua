-- On-screen displays, ported from `~/.config/quickshell/osd/modules/osd/`.
--
-- The original is three `PanelWindow`s driven by an `OSDManager` scope: volume,
-- brightness, and a battery warning. Each is bottom-anchored, centred, fully
-- transparent, `exclusiveZone: 0` and `keyboardFocus: None`, and each fades in
-- on a state change and out again after a delay.
--
-- morf hosts one layer surface per process, so all three are nodes inside the
-- shell's existing fullscreen overlay rather than surfaces of their own. That
-- is the better shape here, not a compromise: a popup surface is unmapped the
-- instant it stops being visible (`sync_window_surfaces` in
-- `morf-cli/src/surfaces.rs` closes every popup that is not effectively
-- visible, on the same pass that would have drawn the fade), which
-- would cut the fade-out at frame zero. As plain nodes nothing is destroyed, so
-- the 200ms `Behavior on opacity` actually runs to completion.
--
-- Placement. `osd.build()` returns one `ui.Item` the size of the output, with
-- the panels positioned absolutely inside it, exactly as `border.build()` does.
-- It therefore expects to sit at the surface origin. Pointer events carry
-- surface coordinates (`morf-cli/src/surface_events.rs`, `PointerMotion`), and
-- morf exposes no way to ask a node for its laid-out absolute position, so the
-- sliders convert surface x to a bar-local fraction using the geometry computed
-- here. `osd.set_origin(x, y)` corrects that if the subtree is placed elsewhere.

local morf = require("morf")
local ui = require("morf.ui")
local io = require("morf.io")
local core = require("morf.core")
local theme = require("theme")

local osd = {}

local WIDTH, HEIGHT = theme.reference()

-- ---------------------------------------------------------------------------
-- Geometry
--
-- Straight out of VolumeOSD.qml / BrightnessOSD.qml / BatteryOSD.qml. Every
-- OSD computes the same four numbers from the monitor size.
-- ---------------------------------------------------------------------------

local function round(value) return math.floor(value + 0.5) end

local OSD_WIDTH = round(WIDTH * 0.272)
local OSD_HEIGHT = round(HEIGHT * 0.065)
local OSD_PADDING = round(OSD_HEIGHT * 0.32)
local OSD_GAP = round(OSD_HEIGHT * 0.22)
local BOTTOM_MARGIN = round(HEIGHT * 0.05)

-- `anchors.bottom` with `margins.bottom`, on a panel of `implicitHeight`.
local PANEL_Y = HEIGHT - BOTTOM_MARGIN - OSD_HEIGHT
local PANEL_X = round((WIDTH - OSD_WIDTH) / 2)

-- The battery warning is a square of side `osdSize`, not a bar.
local BATTERY_SIZE = OSD_HEIGHT
local BATTERY_X = round((WIDTH - BATTERY_SIZE) / 2)

-- `osd/Theme.qml` scales its border against a 2160px short side, same as the
-- rest of the shell; `theme.heavy_border_width()` is that 2/2160.
local BORDER_WIDTH = theme.heavy_border_width()

-- The icon button and the progress plate, from the `iconCircle` and
-- `progressRect` blocks. `progressRect` fills what the circle and the two
-- paddings leave.
local CIRCLE = OSD_HEIGHT * 0.6
local CIRCLE_X = OSD_PADDING
local CIRCLE_Y = (OSD_HEIGHT - CIRCLE) / 2
local PLATE_X = OSD_PADDING + CIRCLE + OSD_GAP
local PLATE_HEIGHT = OSD_HEIGHT * 0.4
local PLATE_Y = (OSD_HEIGHT - PLATE_HEIGHT) / 2
local PLATE_WIDTH = math.max(0, OSD_WIDTH - PLATE_X - OSD_PADDING)

-- StyledProgressBar is centred in the plate at 95% width, 80% height, and its
-- `lineHeight` is that same 80% — so the track fills the bar item exactly.
local BAR_WIDTH = PLATE_WIDTH * 0.95
local BAR_HEIGHT = PLATE_HEIGHT * 0.8
local BAR_X = PLATE_X + (PLATE_WIDTH - BAR_WIDTH) / 2
local BAR_Y = PLATE_Y + (PLATE_HEIGHT - BAR_HEIGHT) / 2
local LINE_HEIGHT = BAR_HEIGHT

local INDICATOR_WIDTH = math.max(1, LINE_HEIGHT * (3 / 60))
local INDICATOR_GAP = math.max(3, LINE_HEIGHT * (8 / 60))
local TRACK_RADIUS = math.max(1, LINE_HEIGHT * (2 / 60))
-- Twice the track height, vertically centred on it, so it overhangs the plate
-- by a quarter of its own height. Nothing clips it in the original either.
local INDICATOR_HEIGHT = LINE_HEIGHT * 2
local INDICATOR_Y = (LINE_HEIGHT - INDICATOR_HEIGHT) / 2

-- Where the surface places this subtree. Pointer events arrive in surface
-- coordinates; the sliders subtract this to get back to bar-local ones.
local origin_x, origin_y = 0, 0

--- Declares where the built subtree sits on the surface. Defaults to (0, 0).
function osd.set_origin(x, y)
  origin_x, origin_y = x or 0, y or 0
end

-- ---------------------------------------------------------------------------
-- Timing, from the original's Timers
-- ---------------------------------------------------------------------------

local POLL_INTERVAL = 750     -- volume and brightness checks
local SINK_INTERVAL = 3000    -- headphone detection
local BATTERY_INTERVAL = 10000
local SUBSCRIBE_RETRY = 5000  -- before a dead `pactl subscribe` is tried again
local HIDE_INTERVAL = 1500    -- hideTimer
local FADE_DURATION = 200     -- Behavior on opacity
local PULSE_DURATION = 600    -- the battery SequentialAnimation's two halves
local TICK_INTERVAL = 60      -- process drain and hide deadlines

local BATTERY_THRESHOLD = 0.15
local WARNING = morf.color "#d32f2f"

-- ---------------------------------------------------------------------------
-- State
-- ---------------------------------------------------------------------------

local volume = morf.signal("quickshell.osd.volume", 0.5)
local volume_muted = morf.signal("quickshell.osd.volume_muted", false)
local volume_headphone = morf.signal("quickshell.osd.volume_headphone", false)
local volume_shown = morf.signal("quickshell.osd.volume_shown", false)

local brightness = morf.signal("quickshell.osd.brightness", 0.7)
local brightness_shown = morf.signal("quickshell.osd.brightness_shown", false)

local battery = morf.signal("quickshell.osd.battery", 1.0)
local battery_charging = morf.signal("quickshell.osd.battery_charging", false)
local battery_shown = morf.signal("quickshell.osd.battery_shown", false)
-- Drives the warning's flash. Toggled by a timer that only exists while the
-- battery is actually critical, so nothing asks for frames the rest of the time.
local battery_pulse = morf.signal("quickshell.osd.battery_pulse", 1.0)

-- `lastVolume` / `lastBrightness`: a poll only shows the OSD when the value
-- actually moved, so a steady state does not keep it on screen.
local last_volume = -1
local last_brightness = -1

-- `isInteracting`: while a slider is held, polling must not fight the drag.
local volume_interacting = false
local brightness_interacting = false

-- The pointer position last reported by a motion event, in bar-local
-- fractions. Press and release carry no coordinates, so they read this.
local volume_pointer = 0
local brightness_pointer = 0

local volume_hide = morf.elapsed_timer()
local brightness_hide = morf.elapsed_timer()

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--- `shouldShowOSD = true; hideTimer.restart()`.
local function show(shown, hide)
  write(shown, true)
  hide:restart()
end

-- ---------------------------------------------------------------------------
-- Child processes
--
-- Every view is created here, at module load. One made later, inside a timer
-- callback, is not picked up by the running service loop and never reports a
-- line. `LD_LIBRARY_PATH` is cleared because morf runs under a nixGL-style
-- wrapper that points it at nix store paths; a system binary that inherits
-- those fails to load its own libraries and exits before printing anything.
-- ---------------------------------------------------------------------------

local CHILD_ENVIRONMENT = { LD_LIBRARY_PATH = "" }

local jobs = {}

--- Declares one reusable child process and how to read its output.
local function job(command, on_done)
  local entry = {
    process = io.process_view { command = command, environment = CHILD_ENVIRONMENT },
    command = command,
    buffer = "",
    busy = false,
    on_done = on_done,
  }
  jobs[#jobs + 1] = entry
  return entry
end

--- Starts a job if it is not already running, optionally with new arguments.
---
--- A call that arrives while the last one is still in flight is queued rather
--- than dropped, which is what `Proc.qml` does with its `entry.pending` flag.
--- It matters at the end of a drag: the release lands on top of the last seek,
--- and without the queue the final position would never reach the sink.
local function run(entry, command)
  command = command or entry.command
  if entry.busy then
    entry.pending = command
    return false
  end
  entry.pending = nil
  entry.buffer = ""
  entry.busy = true
  -- Reassigning the command is what makes a finished view runnable again.
  entry.process:set_command(command)
  entry.process:start()
  return true
end

-- How much of one tick a drain may spend, and over how many reads. The budget
-- is small and only spent while a job is actually in flight, so the work is
-- spread across ticks rather than blocking the frame loop on a slow child.
local DRAIN_SLICE_MS = 2
local DRAIN_SLICES = 8

--- Collects whatever the running jobs have produced.
local function drain()
  for _, entry in ipairs(jobs) do
    if entry.busy then
      for _ = 1, DRAIN_SLICES do
        local event = entry.process:next(DRAIN_SLICE_MS)
        if not event then break end
        if event.kind == "stdout" then
          entry.buffer = entry.buffer .. (event.data or "")
        elseif event.kind == "exit" then
          entry.busy = false
          if entry.on_done then entry.on_done(entry.buffer, event.success) end
          local pending = entry.pending
          if pending then
            entry.pending = nil
            run(entry, pending)
          end
          break
        end
      end
    end
  end
end

local function trimmed(text)
  return (tostring(text or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

-- Volume. `pamixer --get-volume` and `--get-mute`, as two calls, matching the
-- two `runCommand`s in the original's 750ms timer.
local volume_get = job({ "pamixer", "--get-volume" }, function(output, success)
  if not success then return end
  local level = tonumber(trimmed(output))
  if not level then return end
  local next_volume = level / 100
  if math.abs(next_volume - last_volume) > 0.01 or last_volume < 0 then
    write(volume, next_volume)
    last_volume = next_volume
    show(volume_shown, volume_hide)
  end
end)

local mute_get = job({ "pamixer", "--get-mute" }, function(output, success)
  if not success then return end
  local muted = trimmed(output) == "true"
  if muted ~= volume_muted:get() then
    write(volume_muted, muted)
    show(volume_shown, volume_hide)
  end
end)

-- Headphone detection, on its own slower timer in the original.
local sink_get = job({
  "bash", "-c",
  "pactl get-default-sink && pactl list sinks | grep -i 'Active Port'",
}, function(output, success)
  if not success then return end
  local lower = tostring(output):lower()
  local headphone = lower:find("bluez", 1, true) ~= nil
    or lower:find("headphone", 1, true) ~= nil
  if headphone ~= volume_headphone:get() then
    write(volume_headphone, headphone)
  end
end)

-- PulseAudio announces its own changes, so the volume does not have to be
-- asked for. `pactl subscribe` is one long-lived child that prints a line
-- whenever a sink or the server changes; asking on a timer instead meant two
-- `pamixer` forks every 750ms, and morf runs one worker per output, so on a
-- three-monitor desktop that was the single most expensive thing the shell
-- did. The timer stays as a fallback for when the subscription is unavailable.
local sink_events = io.process_view {
  command = { "pactl", "subscribe" },
  environment = CHILD_ENVIRONMENT,
}
local sink_running = false
-- One event feeds two consumers on different clocks: the volume reads it the
-- moment it arrives, the headphone pipeline only when its own timer comes
-- round, so each clears its own flag.
-- Both start set so the first tick reads the current state; after that they
-- are raised only by the subscription.
local sink_changed = true
local sink_ports_changed = true
local sink_retry = core.elapsed_timer()

local function start_sink_events()
  sink_retry:restart()
  sink_running = pcall(function() sink_events:start() end)
end

--- Reads what the subscription has to say, and restarts it when it dies.
---
--- A `pactl` that is missing, or a PulseAudio that goes away, must not turn
--- into a spawn loop, so a failed start waits out the retry interval.
local function drain_sink_events()
  if not sink_running then
    if sink_retry:elapsed_ms() >= SUBSCRIBE_RETRY then start_sink_events() end
    return
  end
  for _ = 1, DRAIN_SLICES do
    local event = sink_events:next(DRAIN_SLICE_MS)
    if not event then break end
    if event.kind == "stdout" then
      local text = tostring(event.data or ""):lower()
      if text:find("sink", 1, true) or text:find("server", 1, true) then
        sink_changed = true
        sink_ports_changed = true
      end
    elseif event.kind == "exit" then
      sink_running = false
      sink_retry:restart()
      break
    end
  end
end

start_sink_events()

local volume_set = job({ "pamixer", "--set-volume", "50" })
local mute_toggle = job({ "pamixer", "-t" })

-- Brightness. The original shells out to `~/.local/sbin/bright get`; that
-- script only reads two sysfs files and converts between them, so the read is
-- done here directly and no child is spawned for it. Writing still goes
-- through the script, which is what owns the mapping and the clamping.
local BRIGHT = core.env("MORF_BRIGHT") or ((core.env("HOME") or "") .. "/.local/sbin/bright")
local BACKLIGHT = core.env("MORF_BACKLIGHT") or "/sys/class/backlight/intel_backlight"

local brightness_set = job({ BRIGHT, "50" })

local backlight_view = io.file_view { path = BACKLIGHT .. "/brightness", preload = true }
local backlight_max_view = io.file_view { path = BACKLIGHT .. "/max_brightness", preload = true }

-- `bright` maps raw to a perceptual percent as
-- `log(raw/min) / log(max/min)`, with min pinned at 1 and max read from sysfs.
local BACKLIGHT_MIN = 1
local backlight_max = 400
if backlight_max_view:loaded() then
  backlight_max = tonumber(trimmed(backlight_max_view:text())) or backlight_max
end
if backlight_max <= BACKLIGHT_MIN then backlight_max = BACKLIGHT_MIN + 1 end
local BACKLIGHT_SPAN = math.log(backlight_max / BACKLIGHT_MIN)

local function backlight_level()
  if not backlight_view:reload() then return nil end
  local raw = tonumber(trimmed(backlight_view:text()))
  if not raw then return nil end
  if raw < BACKLIGHT_MIN then raw = BACKLIGHT_MIN end
  if raw > backlight_max then raw = backlight_max end
  return math.log(raw / BACKLIGHT_MIN) / BACKLIGHT_SPAN
end

-- Battery. The original runs `bash -c "cat capacity status"`; both files are
-- read here directly instead, which is the same two reads without the child.
local BATTERY = core.env("MORF_BATTERY") or "/sys/class/power_supply/BAT0"
local capacity_view = io.file_view { path = BATTERY .. "/capacity", preload = true }
local status_view = io.file_view { path = BATTERY .. "/status", preload = true }

-- ---------------------------------------------------------------------------
-- Polling
-- ---------------------------------------------------------------------------

local function poll_volume()
  if volume_interacting then return end
  run(volume_get)
  run(mute_get)
end

local function poll_brightness()
  if brightness_interacting then return end
  local level = backlight_level()
  if not level then return end
  if math.abs(level - last_brightness) > 0.01 or last_brightness < 0 then
    write(brightness, level)
    last_brightness = level
    show(brightness_shown, brightness_hide)
  end
end

local function poll_battery()
  if not capacity_view:reload() or not status_view:reload() then return end
  local capacity = tonumber(trimmed(capacity_view:text()))
  if not capacity then return end
  local charging = trimmed(status_view:text()) == "Charging"
  local level = capacity / 100
  write(battery, level)
  write(battery_charging, charging)
  local critical = level < BATTERY_THRESHOLD and not charging
  if critical ~= battery_shown:get() then
    write(battery_shown, critical)
    -- Start the flash on its dark half so the warning reads as a pulse from
    -- the first frame, and settle it back to full when it stops.
    write(battery_pulse, critical and 0.3 or 1.0)
  end
end

--- Retires an OSD once its hide interval has run out, matching `hideTimer`.
local function expire(shown, hide, interacting)
  if not shown:get() or interacting then return end
  if hide:elapsed_ms() >= HIDE_INTERVAL then
    write(shown, false)
  end
end

-- ---------------------------------------------------------------------------
-- Interaction
-- ---------------------------------------------------------------------------

--- Converts a surface x to a fraction along a slider, clamped to it.
---
--- Pointer coordinates arrive in surface space, while `BAR_X` is measured
--- inside the panel, so both the panel's offset and the subtree's own origin
--- come off before the division.
local function slider_position(x)
  local fraction = (x - origin_x - PANEL_X - BAR_X) / BAR_WIDTH
  if fraction < 0 then return 0 end
  if fraction > 1 then return 1 end
  return fraction
end

--- `onSeeking`: track the drag live and follow it with the sink.
local function volume_seek(position)
  volume_interacting = true
  write(volume, position)
  run(volume_set, { "pamixer", "--set-volume", tostring(round(position * 100)) })
end

--- `onClicked`: commit, release the poll, and re-arm the hide.
local function volume_commit()
  local position = volume_pointer
  write(volume, position)
  last_volume = position
  run(volume_set, { "pamixer", "--set-volume", tostring(round(position * 100)) })
  volume_interacting = false
  show(volume_shown, volume_hide)
end

local function brightness_seek(position)
  brightness_interacting = true
  write(brightness, position)
  run(brightness_set, { BRIGHT, tostring(round(position * 100)) })
end

local function brightness_commit()
  local position = brightness_pointer
  write(brightness, position)
  last_brightness = position
  run(brightness_set, { BRIGHT, tostring(round(position * 100)) })
  brightness_interacting = false
  show(brightness_shown, brightness_hide)
end

-- ---------------------------------------------------------------------------
-- Glyphs, from the `volumeIcon` / `brightnessIcon` bindings
-- ---------------------------------------------------------------------------

local function volume_icon()
  if volume_muted:get() then
    return volume_headphone:get() and "󰟎" or "󰖁"
  end
  if volume_headphone:get() then return "󰋋" end
  local level = volume:get()
  if level < 0.33 then return "󰕿" end
  if level < 0.66 then return "󰖀" end
  return "󰕾"
end

local function brightness_icon()
  local level = brightness:get()
  if level < 0.33 then return "󰃞" end
  if level < 0.66 then return "󰃟" end
  return "󰃠"
end

local BATTERY_ICON = "󰂃"

-- ---------------------------------------------------------------------------
-- Pieces
-- ---------------------------------------------------------------------------

--- A glyph centred in a box, which is every piece of text in these panels.
local function glyph(width, height, text, size, color)
  return ui.Text {
    width = width,
    height = height,
    text = text,
    font_family = theme.font,
    font_source = theme.font_source,
    font_size = size,
    color = color,
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
end

--- The circular icon button on the left of a bar OSD.
local function icon_circle(text, fill, on_clicked, enabled)
  local values = {
    x = CIRCLE_X,
    y = CIRCLE_Y,
    width = CIRCLE,
    height = CIRCLE,
    radius = CIRCLE / 2,
    color = fill,
    border_width = BORDER_WIDTH,
    border_color = function() return theme.palette.color0 end,
    behavior = {
      color = { duration = FADE_DURATION, easing = "out_cubic" },
      border_color = { duration = FADE_DURATION, easing = "out_cubic" },
    },
    glyph(CIRCLE, CIRCLE, text, CIRCLE * 0.5, function() return theme.palette.color0 end),
  }
  if on_clicked then
    values[#values + 1] = ui.MouseArea {
      cursor = "pointer",
      anchors = { fill = true },
      -- A hidden OSD must not take the click. `enabled` gates both hit
      -- testing and the derived input region
      -- (`morf-layout/src/layout.rs:140`, `:166`), and unlike `visible` it
      -- leaves the node on screen so the fade-out still renders.
      enabled = enabled,
      on_clicked = on_clicked,
    }
  end
  return ui.Rect(values)
end

--- The split track from StyledProgressBar: past, gap, indicator, gap, future.
---
--- `value` is a function so the whole bar is one binding on one signal, and the
--- 200ms behaviors are the `Behavior on width` / `on x` blocks of the original.
local function progress_bar(value, seek, commit, pointer, enabled)
  local function indicator_position() return BAR_WIDTH * value() end
  return ui.Item {
    x = BAR_X,
    y = BAR_Y,
    width = BAR_WIDTH,
    height = LINE_HEIGHT,

    ui.Rect {
      y = 0,
      height = LINE_HEIGHT,
      width = function() return math.max(0, indicator_position() - INDICATOR_GAP) end,
      radius = TRACK_RADIUS,
      color = function() return theme.palette.color0 end,
      behavior = { width = { duration = FADE_DURATION, easing = "out_cubic" } },
    },

    ui.Rect {
      x = function() return indicator_position() - INDICATOR_WIDTH / 2 end,
      y = INDICATOR_Y,
      width = INDICATOR_WIDTH,
      height = INDICATOR_HEIGHT,
      radius = math.max(0.5, INDICATOR_WIDTH * 0.5),
      color = function() return theme.palette.color0 end,
      behavior = { x = { duration = FADE_DURATION, easing = "out_cubic" } },
    },

    ui.Rect {
      x = function() return indicator_position() + INDICATOR_GAP end,
      y = 0,
      width = function()
        return math.max(0, BAR_WIDTH - indicator_position() - INDICATOR_GAP)
      end,
      height = LINE_HEIGHT,
      radius = TRACK_RADIUS,
      color = function() return theme.palette.color240 end,
      -- Width animates on the same curve as x. The original binds
      -- `width: parent.width - x` to the *animated* x so the right edge stays
      -- pinned to the end of the bar; width is affine in x, so easing both over
      -- the same interval keeps them in agreement for every frame between.
      behavior = {
        x = { duration = FADE_DURATION, easing = "out_cubic" },
        width = { duration = FADE_DURATION, easing = "out_cubic" },
      },
    },

    ui.MouseArea {
      cursor = "ew_resize",
      anchors = { fill = true },
      enabled = enabled,
      -- Press and release carry no coordinates today
      -- (`morf-cli/src/surface_events.rs`, `LayerEvent::PointerButton` goes
      -- through `dispatch_ui_event`, which takes no position), so the position
      -- comes from the last motion. A pointer entering a surface is delivered
      -- as a motion as well (`morf-wayland/src/input_handlers.rs:88-90`), so
      -- the cache is populated before any click can land.
      on_position_changed = function(x) pointer(slider_position(x)) end,
      on_dragged = function(x) seek(pointer(slider_position(x))) end,
      on_pressed = function() seek(pointer()) end,
      on_released = commit,
    },
  }
end

--- The shared skeleton of the volume and brightness panels.
local function bar_panel(shown, icon, fill, on_icon_clicked, bar)
  local enabled = function() return shown:get() end
  return ui.Item {
    x = PANEL_X,
    y = PANEL_Y,
    width = OSD_WIDTH,
    height = OSD_HEIGHT,
    opacity = function() return shown:get() and 1 or 0 end,
    behavior = { opacity = { duration = FADE_DURATION, easing = "out_cubic" } },

    icon_circle(icon, fill, on_icon_clicked, enabled),

    -- The plate the bar sits on, `progressRect` in the original.
    ui.Rect {
      x = PLATE_X,
      y = PLATE_Y,
      width = PLATE_WIDTH,
      height = PLATE_HEIGHT,
      radius = theme.progress_radius(),
      color = function() return theme.palette.color1 end,
      border_width = BORDER_WIDTH,
      border_color = function() return theme.palette.color0 end,
      behavior = {
        color = { duration = FADE_DURATION, easing = "out_cubic" },
        border_color = { duration = FADE_DURATION, easing = "out_cubic" },
      },
    },

    bar(enabled),
  }
end

local function volume_panel()
  return bar_panel(
    volume_shown,
    volume_icon,
    function() return volume_muted:get() and theme.palette.color240 or theme.palette.color1 end,
    function()
      -- Optimistic, as the original is: flip locally and let `pamixer -t`
      -- catch up, so the button does not lag a poll behind the click.
      write(volume_muted, not volume_muted:get())
      run(mute_toggle)
      show(volume_shown, volume_hide)
    end,
    function(enabled)
      return progress_bar(
        function() return volume:get() end,
        volume_seek,
        volume_commit,
        function(position)
          if position then volume_pointer = position end
          return volume_pointer
        end,
        enabled
      )
    end
  )
end

local function brightness_panel()
  return bar_panel(
    brightness_shown,
    brightness_icon,
    function() return theme.palette.color1 end,
    nil,
    function(enabled)
      return progress_bar(
        function() return brightness:get() end,
        brightness_seek,
        brightness_commit,
        function(position)
          if position then brightness_pointer = position end
          return brightness_pointer
        end,
        enabled
      )
    end
  )
end

--- The battery warning: a red disc that pulses while the charge is critical.
---
--- The flash is a property behavior driven by a timer that toggles between the
--- two endpoints, and both live inside a `ui.Loader` keyed on the critical
--- state. That gating is the point. An endless animation never settles, so the
--- compositor is asked for a frame forever; dropping the loader's child removes
--- its timer outright (`morf-lua/src/runtime_helpers.rs:71`), and the shell
--- goes quiet again the moment the battery is no longer critical.
local function battery_panel()
  local circle = BATTERY_SIZE * 0.8
  return ui.Item {
    x = BATTERY_X,
    y = PANEL_Y,
    width = BATTERY_SIZE,
    height = BATTERY_SIZE,
    opacity = function() return battery_shown:get() and 1 or 0 end,
    behavior = { opacity = { duration = FADE_DURATION, easing = "out_cubic" } },

    -- `flashLayer`: the pulse multiplies into the fade above it.
    ui.Item {
      width = BATTERY_SIZE,
      height = BATTERY_SIZE,
      opacity = function() return battery_pulse:get() end,
      behavior = { opacity = { duration = PULSE_DURATION, easing = "in_out_quad" } },

      ui.Rect {
        x = (BATTERY_SIZE - circle) / 2,
        y = (BATTERY_SIZE - circle) / 2,
        width = circle,
        height = circle,
        radius = circle / 2,
        color = WARNING,
        border_width = BORDER_WIDTH,
        border_color = function() return theme.palette.color0 end,
        behavior = { border_color = { duration = FADE_DURATION, easing = "out_cubic" } },
        glyph(circle, circle, BATTERY_ICON, circle * 0.5, "#ffffff"),
      },
    },

    ui.Loader {
      width = 0,
      height = 0,
      active = function() return battery_shown:get() end,
      source = function()
        return ui.Timer {
          interval = PULSE_DURATION,
          ["repeat"] = true,
          running = true,
          on_triggered = function()
            write(battery_pulse, battery_pulse:get() > 0.65 and 0.3 or 1.0)
          end,
        }
      end,
    },
  }
end

-- ---------------------------------------------------------------------------
-- Assembly
-- ---------------------------------------------------------------------------

--- All three OSDs and their timers, as one subtree.
---
--- Every timer lives inside it: morf accepts exactly one primary scene root
--- (`morf-cli/src/surfaces.rs:386`), so a timer parked at the top level would
--- be a second root and fail startup.
function osd.build()
  return ui.Item {
    width = WIDTH,
    height = HEIGHT,

    volume_panel(),
    brightness_panel(),
    battery_panel(),

    -- Moves the in-flight children along and retires OSDs whose hide interval
    -- has run out. Cheap and frequent, so a fading panel is never held up by a
    -- slow child, and a child's output is never left sitting in the pipe.
    ui.Timer {
      interval = TICK_INTERVAL,
      ["repeat"] = true,
      running = true,
      on_triggered = function()
        drain()
        drain_sink_events()
        if sink_changed then
          sink_changed = false
          poll_volume()
        end
        expire(volume_shown, volume_hide, volume_interacting)
        expire(brightness_shown, brightness_hide, brightness_interacting)
      end,
    },

    ui.Timer {
      interval = POLL_INTERVAL,
      ["repeat"] = true,
      running = true,
      on_triggered = function()
        -- Brightness has no event source to subscribe to, but it is two sysfs
        -- reads and no child. The volume is only asked for here when the
        -- subscription is not carrying it.
        if not sink_running then poll_volume() end
        poll_brightness()
      end,
    },

    ui.Timer {
      interval = SINK_INTERVAL,
      ["repeat"] = true,
      running = true,
      -- The headphone check is a shell pipeline, so it runs only when a sink
      -- actually changed — throttled to this interval however chatty the
      -- subscription gets.
      on_triggered = function()
        if sink_ports_changed or not sink_running then
          sink_ports_changed = false
          run(sink_get)
        end
      end,
    },

    ui.Timer {
      interval = BATTERY_INTERVAL,
      ["repeat"] = true,
      running = true,
      on_triggered = poll_battery,
    },
  }
end


-- The battery timer is the one the original marks `triggeredOnStart`, so its
-- first read happens here rather than ten seconds in. Volume and brightness are
-- deliberately left alone: their first poll is what raises the OSD, and the
-- original raises it the same way, 750ms after the shell comes up.
poll_battery()

-- Which sink is active only picks the glyph, never whether an OSD shows, so
-- asking once at load costs nothing and spares the icon three seconds of being
-- wrong. The original waits for its own 3000ms timer.
run(sink_get)

return osd
