-- The weather: the conditions now, three tiles, sunrise and sunset, and
-- three days ahead, from wttr.in for `weatherLocation`.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")

local S = theme.S
local C = theme.color

local page = {}

local weather = nil
if config.weatherLocation ~= "" then
  local ok, module = pcall(require, "weather")
  if ok then weather = module end
end
local w = weather and weather.state

page.title = function() return w and w.location or "Weather" end
page.icon = "󰖐"
page.subtitle = function()
  if not w then return "Set weatherLocation in settings.json" end
  if w.error ~= "" then return w.error end
  if not w.ready then return "Fetching…" end
  return w.description .. (w.updated ~= "" and ("  ·  updated " .. w.updated) or "")
end

function page.on_open()
  if weather then weather.refresh() end
end

function page.build(island)
  local W = theme.page_w()
  if not w then
    return kit.page(W, {
      kit.empty("No location set", function() return true end),
      kit.empty('Add "weatherLocation": "Berlin" to ' .. config.path, function() return true end),
    })
  end
  local third = kit.cell(W, 3)
  local function day(row)
    return kit.row {
      width = W, height = S(44),
      icon = row.glyph, title = row.name, subtitle = nil,
      right = kit.figure(row.high .. " / " .. row.low, S(120)), right_w = S(120),
    }
  end
  return kit.page(W, {
    kit.row {
      width = W, height = S(64),
      icon = function() return w.glyph end,
      title = function() return w.temperature ~= "" and w.temperature or "--" end,
      subtitle = function() return w.description end,
      right = kit.figure(function() return w.sunrise .. "  󰖜   " .. w.sunset .. "  󰖛" end, S(200)), right_w = S(200),
    },
    kit.grid(W, 3, {
      kit.stat(third, "Feels like", function() return w.feels end),
      kit.stat(third, "Humidity", function() return w.humidity end),
      kit.stat(third, "Wind", function() return w.wind end),
    }),
    kit.section("Next days"),
    ui.Repeater { as = "column", gap = kit.GAP, model = w.days, delegate = day },
  })
end

return page
