-- The mini dashboard: who and what this machine is, a calendar, the
-- weather, and the buttons that end a session.
--
-- The top card carries the profile, uptime, address, traffic and battery,
-- with a bar of lock / do-not-disturb / date / weather / reboot / power
-- along its foot. Under it, a calendar, or the weather when the
-- temperature in the bar is clicked.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local weather = require("weather")
local notify = require("notify")
local proc = require("proc")
local bar = require("bar")

local S = theme.S
local C = theme.color
local state = system.state
local w = weather.state

local dashboard = {}

dashboard.shown = morf.signal("chillpill.dashboard.shown", false)
dashboard.page = morf.signal("chillpill.dashboard.page", "calendar")

local WIDTH = S(700)
local PAD = S(24)
local INNER = WIDTH - PAD * 2

-- ----------------------------------------------------------------- profile --

local function avatar()
  local SIZE = S(76)
  local picture = config.displayPicture
  if picture == "" then
    for _, candidate in ipairs { config.home .. "/.face", config.home .. "/.face.icon" } do
      if system.slurp(candidate) then picture = candidate break end
    end
  end
  return ui.ClipRect {
    width = SIZE, height = SIZE, radius = SIZE / 2,
    ui.Rect { anchors = { fill = true }, color = C.button },
    picture ~= "" and ui.Image { anchors = { fill = true }, source = picture, fill_mode = "preserve_aspect_crop" }
      or theme.icon { text = "󰀄", size = 34, color = C.dim, anchors = { center_in = true } },
  }
end

local function profile()
  return ui.Item {
    width = INNER, height = S(76),
    ui.Row {
      gap = S(20), align = "center", height = S(76),
      avatar(),
      ui.Column {
        gap = S(8),
        ui.Row {
          gap = S(10), align = "end",
          theme.text { text = state.user.name, size = 24, font_weight = 700 },
          theme.text { text = "(" .. state.user.host .. ")", size = 14, color = C.dim },
        },
        theme.text { text = function() return state.uptime end, size = 14, color = C.dim },
      },
    },
    ui.Row {
      gap = S(8), align = "center", height = S(76),
      anchors = { right = true },
      visible = state.battery.present,
      theme.icon { text = bar.battery_glyph, size = 22, color = bar.battery_color },
      theme.text { text = function() return string.format("%d%%", state.battery.percent) end, size = 22, font_weight = 700 },
    },
  }
end

local function address()
  local n = state.network
  return ui.Item {
    width = INNER, height = S(56),
    ui.Column {
      gap = S(6),
      anchors = { left = true, top = true },
      ui.Row {
        gap = S(10), align = "center",
        theme.icon {
          text = function() return n.vpn and "󰦝" or "󰩟" end, size = 16,
          color = function() return n.vpn and C.green or C.blue end,
        },
        theme.text {
          size = 18, font_weight = 700,
          text = function()
            if not config.showSensitiveInfo then return "hidden" end
            local ip = n.vpn and n.vpn_ip ~= "" and n.vpn_ip or n.ip
            return ip ~= "" and ip or "no address"
          end,
        },
      },
      ui.Row {
        gap = S(12), align = "center",
        theme.text {
          size = 13, color = C.dim,
          text = function() return n.vpn and n.vpn_iface ~= "" and n.vpn_iface or n.iface end,
        },
        theme.text {
          size = 13, color = C.dim,
          text = function() return n.vpn and "VPN" or "" end,
          visible = function() return n.vpn end,
        },
      },
    },
    ui.Column {
      gap = S(6),
      anchors = { right = true, top = true },
      ui.Row {
        gap = S(8), align = "center",
        anchors = { right = true },
        theme.icon { text = "", size = 13 },
        theme.text { text = function() return system.bytes(state.traffic.rx) end, size = 18, font_weight = 700 },
      },
      ui.Row {
        gap = S(8), align = "center",
        anchors = { right = true },
        theme.icon { text = "", size = 13, color = C.dim },
        theme.text { text = function() return system.bytes(state.traffic.tx) end, size = 15, color = C.dim },
      },
    },
  }
end

-- ------------------------------------------------------------------- foot --

-- Reboot and power off ask twice: the second click within three seconds
-- is the one that counts.
local armed = morf.signal("chillpill.dashboard.armed", "")
local armed_clock = morf.elapsed_timer()

local function dangerous(name, command)
  return function()
    if armed:get() == name and armed_clock:elapsed_ms() < 3000 then
      armed:set("")
      system.launch(command)
    else
      armed:set(name)
      armed_clock:restart()
      notify.local_notice("Power", name == "reboot" and "Click again to reboot" or "Click again to power off",
        "Within three seconds", "󰐥")
    end
  end
end

local function foot_button(glyph, on_click, color)
  return theme.button {
    width = S(52), height = S(44), radius = 12,
    color = C.button, hover_color = C.hover,
    on_click = on_click,
    theme.icon { text = glyph, size = 16, anchors = { center_in = true }, color = color or C.text },
  }
end

local function foot()
  local date_format = config.clockFormat == "12h" and "%I:%M %p %a, %d %b %Y" or "%H:%M %a, %d %b %Y"
  return theme.box {
    width = INNER, height = S(64), radius = 20, color = C.card,
    ui.Flex {
      direction = "row", align = "center", justify = "space_between",
      anchors = { fill = true, margins = S(10) },
      gap = S(8),
      ui.Row {
        gap = S(8), align = "center",
        foot_button("󰌾", function() system.launch(config.screenLockAppCommand) end),
        foot_button(function() return notify.silent:get() and "󰖔" or "󰽥" end,
          function() notify.silent:set(not notify.silent:get()) end,
          function() return notify.silent:get() and C.blue or C.text end),
      },
      ui.Item {
        layout = { grow = 1, minimum_width = 0 },
        height = S(44),
        theme.text {
          size = 16, anchors = { center_in = true },
          text = function() return bar.clock:format(date_format):gsub("AM", "am"):gsub("PM", "pm") end,
        },
      },
      ui.Row {
        gap = S(8), align = "center",
        ui.Item {
        width = S(96), height = S(44),
        ui.Row {
          gap = S(8), align = "center", height = S(44), anchors = { center_in = true },
          theme.icon { text = function() return w.glyph end, size = 16 },
          theme.text { text = function() return w.temperature ~= "" and w.temperature or "--°" end, size = 15 },
        },
        ui.MouseArea {
          anchors = { fill = true }, cursor = "pointer",
          on_clicked = function()
            dashboard.page:set(dashboard.page:get() == "weather" and "calendar" or "weather")
          end,
        },
      },
        foot_button("󰜉", dangerous("reboot", "systemctl reboot"),
          function() return armed:get() == "reboot" and C.orange or C.text end),
        foot_button("󰐥", dangerous("poweroff", "systemctl poweroff"),
          function() return armed:get() == "poweroff" and C.red or C.text end),
      },
    },
  }
end

-- --------------------------------------------------------------- calendar --

local calendar = morf.state { year = 2026, month = 1, today = 1, today_month = 1, today_year = 2026, picked = "" }
local holidays = {}   -- "YYYY-MM-DD" -> name
local holiday_years = {}

local MONTHS = { "January", "February", "March", "April", "May", "June", "July", "August",
  "September", "October", "November", "December" }

local function days_in(year, month)
  local lengths = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }
  if month == 2 and (year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0)) then return 29 end
  return lengths[month]
end

--- 0 = Sunday.
local function weekday_of(year, month, day)
  local a = math.floor((14 - month) / 12)
  local y = year - a
  local m = month + 12 * a - 2
  return (day + y + math.floor(y / 4) - math.floor(y / 100) + math.floor(y / 400) + math.floor(31 * m / 12)) % 7
end

local function fetch_holidays(year)
  if holiday_years[year] or config.country == "" then return end
  holiday_years[year] = true
  proc.exec({ "curl", "-sL", "--max-time", "15",
    "https://date.nager.at/api/v3/PublicHolidays/" .. year .. "/" .. (config.country or "IN") },
    function(output, success)
      if not success then return end
      local ok, list = pcall(require("morf.io").json.decode, output)
      if not ok or type(list) ~= "table" then return end
      for _, entry in ipairs(list) do
        if type(entry.date) == "string" then holidays[entry.date] = entry.localName or entry.name end
      end
      calendar.picked = calendar.picked  -- nudge readers
    end)
end

local function sync_today()
  local now = bar.clock:snapshot()
  if calendar.today ~= now.day or calendar.today_month ~= now.month then
    calendar.today, calendar.today_month, calendar.today_year = now.day, now.month, now.year
    calendar.year, calendar.month = now.year, now.month
    calendar.picked = ""
  end
  fetch_holidays(now.year)
end
sync_today()
morf.timer(60000, sync_today, true)

local cells = morf.list_model({})

local function fill_cells()
  local rows = {}
  local year, month = calendar.year, calendar.month
  local first = weekday_of(year, month, 1)
  local count = days_in(year, month)
  for blank = 1, first do rows[#rows + 1] = { id = "b" .. blank, day = 0, key = "" } end
  for day = 1, count do
    rows[#rows + 1] = { id = tostring(day), day = day, key = string.format("%04d-%02d-%02d", year, month, day) }
  end
  while #rows % 7 ~= 0 do rows[#rows + 1] = { id = "e" .. #rows, day = 0, key = "" } end
  cells:replace(rows, "id")
end
fill_cells()

local function shift_month(by)
  local month = calendar.month + by
  local year = calendar.year
  if month < 1 then month, year = 12, year - 1 end
  if month > 12 then month, year = 1, year + 1 end
  calendar.month, calendar.year = month, year
  fetch_holidays(year)
  fill_cells()
end

local CELL = S(44)
local CAL_WIDTH = CELL * 7 + S(8) * 6

local function cell(row)
  if row.day == 0 then return ui.Item { width = CELL, height = CELL } end
  local is_today = function()
    return row.day == calendar.today and calendar.month == calendar.today_month and calendar.year == calendar.today_year
  end
  local is_holiday = function() return holidays[row.key] ~= nil end
  return ui.Rect {
    width = CELL, height = CELL, radius = S(10),
    color = function() return calendar.picked == row.key and C.button or morf.color("transparent") end,
    behavior = { color = theme.motion.quick },
    border_width = function() return is_today() and 1 or 0 end,
    border_color = C.faint,
    theme.text {
      text = string.format("%d", row.day), size = 15, anchors = { center_in = true },
      color = function() return (is_today() or is_holiday()) and C.red or C.text end,
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_clicked = function() calendar.picked = calendar.picked == row.key and "" or row.key end,
    },
  }
end

local function calendar_card(shown)
  local function arrow(glyph, by)
    return ui.Item {
      width = S(36), height = S(36),
      theme.icon { text = glyph, size = 16, color = C.dim, anchors = { center_in = true } },
      ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() shift_month(by) end },
    }
  end
  local header = {}
  for _, name in ipairs { "Su", "Mo", "Tu", "We", "Th", "Fr", "Sa" } do
    header[#header + 1] = ui.Item {
      width = CELL, height = S(28),
      theme.text { text = name, size = 13, color = C.dim, anchors = { center_in = true } },
    }
  end
  return theme.padded(theme.reveal(shown, {
    radius = 32,
    pad = S(22),
    from_y = -S(20),
    transform_origin_y = 0,
    ui.Column {
      gap = S(12),
      ui.Item {
        width = CAL_WIDTH, height = S(36),
        ui.Item { anchors = { left = true }, arrow("󰅁", -1) },
        theme.text {
          size = 20, font_weight = 700, anchors = { center_in = true },
          text = function() return MONTHS[calendar.month] .. " " .. calendar.year end,
        },
        ui.Item { anchors = { right = true }, arrow("󰅂", 1) },
      },
      ui.Row { gap = S(8), table.unpack(header) },
      ui.Repeater {
        as = "grid", columns = 7, gap = S(8),
        model = cells,
        delegate = cell,
      },
    },
  }))
end

--- The name of a holiday on the picked day, or today's.
local function holiday_line()
  local key = calendar.picked
  if key == "" then
    key = string.format("%04d-%02d-%02d", calendar.today_year, calendar.today_month, calendar.today)
  end
  return holidays[key] or ""
end

-- ---------------------------------------------------------------- weather --

local function tile(glyph, color, value, label)
  local TILE = math.floor((S(440) - S(16) * 2) / 3)
  return theme.box {
    width = TILE, height = S(120), radius = 18, color = C.card,
    ui.Column {
      gap = S(8), anchors = { center_in = true },
      ui.Item { width = TILE - S(20), height = S(24), theme.icon { text = glyph, size = 20, color = color, anchors = { center_in = true } } },
      ui.Item { width = TILE - S(20), height = S(22), theme.text { text = value, size = 16, font_weight = 700, anchors = { center_in = true } } },
      ui.Item { width = TILE - S(20), height = S(18), theme.text { text = label, size = 13, color = C.dim, anchors = { center_in = true } } },
    },
  }
end

local function day_column(row)
  return ui.Column {
    gap = S(10),
    ui.Item { width = S(90), height = S(18), theme.text { text = row.name, size = 14, color = C.dim, anchors = { center_in = true } } },
    ui.Item { width = S(90), height = S(28), theme.icon { text = row.glyph, size = 22, anchors = { center_in = true } } },
    ui.Item { width = S(90), height = S(18), theme.text { text = row.high .. "/" .. row.low, size = 13, color = C.dim, anchors = { center_in = true } } },
  }
end

local function weather_card(shown)
  local W = S(440)
  return theme.padded(theme.reveal(shown, {
    radius = 32,
    pad = S(24),
    from_y = -S(20),
    transform_origin_y = 0,
    ui.Column {
      gap = S(18),
      ui.Item {
        width = W, height = S(30),
        theme.text { text = function() return w.location end, size = 20, font_weight = 700, anchors = { left = true } },
        ui.Item {
          width = S(30), height = S(30), anchors = { right = true },
          theme.icon { text = "󰑐", size = 16, color = C.dim, anchors = { center_in = true } },
          ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = weather.refresh },
        },
      },
      ui.Row {
        gap = S(24), align = "center",
        theme.icon { text = function() return w.glyph end, size = 56, color = C.dim },
        ui.Column {
          gap = S(6),
          theme.text { text = function() return w.temperature ~= "" and w.temperature or "--" end, size = 44, font_weight = 700 },
          theme.text {
            size = 15, color = C.dim,
            text = function() return w.error ~= "" and w.error or w.description end,
          },
        },
      },
      ui.Row {
        gap = S(16),
        tile("󰔏", C.orange, function() return w.feels end, "Feels"),
        tile("󰖎", C.blue, function() return w.humidity end, "Humidity"),
        tile("󰖝", C.green, function() return w.wind end, "Wind"),
      },
      ui.Rect { width = W, height = 1, color = C.edge },
      ui.Item {
        width = W, height = S(24),
        ui.Row {
          gap = S(8), align = "center", anchors = { left = true },
          theme.icon { text = "󰖜", size = 16, color = C.yellow },
          theme.text { text = function() return w.sunrise end, size = 14 },
        },
        ui.Row {
          gap = S(8), align = "center", anchors = { right = true },
          theme.icon { text = "󰖛", size = 16, color = C.orange },
          theme.text { text = function() return w.sunset end, size = 14 },
        },
      },
      ui.Rect { width = W, height = 1, color = C.edge },
      ui.Repeater { as = "row", gap = S(12), model = w.days, delegate = day_column },
      ui.Item {
        width = W, height = S(18),
        theme.text {
          size = 13, color = C.dim, anchors = { center_in = true },
          text = function() return w.updated ~= "" and ("Updated at " .. w.updated) or "" end,
        },
      },
    },
  }))
end

-- ------------------------------------------------------------------ panel --

function dashboard.build(top)
  local on_calendar = morf.signal("chillpill.dashboard.on_calendar", true)
  local on_weather = morf.signal("chillpill.dashboard.on_weather", false)
  morf.timer(24, function()
    on_calendar:set(dashboard.shown:get() and dashboard.page:get() == "calendar")
    on_weather:set(dashboard.shown:get() and dashboard.page:get() == "weather")
  end, true)
  local calendar_node = calendar_card(on_calendar)
  local weather_node = weather_card(on_weather)
  local top_card = theme.padded {
    width = WIDTH,
    pad = PAD,
    radius = 36,
    ui.Column {
      gap = S(20),
      profile(),
      address(),
      foot(),
    },
  }
  local holiday = theme.box {
    radius = 18,
    visible = function() return dashboard.page:get() == "calendar" and holiday_line() ~= "" end,
    ui.Inset {
      top_margin = S(10), bottom_margin = S(10), left_margin = S(20), right_margin = S(20),
      theme.text { text = holiday_line, size = 15 },
    },
  }
  return ui.Flex {
    direction = "column",
    align = "center",
    gap = S(20),
    anchors = { left = true, right = true, top = true, top_margin = top },
    ui.Item(theme.reveal(dashboard.shown, {
      from_y = -S(28),
      transform_origin_y = 0,
      ui.Flex {
        direction = "column",
        gap = S(20),
        align = "center",
        top_card,
        calendar_node,
        weather_node,
        holiday,
      },
    })),
  }
end

return dashboard
