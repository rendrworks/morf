-- The weather: the conditions now, three tiles, sunrise and sunset, and
-- three days ahead, from wttr.in for `weatherLocation`.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

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
  local W = S(config.panelW) - S(32)
  if not w then
    return ui.Column {
      gap = S(8),
      theme.text { text = "No location set.", size = config.fontSize - 1 },
      theme.text { text = 'Add "weatherLocation": "Berlin" to ' .. config.path, size = config.fontSize - 4, color = C.muted,
        width = W, wrap = true, max_lines = 2 },
    }
  end
  local third = math.floor((W - S(8) * 2) / 3)
  local function tile(glyph, color, value, label)
    return theme.card {
      width = third, height = S(78),
      ui.Column {
        gap = S(4), anchors = { center_in = true },
        ui.Item { width = third - S(20), height = S(20), theme.icon { text = glyph, size = config.iconSize, color = color, anchors = { center_in = true } } },
        ui.Item { width = third - S(20), height = S(18), theme.text { text = value, font_weight = 700, size = config.fontSize - 1, anchors = { center_in = true } } },
        ui.Item { width = third - S(20), height = S(14), theme.text { text = label, size = config.fontSize - 5, color = C.muted, anchors = { center_in = true } } },
      },
    }
  end
  local function day(row)
    return ui.Column {
      gap = S(6),
      ui.Item { width = S(70), height = S(16), theme.text { text = row.name, size = config.fontSize - 4, color = C.muted, anchors = { center_in = true } } },
      ui.Item { width = S(70), height = S(24), theme.icon { text = row.glyph, size = config.iconSize + 2, anchors = { center_in = true } } },
      ui.Item { width = S(70), height = S(16), theme.text { text = row.high .. "/" .. row.low, size = config.fontSize - 4, color = C.muted, anchors = { center_in = true } } },
    }
  end
  return ui.Column {
    gap = S(12),
    ui.Row {
      gap = S(20), align = "center",
      theme.icon { text = function() return w.glyph end, size = 48, color = C.muted },
      theme.text { text = function() return w.temperature ~= "" and w.temperature or "--" end, size = 40, font_weight = 700 },
    },
    ui.Row {
      gap = S(8),
      tile("󰔏", C.warn, function() return w.feels end, "Feels like"),
      tile("󰖎", C.on, function() return w.humidity end, "Humidity"),
      tile("󰖝", C.ok, function() return w.wind end, "Wind"),
    },
    ui.Item {
      width = W, height = S(22),
      ui.Row { gap = S(8), align = "center", anchors = { left = true },
        theme.icon { text = "󰖜", size = config.iconSize - 2, color = C.coffee },
        theme.text { text = function() return w.sunrise end, size = config.fontSize - 2 } },
      ui.Row { gap = S(8), align = "center", anchors = { right = true },
        theme.icon { text = "󰖛", size = config.iconSize - 2, color = C.warn },
        theme.text { text = function() return w.sunset end, size = config.fontSize - 2 } },
    },
    ui.Rect { width = W, height = 1, color = C.edge },
    ui.Repeater { as = "row", gap = S(16), model = w.days, delegate = day },
  }
end

return page
