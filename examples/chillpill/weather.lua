-- The weather, from wttr.in.
--
-- One request every half hour for `weatherLocation`, parsed from the JSON
-- the service offers: the conditions now, sunrise and sunset, and three
-- days of highs and lows. `weatherUnits` picks Celsius or Fahrenheit.

local morf = require("morf")
local io = require("morf.io")
local proc = require("proc")
local config = require("config")

local weather = {}

weather.state = morf.state {
  ready = false,
  error = "",
  location = config.weatherLocation,
  temperature = "",     -- "37°C"
  feels = "",
  humidity = "",
  wind = "",
  description = "",
  glyph = "󰖐",
  sunrise = "",
  sunset = "",
  updated = "",
  days = {},            -- { id, name, glyph, high, low }
}
local state = weather.state

local metric = config.weatherUnits ~= "imperial"
local unit = metric and "°C" or "°F"

-- wttr.in's WWO codes, grouped into a glyph each.
local CODES = {
  [113] = "󰖙", [116] = "󰖕", [119] = "󰖐", [122] = "󰖐",
  [143] = "󰖑", [248] = "󰖑", [260] = "󰖑",
  [176] = "󰖗", [263] = "󰖗", [266] = "󰖗", [293] = "󰖗", [296] = "󰖗",
  [299] = "󰖖", [302] = "󰖖", [305] = "󰖖", [308] = "󰖖", [353] = "󰖗", [356] = "󰖖", [359] = "󰖖",
  [179] = "󰖘", [182] = "󰙿", [185] = "󰙿", [227] = "󰖘", [230] = "󰼶",
  [281] = "󰙿", [284] = "󰙿", [311] = "󰙿", [314] = "󰙿", [317] = "󰙿", [320] = "󰖘",
  [323] = "󰖘", [326] = "󰖘", [329] = "󰼶", [332] = "󰼶", [335] = "󰼶", [338] = "󰼶",
  [350] = "󰖒", [362] = "󰙿", [365] = "󰙿", [368] = "󰖘", [371] = "󰼶", [374] = "󰖒", [377] = "󰖒",
  [200] = "󰙾", [386] = "󰙾", [389] = "󰙾", [392] = "󰙾", [395] = "󰙾",
}

local function glyph_for(code, is_day)
  local code_number = tonumber(code)
  if code_number == 113 and is_day == false then return "󰖔" end
  return CODES[code_number] or "󰖐"
end

--- `05:32 AM` from wttr.in, kept as it comes; `24h` clocks get `05:32`.
local function clock(text)
  if config.clockFormat == "12h" then return text end
  local hour, minute, half = tostring(text):match("(%d+):(%d+)%s*(%a%a)")
  if not hour then return tostring(text) end
  hour = tonumber(hour)
  if half:upper() == "PM" and hour < 12 then hour = hour + 12 end
  if half:upper() == "AM" and hour == 12 then hour = 0 end
  return string.format("%02d:%s", hour, minute)
end

local function parse(text)
  local ok, data = pcall(io.json.decode, text)
  if not ok or type(data) ~= "table" then return false end
  local current = (data.current_condition or {})[1]
  if type(current) ~= "table" then return false end
  local temp = metric and current.temp_C or current.temp_F
  local feels = metric and current.FeelsLikeC or current.FeelsLikeF
  local wind = metric and (current.windspeedKmph .. " km/h") or (current.windspeedMiles .. " mph")
  state.temperature = tostring(temp) .. unit
  state.feels = tostring(feels) .. "°"
  state.humidity = tostring(current.humidity) .. "%"
  state.wind = wind
  local description = ((current.weatherDesc or {})[1] or {}).value or ""
  state.description = description
  local hour = tonumber(require("bar").clock:format("%H")) or 12
  state.glyph = glyph_for(current.weatherCode, hour >= 6 and hour < 19)

  local days = {}
  for index, day in ipairs(data.weather or {}) do
    if index > 3 then break end
    local astronomy = (day.astronomy or {})[1] or {}
    if index == 1 then
      state.sunrise = clock(astronomy.sunrise or "")
      state.sunset = clock(astronomy.sunset or "")
    end
    local midday = (day.hourly or {})[5] or (day.hourly or {})[1] or {}
    local y, m, d = tostring(day.date or ""):match("(%d+)-(%d+)-(%d+)")
    local name = "Day " .. index
    if y then
      -- Zeller, for the weekday of a date without a calendar library.
      y, m, d = tonumber(y), tonumber(m), tonumber(d)
      local a = math.floor((14 - m) / 12)
      local yy = y - a
      local mm = m + 12 * a - 2
      local weekday = (d + yy + math.floor(yy / 4) - math.floor(yy / 100) + math.floor(yy / 400) + math.floor(31 * mm / 12)) % 7
      name = ({ "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat" })[weekday + 1]
    end
    days[#days + 1] = {
      id = day.date or tostring(index),
      name = name,
      glyph = glyph_for(midday.weatherCode, true),
      high = tostring(metric and day.maxtempC or day.maxtempF) .. "°",
      low = tostring(metric and day.mintempC or day.mintempF) .. "°",
    }
  end
  state.days:replace(days, "id")
  local area = ((data.nearest_area or {})[1] or {})
  local city = ((area.areaName or {})[1] or {}).value
  if type(city) == "string" and city ~= "" and config.weatherLocation == "" then state.location = city end
  state.ready = true
  state.error = ""
  return true
end

local fetching = false

function weather.refresh()
  if fetching then return end
  fetching = true
  local location = config.weatherLocation:gsub(" ", "+")
  proc.exec({ "curl", "-sL", "--max-time", "15", "https://wttr.in/" .. location .. "?format=j1" },
    function(output, success)
      fetching = false
      if not success or not parse(output) then
        state.error = "No weather right now"
        return
      end
      state.updated = require("bar").clock:format("%H:%M")
    end)
end

weather.refresh()
morf.timer(30 * 60 * 1000, weather.refresh, true)

return weather
