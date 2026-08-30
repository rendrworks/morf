-- A port of `~/.config/quickshell/board` onto mold primitives.
--
-- The original is a Quickshell `Scope` that loads `board/Board.qml` when the
-- focused workspace goes empty. Board.qml is a single `PanelWindow` holding six
-- cards laid out on a hand-computed grid; each card is a `Rectangle` plus a
-- handful of `Process`, `FileView`, `SystemClock` and `MprisPlayer` readers.
--
-- This file is the same six cards on the same grid, built out of Item, Rect,
-- ClipRect, Text, Image, MouseArea and Timer. Nothing here is a widget the
-- engine knows about: the geometry, the palette, the polling and the parsing
-- are all configuration. What the engine supplies is the scene, the frame
-- clock, the reactive graph, and the file and process services.
--
-- Not ported: the `Watcher.qml` show/hide policy (it is Hyprland workspace
-- state, and belongs to a consumer plugin, not to an example), and the khal
-- calendar service (khal is not installed here, so the original draws no event
-- dots either).

local mold = require("mold")
local core = require("mold.core")
local io = require("mold.io")
local ui = require("mold.ui")
local window = require("mold.window")

local font = "IosevkaTerm Nerd Font Mono"
local font_source = core.shell_path("assets/fonts")
local clock = core.system_clock { precision = "seconds" }
local calendar_clock = core.system_clock { precision = "hours" }

local home = core.env("HOME") or ""

--------------------------------------------------------------------------------
-- Palette
--------------------------------------------------------------------------------

-- `Theme.qml` reads the same pywal file and falls back to the same greys.
local theme = {
  color0 = "#000000",
  color1 = "#ffffff",
  color236 = "#1e1e1e",
  color238 = "#2a2a2a",
  color240 = "#303030",
  color244 = "#555555",
}

local wal = io.file_view {
  path = home .. "/.cache/wal/colors.json",
  preload = true,
}
if wal:loaded() then
  local values = io.json.decode(wal:text())
  local colors = values.colors or {}
  for _, name in ipairs { "color0", "color1", "color236", "color238", "color240", "color244" } do
    theme[name] = colors[name] or theme[name]
  end
end

local function alpha(color, value)
  return color .. string.format("%02x", math.floor(value * 255 + 0.5))
end

local function clamp01(value)
  if value < 0 then return 0 end
  if value > 1 then return 1 end
  return value
end

--------------------------------------------------------------------------------
-- Shared node helpers
--------------------------------------------------------------------------------

local function text(values)
  values.font_family = font
  values.font_source = font_source
  values.font_weight = values.font_weight or 400
  return ui.Text(values)
end

local function card(x, y, width, height, radius, border_width, children)
  local values = {
    x = x,
    y = y,
    width = width,
    height = height,
    radius = radius,
    color = theme.color236,
    border_width = border_width,
    border_color = alpha(theme.color244, 0.08),
  }
  for _, child in ipairs(children or {}) do values[#values + 1] = child end
  return ui.Rect(values)
end

local function centered_label(value, width, height, size, color, weight)
  return text {
    width = width,
    height = height,
    text = value,
    font_size = size,
    font_weight = weight or 400,
    color = color,
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
end

--------------------------------------------------------------------------------
-- Live state
--------------------------------------------------------------------------------

-- Every child process here is a system binary. mold is normally launched
-- through a nixGL-style wrapper that rewrites LD_LIBRARY_PATH to nix store
-- paths; a child that inherits it fails to load its own libraries and exits
-- before printing a line, so the search path is cleared for every spawn.
local CHILD_ENVIRONMENT = { LD_LIBRARY_PATH = "" }

-- `LogoCard.qml` reads BAT0 through `cat`; `UserInfoCard.qml` reads volume
-- through `pamixer` and brightness through `~/.local/sbin/bright`. Battery and
-- brightness are plain sysfs files, so they are read directly and cost no
-- process at all.
local battery = mold.signal("board.battery", 0)         -- percent, 0..100
local brightness = mold.signal("board.brightness", -1)  -- 0..1, negative = unknown
local volume = mold.signal("board.volume", -1)          -- 0..1, negative = unknown
local uptime = mold.signal("board.uptime", "Loading...")

local media_revision = mold.signal("board.media.revision", 0)
local media_position = mold.signal("board.media.position", 0)
local media_hover = mold.signal("board.media.hover", "")

local now = calendar_clock:snapshot()
local calendar_year = mold.signal("board.calendar.year", now.year)
local calendar_month = mold.signal("board.calendar.month", now.month)
local calendar_hover = mold.signal("board.calendar.hover", 0)

local colon_visible = mold.signal("board.clock.colon", true)

--- The first of `paths` that reads back, or nil.
local function first_readable(paths)
  for _, path in ipairs(paths) do
    local view = io.file_view { path = path, preload = true }
    if view:loaded() then return view end
  end
  return nil
end

local battery_file = first_readable {
  "/sys/class/power_supply/BAT0/capacity",
  "/sys/class/power_supply/BAT1/capacity",
  "/sys/class/power_supply/BAT2/capacity",
  "/sys/class/power_supply/battery/capacity",
  "/sys/class/power_supply/macsmc-battery/capacity",
}

-- Both halves of a backlight have to come from the same directory, so the
-- probe pairs them rather than searching for each file on its own.
local backlight_file, backlight_max
for _, directory in ipairs {
  "/sys/class/backlight/intel_backlight",
  "/sys/class/backlight/amdgpu_bl0",
  "/sys/class/backlight/amdgpu_bl1",
  "/sys/class/backlight/acpi_video0",
  "/sys/class/backlight/nvidia_wmi_ec_backlight",
  "/sys/class/backlight/apple-panel-bl",
} do
  if not backlight_file then
    local maximum = io.file_view { path = directory .. "/max_brightness", preload = true }
    local current = io.file_view { path = directory .. "/brightness", preload = true }
    if maximum:loaded() and current:loaded() then
      local value = tonumber((maximum:text() or ""):match("%d+"))
      if value and value > 1 then
        backlight_file = current
        backlight_max = value
      end
    end
  end
end

local uptime_file = io.file_view { path = "/proc/uptime", preload = true }

-- The same script `UserInfoCard.qml` drives, used the same way. Writing sysfs
-- directly would need the caller to be in the `video` group on every machine,
-- and would skip the perceptual curve the script applies.
local bright_command = home .. "/.local/sbin/bright"
local bright_available = io.file_view { path = bright_command, preload = false }:exists()

local function refresh_battery()
  if not battery_file or not battery_file:reload() then return end
  local value = tonumber((battery_file:text() or ""):match("%d+"))
  if value then battery:set(math.max(0, math.min(100, value))) end
end

-- `bright get` reports a perceptual percentage, not the raw register:
-- level = log(raw / 1) / log(max / 1). Reading sysfs without reproducing that
-- curve would put the readout somewhere else than the original's.
local function refresh_brightness()
  if not backlight_file or not backlight_file:reload() then return end
  local raw = tonumber((backlight_file:text() or ""):match("%d+"))
  if not raw then return end
  if raw < 1 then raw = 1 end
  if raw > backlight_max then raw = backlight_max end
  brightness:set(clamp01(math.log(raw) / math.log(backlight_max)))
end

--- Formats seconds the way `uptime -p | sed 's/up //'` does.
local function format_uptime(seconds)
  local minutes = math.floor(seconds / 60)
  local days = math.floor(minutes / 1440)
  local hours = math.floor((minutes % 1440) / 60)
  minutes = minutes % 60
  local parts = {}
  if days > 0 then parts[#parts + 1] = days .. (days == 1 and " day" or " days") end
  if hours > 0 then parts[#parts + 1] = hours .. (hours == 1 and " hour" or " hours") end
  if minutes > 0 or #parts == 0 then
    parts[#parts + 1] = minutes .. (minutes == 1 and " minute" or " minutes")
  end
  return table.concat(parts, ", ")
end

local function refresh_uptime()
  if not uptime_file:reload() then return end
  local seconds = tonumber((uptime_file:text() or ""):match("[%d%.]+"))
  if seconds then uptime:set(format_uptime(seconds)) end
end

--------------------------------------------------------------------------------
-- Process views
--------------------------------------------------------------------------------

-- Views are built once, at load, and reused: reassigning the command is what
-- makes a finished view runnable again. A view is only runnable once its
-- previous run has been drained to `exit`, so every view — including the
-- fire-and-forget ones — is drained on every tick.
local views = {}

local function define_view(key, command)
  views[key] = {
    process = io.process_view { command = command, environment = CHILD_ENVIRONMENT },
    buffer = "",
    busy = false,
    pending = nil,
    handler = nil,
    broken = false,
  }
end

local function start_view(view, command)
  view.process:set_command(command)
  view.buffer = ""
  local started = pcall(function() view.process:start() end)
  if not started then
    -- The binary is missing or not executable. One failure is enough; retrying
    -- it on every tick would spend the frame budget on a spawn that cannot work.
    view.broken = true
    return
  end
  view.busy = true
end

--- Runs `command` on the named view, coalescing to the newest request while
--- an earlier run is still in flight. `Proc.qml` debounces the same way.
local function run(key, command, handler)
  local view = views[key]
  if not view or view.broken then return end
  view.handler = handler
  if view.busy then
    view.pending = command
    return
  end
  start_view(view, command)
end

-- `process_view:next` honours its timeout, and a running child delivers its
-- output while `next` waits, so a purely non-blocking drain never advances.
-- The budget is small and is only spent while a query is actually in flight.
local DRAIN_SLICE_MS = 1
local DRAIN_SLICES = 16

local function drain()
  for _, view in pairs(views) do
    if view.busy then
      for _ = 1, DRAIN_SLICES do
        local event = view.process:next(DRAIN_SLICE_MS)
        if not event then break end
        if event.kind == "stdout" then
          view.buffer = view.buffer .. (event.data or "")
        elseif event.kind == "exit" then
          view.busy = false
          if view.handler then view.handler(view.buffer, event.success) end
          if view.pending then
            local command = view.pending
            view.pending = nil
            start_view(view, command)
          end
          break
        end
      end
    end
  end
end

--------------------------------------------------------------------------------
-- Volume and brightness
--------------------------------------------------------------------------------

define_view("volume_get", { "pamixer", "--get-volume" })
define_view("volume_set", { "pamixer", "--set-volume", "50" })
define_view("bright_set", { bright_command, "50" })

local function refresh_volume()
  run("volume_get", { "pamixer", "--get-volume" }, function(output)
    local value = tonumber(output:match("%d+"))
    if value then volume:set(clamp01(value / 100)) end
  end)
end

local function set_volume(position)
  position = clamp01(position)
  -- The original assigns the new value locally too, so the bar tracks the
  -- pointer instead of waiting for the next read to come back.
  volume:set(position)
  run("volume_set", { "pamixer", "--set-volume", tostring(math.floor(position * 100 + 0.5)) })
end

local function set_brightness(position)
  if not bright_available then return end
  position = clamp01(position)
  brightness:set(position)
  run("bright_set", { bright_command, tostring(math.floor(position * 100 + 0.5)) })
end

--------------------------------------------------------------------------------
-- Media
--------------------------------------------------------------------------------

-- `MprisController.qml` picks the playing player, else the first controllable
-- one. `playerctl -a` reports every player, one record per line, so the same
-- choice is made here over its output.
--
-- Fields are separated by US (0x1f) rather than by a printable character, so a
-- title or artist containing punctuation cannot split a record. A title
-- containing a newline still would; such a record is dropped rather than
-- mis-parsed.
local MEDIA_SEPARATOR = "\31"
local MEDIA_FORMAT = table.concat({
  "{{playerName}}",
  "{{status}}",
  "{{mpris:length}}",
  "{{position}}",
  "{{shuffle}}",
  "{{loop}}",
  "{{volume}}",
  "{{mpris:artUrl}}",
  "{{title}}",
  "{{artist}}",
}, MEDIA_SEPARATOR)

define_view("media_poll", { "playerctl", "-a", "metadata", "--format", MEDIA_FORMAT })
define_view("media_command", { "playerctl", "--version" })

local media = {
  active = false,
  player = "",
  status = "Stopped",
  title = "",
  artist = "",
  length = 0,   -- seconds
  position = 0, -- seconds
  shuffle = false,
  loop = "None",
  volume = 0,
  art = "",
}
local media_seeking = false

--- Splits on a single non-magic byte, keeping empty fields.
local function split(value, separator)
  local fields = {}
  local start = 1
  while true do
    local from, to = value:find(separator, start)
    if not from then
      fields[#fields + 1] = value:sub(start)
      return fields
    end
    fields[#fields + 1] = value:sub(start, from - 1)
    start = to + 1
  end
end

local function apply_media(output, success)
  local chosen
  if success then
    for _, line in ipairs(split(output, "\n")) do
      if line ~= "" then
        local fields = split(line, MEDIA_SEPARATOR)
        if #fields >= 10 then
          local record = {
            player = fields[1],
            status = fields[2],
            length = (tonumber(fields[3]) or 0) / 1000000,
            position = (tonumber(fields[4]) or 0) / 1000000,
            shuffle = fields[5] == "true",
            loop = fields[6] ~= "" and fields[6] or "None",
            volume = clamp01(tonumber(fields[7]) or 0),
            art = fields[8],
            title = fields[9],
            artist = fields[10],
          }
          if not chosen or (record.status == "Playing" and chosen.status ~= "Playing") then
            chosen = record
          end
        end
      end
    end
  end

  local changed = false
  local active = chosen ~= nil
  if media.active ~= active then
    media.active = active
    changed = true
  end
  if chosen then
    for _, key in ipairs { "player", "status", "title", "artist", "length", "shuffle", "loop", "volume", "art" } do
      if media[key] ~= chosen[key] then
        media[key] = chosen[key]
        changed = true
      end
    end
    -- While a drag is in flight the pointer owns the position, exactly as the
    -- original's `isSeeking` guard does.
    if not media_seeking then
      media.position = chosen.position
      media_position:set(chosen.position)
    end
  else
    media.position = 0
    media_position:set(0)
  end
  if changed then media_revision:set(media_revision:get() + 1) end
end

local function refresh_media()
  run("media_poll", { "playerctl", "-a", "metadata", "--format", MEDIA_FORMAT }, apply_media)
end

local function media_command(arguments)
  if not media.active or media.player == "" then return end
  local command = { "playerctl", "-p", media.player }
  for _, value in ipairs(arguments) do command[#command + 1] = value end
  run("media_command", command)
end

local function toggle_playing()
  if not media.active then return end
  media.status = media.status == "Playing" and "Paused" or "Playing"
  media_revision:set(media_revision:get() + 1)
  media_command { "play-pause" }
end

local function previous_track()
  -- `MediaPanel.qml` restarts the track instead of stepping back once it is
  -- more than eight seconds in.
  if media.position > 8 then
    media_command { "position", "0" }
  else
    media_command { "previous" }
  end
end

local function cycle_loop()
  local next_state = "Playlist"
  if media.loop == "Playlist" then
    next_state = "Track"
  elseif media.loop == "Track" then
    next_state = "None"
  end
  media.loop = next_state
  media_revision:set(media_revision:get() + 1)
  media_command { "loop", next_state }
end

local function toggle_shuffle()
  media.shuffle = not media.shuffle
  media_revision:set(media_revision:get() + 1)
  media_command { "shuffle", "Toggle" }
end

local function set_media_volume(position)
  position = clamp01(position)
  media.volume = position
  media_revision:set(media_revision:get() + 1)
  media_command { "volume", string.format("%.3f", position) }
end

local function seek_media(ratio, committed)
  if media.length <= 0 then return end
  media_seeking = not committed
  local seconds = clamp01(ratio) * media.length
  media.position = seconds
  media_position:set(seconds)
  if committed then
    media_command { "position", string.format("%.3f", seconds) }
  end
end

--------------------------------------------------------------------------------
-- The poll tick
--------------------------------------------------------------------------------

local TICK_MS = 250
local tick = 0

local function poll()
  tick = tick + 1
  drain()

  -- Between polls the position is advanced locally, so the seek bar creeps
  -- forward on every tick instead of stepping half a second at a time; the
  -- bar's own 200ms curve smooths the rest. Every poll corrects the estimate.
  -- The original does the same thing by re-reading `position` on a 300ms timer.
  if media.active and media.status == "Playing" and not media_seeking then
    local position = media.position + TICK_MS / 1000
    if media.length > 0 and position > media.length then position = media.length end
    media.position = position
    media_position:set(position)
  end

  -- The two readers that cost a process are started on the first tick rather
  -- than at load, so nothing is forked while the configuration is still being
  -- evaluated.
  if tick == 1 then
    refresh_volume()
    refresh_media()
  end
  if tick % (media.active and 2 or 8) == 0 then refresh_media() end
  if tick % 8 == 0 then refresh_brightness() end
  if tick % 12 == 0 then refresh_volume() end
  if tick % 120 == 0 then refresh_battery() end
  if tick % 240 == 0 then refresh_uptime() end
end

-- The three file-backed readouts are correct on the first frame; the two that
-- need a child process are started from the first tick.
refresh_battery()
refresh_brightness()
refresh_uptime()

--------------------------------------------------------------------------------
-- Calendar arithmetic
--------------------------------------------------------------------------------

local MONTH_NAMES = {
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
}

local function days_in_month(year, month)
  local days = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }
  if month == 2 and year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0) then return 29 end
  return days[month]
end

--- Sakamoto's method, returned Monday-first to match the column headings.
local function weekday_of(year, month, day)
  local offsets = { 0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4 }
  local value = year
  if month < 3 then value = value - 1 end
  local weekday = (value
    + math.floor(value / 4)
    - math.floor(value / 100)
    + math.floor(value / 400)
    + offsets[month]
    + day) % 7
  return weekday == 0 and 7 or weekday
end

local function step_month(delta)
  local month = calendar_month:get() + delta
  local year = calendar_year:get()
  while month < 1 do
    month = month + 12
    year = year - 1
  end
  while month > 12 do
    month = month - 12
    year = year + 1
  end
  calendar_year:set(year)
  calendar_month:set(month)
end

--------------------------------------------------------------------------------
-- Progress bar
--------------------------------------------------------------------------------

-- `StyledProgressBar.qml`: a filled track, a rounded indicator, and an empty
-- track, with a gap punched around the indicator. When `on_seek` is given it
-- also takes the pointer, as the original's `interactive` bars do.
--
-- Pressed and released carry no coordinates, so the pointer position is kept
-- from the motion events: `on_position_changed` while the pointer is over the
-- bar, `on_dragged` once a drag has pulled it off the bar.
local function progress_bar(options)
  local width = options.width
  local line_height = options.line_height
  local value = options.value
  local indicator_width = line_height * 0.3
  local indicator_gap = line_height * 0.8
  local fill_color = options.fill_color or theme.color1

  local function indicator_position()
    return width * clamp01(value())
  end

  local node = {
    x = options.x,
    y = options.y,
    width = width,
    height = line_height * 6,
    ui.Rect {
      y = line_height * 2.5,
      height = line_height,
      width = function() return math.max(0, indicator_position() - indicator_gap) end,
      radius = line_height * 0.2,
      color = fill_color,
      behavior = { width = { duration = 200, easing = "out_cubic" } },
    },
    ui.Rect {
      x = function() return indicator_position() - indicator_width * 0.5 end,
      y = line_height * 1.75,
      width = indicator_width,
      height = line_height * 2.5,
      radius = indicator_width * 0.5,
      color = fill_color,
      behavior = { x = { duration = 200, easing = "out_cubic" } },
    },
    ui.Rect {
      x = function() return indicator_position() + indicator_gap end,
      y = line_height * 2.5,
      width = function() return math.max(0, width - indicator_position() - indicator_gap) end,
      height = line_height,
      radius = line_height * 0.2,
      color = alpha(theme.color244, 0.15),
      behavior = {
        x = { duration = 200, easing = "out_cubic" },
        -- The original derives the empty track's width from its animated `x`,
        -- so the two move together. Here they are separate bindings on the
        -- same value and the width needs its own curve to keep step.
        width = { duration = 200, easing = "out_cubic" },
      },
    },
  }
  if options.visible then node.visible = options.visible end

  if options.on_seek then
    local origin = options.origin_x or 0
    local pointer_x = origin
    local pressed = false
    local function ratio_at(surface_x)
      return clamp01((surface_x - origin) / width)
    end
    node[#node + 1] = ui.MouseArea {
      anchors = { fill = true },
      on_position_changed = function(surface_x)
        pointer_x = surface_x
        if pressed then options.on_seek(ratio_at(surface_x), false) end
      end,
      on_dragged = function(surface_x)
        pointer_x = surface_x
        options.on_seek(ratio_at(surface_x), false)
      end,
      on_pressed = function()
        pressed = true
        options.on_seek(ratio_at(pointer_x), false)
      end,
      on_released = function()
        pressed = false
        options.on_seek(ratio_at(pointer_x), true)
      end,
    }
  end
  return ui.Item(node)
end

--------------------------------------------------------------------------------
-- Cards
--------------------------------------------------------------------------------

local function logo_card(x, y, width, height, radius, border_width, line_height, small_radius)
  local logo_size = math.min(width, height) * 0.8
  local bar_width = width * 0.7
  local content_height = logo_size + line_height * 6
  local content_y = (height - content_height) * 0.5 + height * 0.05
  return card(x, y, width, height, radius, border_width, {
    ui.Image {
      x = (width - logo_size) * 0.5,
      y = content_y,
      width = logo_size,
      height = logo_size,
      source = "file://" .. home .. "/.config/bresilla.svg",
      fill_mode = "preserve_aspect_fit",
    },
    ui.Item {
      x = width * 0.15,
      y = content_y + logo_size,
      width = bar_width,
      height = line_height * 6,
      ui.Rect {
        y = line_height * 2.5,
        height = line_height,
        width = function() return math.max(0, bar_width * battery:get() / 100) end,
        radius = small_radius,
        color = function() return battery:get() > 20 and theme.color1 or "#ff5555" end,
        behavior = { width = { duration = 300, easing = "in_out_quad" } },
      },
      ui.Rect {
        x = function() return bar_width * battery:get() / 100 end,
        y = line_height * 2.5,
        width = function() return math.max(0, bar_width * (1 - battery:get() / 100)) end,
        height = line_height,
        radius = small_radius,
        color = alpha(theme.color244, 0.15),
        behavior = {
          x = { duration = 300, easing = "in_out_quad" },
          width = { duration = 300, easing = "in_out_quad" },
        },
      },
    },
  })
end

local function system_card(x, y, width, height, radius, border_width, font_size)
  return card(x, y, width, height, radius, border_width, {
    centered_label("System", width, height, font_size, theme.color1),
  })
end

local function user_card(x, y, width, height, radius, border_width, line_height)
  local spacing = height * 0.08
  local item_width = (width - spacing * 2) * 0.5 - spacing * 1.5
  local item_height = height - spacing * 2
  local right_x = spacing + item_width + spacing * 1.5
  local title_size = height * 0.24
  local small_size = height * 0.13
  local icon_size = height * 0.34
  local left_height = title_size * 1.2 + small_size * 1.2 + spacing * 0.75
  local left_y = spacing + (item_height - left_height) * 0.5
  local controls_width = item_width * 0.8
  local controls_height = icon_size * 2 + spacing
  local controls_x = right_x + (item_width - controls_width) * 0.5
  local controls_y = spacing + (item_height - controls_height) * 0.5
  local bar_width = controls_width - icon_size - spacing

  --- One circled glyph and, beside it, the bar it controls.
  local function icon_row(row_y, glyph, bar)
    local children = {
      ui.Rect {
        x = controls_x,
        y = row_y,
        width = icon_size,
        height = icon_size,
        radius = icon_size * 0.5,
        color = alpha(theme.color1, 0.15),
        centered_label(glyph, icon_size, icon_size, icon_size * 0.6, theme.color1),
      },
    }
    local bar_x = controls_x + icon_size + spacing
    children[#children + 1] = progress_bar {
      x = bar_x,
      y = row_y + (icon_size - line_height * 6) * 0.5,
      origin_x = x + bar_x,
      width = bar_width,
      line_height = line_height,
      value = bar.value,
      visible = bar.visible,
      on_seek = bar.on_seek,
    }
    return children
  end

  local children = {
    text {
      x = spacing,
      y = left_y,
      width = item_width,
      height = title_size * 1.2,
      text = core.env("USER") or "User",
      font_size = title_size,
      font_weight = 500,
      color = theme.color1,
      elide = "right",
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    text {
      x = spacing,
      y = left_y + title_size * 1.2 + spacing * 0.75,
      width = item_width,
      height = small_size * 1.2,
      text = function() return uptime:get() end,
      font_size = small_size,
      color = alpha(theme.color1, 0.7),
      elide = "right",
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
  }

  local volume_row = icon_row(controls_y, "󰕾", {
    value = function() return volume:get() end,
    visible = function() return volume:get() >= 0 end,
    on_seek = function(ratio) set_volume(ratio) end,
  })
  for _, child in ipairs(volume_row) do children[#children + 1] = child end

  local brightness_row = icon_row(controls_y + icon_size + spacing, "󰃠", {
    value = function() return brightness:get() end,
    visible = function() return brightness:get() >= 0 end,
    on_seek = bright_available and function(ratio) set_brightness(ratio) end or nil,
  })
  for _, child in ipairs(brightness_row) do children[#children + 1] = child end

  return card(x, y, width, height, radius, border_width, children)
end

local function clock_card(x, y, width, height, radius, border_width)
  local digit_size = math.min(width, height) * 0.35
  local digit_width = digit_size * 0.58
  local row_spacing = digit_width * 0.15
  local colon_width = digit_width * 0.4
  local row_width = digit_width * 4 + colon_width + row_spacing * 4
  local date_size = digit_size * 0.375
  local column_spacing = height * 0.02
  local column_height = digit_size * 1.2 + column_spacing + date_size * 1.2
  local column_y = (height - column_height) * 0.5
  local cursor = (width - row_width) * 0.5
  local children = {}

  local function digit(value, node_width)
    children[#children + 1] = text {
      x = cursor,
      y = column_y,
      width = node_width,
      height = digit_size * 1.2,
      text = value,
      font_size = digit_size,
      font_weight = 500,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    }
    cursor = cursor + node_width + row_spacing
  end

  digit(function() return clock:format("%H"):sub(1, 1) end, digit_width)
  digit(function() return clock:format("%H"):sub(2, 2) end, digit_width)
  children[#children + 1] = text {
    x = cursor,
    y = column_y,
    width = colon_width,
    height = digit_size * 1.2,
    text = ":",
    font_size = digit_size,
    font_weight = 500,
    color = theme.color1,
    opacity = function() return colon_visible:get() and 1 or 0 end,
    behavior = { opacity = { duration = 100, easing = "in_out_quad" } },
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
  cursor = cursor + colon_width + row_spacing
  digit(function() return clock:format("%M"):sub(1, 1) end, digit_width)
  digit(function() return clock:format("%M"):sub(2, 2) end, digit_width)
  children[#children + 1] = text {
    y = column_y + digit_size * 1.2 + column_spacing,
    width = width,
    height = date_size * 1.2,
    text = function() return clock:format("%b %d") end,
    font_size = date_size,
    color = alpha(theme.color1, 0.7),
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
  children[#children + 1] = ui.Timer {
    interval = 500,
    ["repeat"] = true,
    running = true,
    on_triggered = function() colon_visible:set(not colon_visible:get()) end,
  }
  return card(x, y, width, height, radius, border_width, children)
end

local function calendar_card(x, y, width, height, radius, border_width)
  local scaled = math.min(width, height)
  local font_size = scaled * 0.04
  local spacing = scaled * 0.02
  local margin = spacing * 2
  local content_width = width - margin * 2
  local content_height = height - margin * 2
  local header_height = font_size * 2
  local weekdays_height = font_size * 1.5
  local weekdays_y = margin + header_height + spacing * 2
  local grid_y = weekdays_y + weekdays_height + spacing * 2
  local grid_height = content_height - header_height - weekdays_height - spacing * 4
  local cell_width = content_width / 7
  local cell_height = grid_height / 5

  --- A month step button: a glyph on a square that lights up under the pointer.
  local function step_button(button_x, glyph, delta, hover_key)
    return ui.Rect {
      x = button_x,
      y = margin,
      width = font_size * 2,
      height = header_height,
      radius = radius,
      color = function()
        return calendar_hover:get() == hover_key and alpha(theme.color1, 0.12) or "transparent"
      end,
      centered_label(glyph, font_size * 2, header_height, font_size, theme.color1),
      ui.MouseArea {
        anchors = { fill = true },
        on_entered = function() calendar_hover:set(hover_key) end,
        on_exited = function()
          if calendar_hover:get() == hover_key then calendar_hover:set(0) end
        end,
        on_clicked = function() step_month(delta) end,
      },
    }
  end

  local children = {
    step_button(margin, "<", -1, -1),
    text {
      x = margin + font_size * 2,
      y = margin,
      width = content_width - font_size * 4,
      height = header_height,
      text = function()
        return MONTH_NAMES[calendar_month:get()] .. " " .. tostring(calendar_year:get())
      end,
      font_size = font_size * 1.3,
      font_weight = 500,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    step_button(margin + content_width - font_size * 2, ">", 1, -2),
  }

  local weekday_names = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }
  for index, name in ipairs(weekday_names) do
    children[#children + 1] = text {
      x = margin + (index - 1) * cell_width,
      y = weekdays_y,
      width = cell_width,
      height = weekdays_height,
      text = name,
      font_size = font_size,
      font_weight = 500,
      color = alpha(theme.color1, 0.6),
      horizontal_alignment = "center",
      vertical_alignment = "center",
    }
  end

  for index = 0, 34 do
    local cell = index + 1
    local column = index % 7
    local row = math.floor(index / 7)
    local circle_size = math.min(cell_width - 4, cell_height - 4)
    local circle_x = margin + column * cell_width + (cell_width - circle_size) * 0.5
    local circle_y = grid_y + row * cell_height + (cell_height - circle_size) * 0.5

    --- The day this cell shows, and whether it belongs to the shown month.
    --- Reads both calendar signals, so every binding built on it follows the
    --- month buttons.
    local function day_of_cell()
      local year = calendar_year:get()
      local month = calendar_month:get()
      local offset = index - weekday_of(year, month, 1) + 2
      local days = days_in_month(year, month)
      if offset < 1 then
        local previous_month = month == 1 and 12 or month - 1
        local previous_year = month == 1 and year - 1 or year
        return days_in_month(previous_year, previous_month) + offset, false
      elseif offset > days then
        return offset - days, false
      end
      return offset, true
    end

    --- Reads the hour-precision clock, so the highlight moves at midnight
    --- instead of being frozen at the value the config was loaded with.
    local function is_today()
      local day, current = day_of_cell()
      if not current then return false end
      local today = calendar_clock:snapshot()
      return today.day == day
        and today.month == calendar_month:get()
        and today.year == calendar_year:get()
    end

    children[#children + 1] = ui.Rect {
      x = circle_x,
      y = circle_y,
      width = circle_size,
      height = circle_size,
      radius = circle_size * 0.5,
      color = function()
        if is_today() then return alpha(theme.color1, 0.12) end
        if calendar_hover:get() == cell then return alpha(theme.color1, 0.08) end
        return "transparent"
      end,
      text {
        width = circle_size,
        height = circle_size,
        text = function() return tostring((day_of_cell())) end,
        font_size = font_size,
        font_weight = function() return is_today() and 500 or 400 end,
        color = function()
          local _, current = day_of_cell()
          if is_today() or current then return theme.color1 end
          return alpha(theme.color1, 0.4)
        end,
        horizontal_alignment = "center",
        vertical_alignment = "center",
      },
      ui.MouseArea {
        anchors = { fill = true },
        on_entered = function() calendar_hover:set(cell) end,
        on_exited = function()
          if calendar_hover:get() == cell then calendar_hover:set(0) end
        end,
      },
    }
  end
  return card(x, y, width, height, radius, border_width, children)
end

local function media_card(x, y, width, height, radius, border_width, line_height)
  local scaled = math.min(width, height)
  local icon_size = scaled * 0.15
  local font_size = scaled * 0.05
  local font_medium = scaled * 0.055
  local spacing = scaled * 0.02
  local button_size = scaled * 0.14

  local function playing() media_revision:get() return media.active end
  local function idle() media_revision:get() return not media.active end

  -- Empty state, unchanged: the icon and label the original shows when
  -- `MprisController.activePlayer` is null.
  local empty_height = icon_size * 1.2 + spacing + font_size * 1.2
  local empty_y = (height - empty_height) * 0.5
  local children = {
    text {
      y = empty_y,
      width = width,
      height = icon_size * 1.2,
      text = "󰝚",
      font_size = icon_size,
      color = alpha(theme.color1, 0.5),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      visible = idle,
    },
    text {
      y = empty_y + icon_size * 1.2 + spacing,
      width = width,
      height = font_size * 1.2,
      text = "No Media",
      font_size = font_size,
      color = alpha(theme.color1, 0.7),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      visible = idle,
    },
  }

  -- Populated state: the five rows of the original's centred column.
  local column_x = spacing
  local column_width = width - spacing * 2
  local column_spacing = spacing * 2
  local volume_height = spacing * 2
  local art_height = height * 0.35
  local title_height = font_medium * 1.2
  local artist_height = font_size * 1.2
  local text_height = title_height + spacing * 0.5 + artist_height
  local seek_height = spacing * 3
  local column_height = volume_height + art_height + text_height + seek_height
    + button_size + column_spacing * 4
  local column_y = (height - column_height) * 0.5

  local volume_y = column_y
  local art_y = volume_y + volume_height + column_spacing
  local text_y = art_y + art_height + column_spacing
  local seek_y = text_y + text_height + column_spacing
  local buttons_y = seek_y + seek_height + column_spacing

  local volume_width = column_width * 0.5
  local volume_x = column_x + (column_width - volume_width) * 0.5
  children[#children + 1] = progress_bar {
    x = volume_x,
    y = volume_y + (volume_height - line_height * 6) * 0.5,
    origin_x = x + volume_x,
    width = volume_width,
    line_height = line_height,
    value = function() media_revision:get() return media.volume end,
    visible = playing,
    on_seek = function(ratio) set_media_volume(ratio) end,
  }

  local art_box = math.min(width * 0.6, art_height)
  local art_size = art_box * 0.9
  local art_x = (width - art_size) * 0.5
  --- Only local art can be shown: the image cache resolves `file://` and
  --- nothing else, so a player advertising an https cover falls back to the
  --- same glyph the empty state uses.
  local function local_art()
    media_revision:get()
    return media.art:sub(1, 7) == "file://"
  end
  children[#children + 1] = ui.ClipRect {
    x = art_x,
    y = art_y + (art_height - art_size) * 0.5,
    width = art_size,
    height = art_size,
    radius = art_size * 0.5,
    color = alpha(theme.color1, 0.08),
    border_width = border_width * 2,
    border_color = theme.color1,
    visible = playing,
    ui.Image {
      width = art_size,
      height = art_size,
      source = function() return local_art() and media.art or "" end,
      fill_mode = "preserve_aspect_crop",
      visible = local_art,
    },
    text {
      width = art_size,
      height = art_size,
      text = "󰝚",
      font_size = art_size * 0.4,
      color = alpha(theme.color1, 0.5),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      visible = function() return not local_art() end,
    },
  }

  children[#children + 1] = text {
    x = column_x,
    y = text_y,
    width = column_width,
    height = title_height,
    text = function()
      media_revision:get()
      return media.title ~= "" and media.title or "Unknown"
    end,
    font_size = font_medium,
    font_weight = 500,
    color = theme.color1,
    elide = "right",
    horizontal_alignment = "center",
    vertical_alignment = "center",
    visible = playing,
  }
  children[#children + 1] = text {
    x = column_x,
    y = text_y + title_height + spacing * 0.5,
    width = column_width,
    height = artist_height,
    text = function()
      media_revision:get()
      return media.artist ~= "" and media.artist or "Unknown Artist"
    end,
    font_size = font_size,
    color = alpha(theme.color1, 0.7),
    elide = "right",
    horizontal_alignment = "center",
    vertical_alignment = "center",
    visible = playing,
  }

  local seek_width = column_width * 0.8
  local seek_x = column_x + (column_width - seek_width) * 0.5
  children[#children + 1] = progress_bar {
    x = seek_x,
    y = seek_y + (seek_height - line_height * 6) * 0.5,
    origin_x = x + seek_x,
    width = seek_width,
    line_height = line_height,
    value = function()
      media_revision:get()
      if media.length <= 0 then return 0 end
      return media_position:get() / media.length
    end,
    visible = playing,
    on_seek = seek_media,
  }

  --- One round transport button. `hover_key` names it in the shared hover
  --- signal, so only the button under the pointer repaints.
  local function transport(button_x, size, glyph, glyph_size, foreground, background, on_clicked, hover_key)
    return ui.Rect {
      x = button_x,
      y = buttons_y + (button_size - size) * 0.5,
      width = size,
      height = size,
      radius = size * 0.5,
      color = background,
      visible = playing,
      text {
        width = size,
        height = size,
        text = glyph,
        font_size = glyph_size,
        color = foreground,
        horizontal_alignment = "center",
        vertical_alignment = "center",
      },
      ui.MouseArea {
        anchors = { fill = true },
        on_entered = function() media_hover:set(hover_key) end,
        on_exited = function()
          if media_hover:get() == hover_key then media_hover:set("") end
        end,
        on_clicked = on_clicked,
      },
    }
  end

  --- The plain buttons are transparent until the pointer is on them.
  local function hover_background(hover_key)
    return function()
      return media_hover:get() == hover_key and theme.color240 or "transparent"
    end
  end

  local small = button_size * 0.6
  local medium = button_size * 0.8
  local row_width = small * 2 + medium * 2 + button_size + spacing * 4
  local cursor = (width - row_width) * 0.5

  children[#children + 1] = transport(cursor, small, "󰒟", button_size * 0.35,
    -- The original tints the glyph with `Theme.primary` when the mode is on
    -- and `Theme.surfaceText` when it is off; both resolve to color1, so only
    -- the pill behind the glyph actually changes.
    theme.color1,
    function()
      media_revision:get()
      if media.shuffle then return alpha(theme.color1, 0.2) end
      return media_hover:get() == "shuffle" and theme.color240 or "transparent"
    end,
    toggle_shuffle, "shuffle")
  cursor = cursor + small + spacing

  children[#children + 1] = transport(cursor, medium, "󰒮", button_size * 0.5,
    theme.color1, hover_background("previous"), previous_track, "previous")
  cursor = cursor + medium + spacing

  children[#children + 1] = transport(cursor, button_size,
    function()
      media_revision:get()
      return media.status == "Playing" and "󰏤" or "󰐊"
    end,
    button_size * 0.6, theme.color0, theme.color1, toggle_playing, "play")
  cursor = cursor + button_size + spacing

  children[#children + 1] = transport(cursor, medium, "󰒭", button_size * 0.5,
    theme.color1, hover_background("next"), function() media_command { "next" } end, "next")
  cursor = cursor + medium + spacing

  children[#children + 1] = transport(cursor, small,
    function()
      media_revision:get()
      return media.loop == "Track" and "󰑘" or "󰑖"
    end,
    button_size * 0.35, theme.color1,
    function()
      media_revision:get()
      if media.loop ~= "None" then return alpha(theme.color1, 0.2) end
      return media_hover:get() == "loop" and theme.color240 or "transparent"
    end,
    cycle_loop, "loop")

  return card(x, y, width, height, radius, border_width, children)
end

--------------------------------------------------------------------------------
-- The board
--------------------------------------------------------------------------------

mold.variants(mold.screens, function(screen)
  local screen_width = screen.width or 1920
  local screen_height = screen.height or 1080
  local short_side = math.min(screen_width, screen_height)
  local width = math.floor(screen_width * 0.576 + 0.5)
  local height = math.floor(screen_height * 0.544 + 0.5)
  local left = math.floor((screen_width - width) / 2 + 0.5)
  local top = math.floor((screen_height - height) / 2 + 0.5)
  local spacing = short_side * 12 / 2160
  local gap = spacing * 0.5
  local inset = gap
  local radius = short_side * 10 / 2160
  local border_width = short_side / 2160
  local line_height = short_side * 10 / 2160
  local small_radius = short_side * 2 / 2160
  local inner_width = width - inset * 2
  local inner_height = height - inset * 2
  local left_width = (inner_width - gap) * 0.22
  local right_width = (inner_width - gap) * 0.78
  local clock_height = (inner_height - gap) * 0.4
  local system_height = (inner_height - gap) * 0.6
  local user_height = (inner_height - gap) * 0.25
  local bottom_height = (inner_height - gap) * 0.75
  local info_width = (right_width - gap) * 0.68
  local logo_width = (right_width - gap) * 0.32
  local calendar_width = (right_width - gap) * 0.56
  local media_width = (right_width - gap) * 0.44
  local right_x = inset + left_width + gap

  mold.surface.namespace = "mold-board"
  mold.surface.width = width
  mold.surface.height = height
  mold.surface.exclusive_zone = 0
  mold.surface.anchors = { top = true, left = true }
  mold.surface.margin_left = left
  mold.surface.margin_top = top
  mold.surface.layer = "top"
  mold.surface.keyboard_focus = "none"
  -- `mask: Region { item: container }` in the original: the whole rounded
  -- board takes the pointer, not just the parts with a MouseArea under them.
  mold.surface.mask = window.region {
    width = width,
    height = height,
    radius = math.floor(radius + 0.5),
  }

  return ui.ClipRect {
    width = width,
    height = height,
    color = theme.color238,
    opacity = 0.98,
    radius = radius,
    logo_card(inset, inset, left_width, clock_height, radius, border_width, line_height, small_radius),
    system_card(
      inset,
      inset + clock_height + gap,
      left_width,
      system_height,
      radius,
      border_width,
      short_side * 28 / 2160
    ),
    user_card(right_x, inset, info_width, user_height, radius, border_width, line_height),
    clock_card(right_x + info_width + gap, inset, logo_width, user_height, radius, border_width),
    calendar_card(
      right_x,
      inset + user_height + gap,
      calendar_width,
      bottom_height,
      radius,
      border_width
    ),
    media_card(
      right_x + calendar_width + gap,
      inset + user_height + gap,
      media_width,
      bottom_height,
      radius,
      border_width,
      line_height
    ),

    -- Everything that reads the outside world runs off this one tick: it
    -- drains whatever the running queries have produced, advances the track
    -- position between polls, and re-runs each reader on its own period.
    ui.Timer {
      interval = TICK_MS,
      ["repeat"] = true,
      running = true,
      on_triggered = poll,
    },
  }
end)
