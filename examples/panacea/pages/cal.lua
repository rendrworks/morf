-- The calendar: a month, today marked, and the weather beside the date
-- when a location is set.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

local S = theme.S
local C = theme.color

local page = {}

local calendar = morf.state { year = 2026, month = 1, today = 1, today_month = 1, today_year = 2026 }
local MONTHS = { "January", "February", "March", "April", "May", "June", "July", "August",
  "September", "October", "November", "December" }

local function days_in(year, month)
  local lengths = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }
  if month == 2 and (year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0)) then return 29 end
  return lengths[month]
end

--- 0 is Monday: the week starts there, as Panacea's does.
local function weekday_of(year, month, day)
  local a = math.floor((14 - month) / 12)
  local y = year - a
  local m = month + 12 * a - 2
  local sunday_first = (day + y + math.floor(y / 4) - math.floor(y / 100) + math.floor(y / 400) + math.floor(31 * m / 12)) % 7
  return (sunday_first + 6) % 7
end

local cells = morf.list_model({})

local function fill()
  local rows = {}
  local year, month = calendar.year, calendar.month
  for blank = 1, weekday_of(year, month, 1) do rows[#rows + 1] = { id = "b" .. blank, day = 0 } end
  for day = 1, days_in(year, month) do rows[#rows + 1] = { id = tostring(day), day = day } end
  while #rows % 7 ~= 0 do rows[#rows + 1] = { id = "e" .. #rows, day = 0 } end
  cells:replace(rows, "id")
end

local function sync()
  local now = theme.clock:snapshot()
  if calendar.today ~= now.day or calendar.today_month ~= now.month then
    calendar.today, calendar.today_month, calendar.today_year = now.day, now.month, now.year
    calendar.year, calendar.month = now.year, now.month
    fill()
  end
end
sync()
morf.timer(60000, sync, true)

local slide = morf.signal("panacea.cal.slide", 0)
local function shift(by)
  slide:set(by * S(40))
  morf.timer(16, function() slide:set(0) end, false)
  local month, year = calendar.month + by, calendar.year
  if month < 1 then month, year = 12, year - 1 end
  if month > 12 then month, year = 1, year + 1 end
  calendar.month, calendar.year = month, year
  fill()
end

function page.on_open()
  calendar.year, calendar.month = calendar.today_year, calendar.today_month
  fill()
end

function page.on_key(keysym)
  if keysym == 0xff51 then shift(-1) return true end
  if keysym == 0xff53 then shift(1) return true end
  return false
end

function page.build(island)
  local W = S(config.panelW) - S(32)
  local CELL = math.floor((W - S(6) * 6) / 7)
  local function cell(row)
    if row.day == 0 then return ui.Item { width = CELL, height = S(34) } end
    local today = function()
      return row.day == calendar.today and calendar.month == calendar.today_month and calendar.year == calendar.today_year
    end
    return ui.Rect {
      width = CELL, height = S(34), radius = S(10),
      color = function() return today() and C.on or morf.color("transparent") end,
      theme.text {
        text = string.format("%d", row.day), size = config.fontSize - 2, anchors = { center_in = true },
        color = function() return today() and C.fg or C.muted end,
        font_weight = function() return today() and 700 or 400 end,
      },
    }
  end
  local header = {}
  for _, name in ipairs { "Mo", "Tu", "We", "Th", "Fr", "Sa", "Su" } do
    header[#header + 1] = ui.Item {
      width = CELL, height = S(20),
      theme.text { text = name, size = config.fontSize - 5, color = C.faint, anchors = { center_in = true } },
    }
  end
  local weather = nil
  if config.weatherLocation ~= "" then
    local ok, module = pcall(require, "weather")
    if ok then weather = module.state end
  end
  return ui.Column {
    gap = S(10),
    ui.Item {
      width = W, height = S(44),
      ui.Column {
        gap = S(2), anchors = { left = true, left_margin = S(4) },
        theme.text { text = function() return theme.clock:format("%A") end, font_weight = 700, size = config.fontSize + 1 },
        theme.text { text = function() return theme.clock:format(config.clockDateFmt .. " %Y") end, size = config.fontSize - 3, color = C.muted },
      },
      weather and ui.Row {
        gap = S(8), align = "center", anchors = { right = true, top = true, top_margin = S(6) },
        theme.icon { text = function() return weather.glyph end, size = config.iconSize },
        theme.text { text = function() return weather.temperature ~= "" and weather.temperature or "--" end, font_weight = 700 },
        theme.text { text = function() return weather.description end, size = config.fontSize - 4, color = C.muted },
      } or nil,
    },
    ui.Item {
      width = W, height = S(30),
      ui.Item {
        width = S(30), height = S(30), anchors = { left = true },
        theme.icon { text = "󰅁", size = config.iconSize - 2, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() shift(-1) end },
      },
      theme.text {
        text = function() return MONTHS[calendar.month] .. " " .. calendar.year end,
        font_weight = 700, anchors = { center_in = true },
      },
      ui.Item {
        width = S(30), height = S(30), anchors = { right = true },
        theme.icon { text = "󰅂", size = config.iconSize - 2, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() shift(1) end },
      },
    },
    ui.Row { gap = S(6), table.unpack(header) },
    ui.Item {
      width = W,
      translate_x = function() return slide:get() end,
      opacity = function() return slide:get() == 0 and 1 or 0 end,
      behavior = { translate_x = theme.motion.move, opacity = theme.motion.fade },
      ui.Repeater { as = "grid", columns = 7, gap = S(6), model = cells, delegate = cell },
    },
  }
end

return page
