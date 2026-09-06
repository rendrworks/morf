-- The shade, on a phone: what the first pull down shows. A few of the
-- tiles as round buttons, the brightness, and the notifications; a
-- further pull opens the whole of quick settings, and a sweep up closes.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local notify = require("notify")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

page.big = true
page.subtitle = function() return theme.clock:format("%A, " .. config.clockDateFmt) end
page.slots = {}

function page.build(island)
  local kit = require("kit")
  local tiles = require("tiles")
  local bar = require("bar_glyphs")
  local W = theme.page_w()

  -- The first few tiles, as round buttons in one row: a tap flips each, a
  -- long look is a pull away.
  local quick = {}
  local names = config.shadeTiles or { "wifi", "bluetooth", "dnd", "sound", "caffeine" }
  for _, name in ipairs(names) do
    local entry = tiles.all[name]
    if entry then
      local on = entry.on or function() return false end
      local D = kit.cell(W, #names)
      quick[#quick + 1] = theme.button {
        width = D, height = kit.ROW, radius = kit.ROW / 2,
        visible = entry.available,
        color = function() return on() and (entry.tint or C.on_tint) or C.card end,
        hover_color = function() return on() and (entry.tint or C.on_tint) or C.card_hover end,
        border_width = 1, border_color = function() return on() and (entry.edge or C.on_edge) or C.edge end,
        behavior = { color = theme.motion.fade, scale = theme.motion.snappy, border_color = theme.motion.fade },
        on_click = entry.toggle or function() island.open(entry.page) end,
        ui.Item {
          width = D, height = kit.ROW,
          kit.glyph(S(config.iconSize), entry.icon, function() return on() and (entry.icon_color or C.on) or C.fg end),
        },
        entry.page and ui.MouseArea {
          anchors = { fill = true }, accepted_buttons = "right",
          on_clicked = function() island.open(entry.page) end,
        } or nil,
      }
    end
  end

  local RECENT = 6
  local recent = morf.list_model({})
  local shown_ids = ""
  theme.tick(function()
    local rows, ids = {}, ""
    for index = 1, math.min(RECENT, notify.history:len()) do
      local entry = notify.history:get(index)
      rows[#rows + 1] = entry
      ids = ids .. tostring(entry.id) .. ","
    end
    if ids ~= shown_ids then
      shown_ids = ids
      recent:replace(rows, "id")
    end
  end)
  local function notice(entry)
    return kit.row {
      width = W,
      icon = notify.app_glyph(entry),
      accent = entry.urgency >= 2 and C.crit or C.on, tint = C.crit_tint, edge = C.crit:alpha(0.5),
      active = function() return entry.urgency >= 2 end,
      title = entry.summary ~= "" and entry.summary or entry.app,
      subtitle = (entry.body ~= "" and entry.body or entry.app) .. "  ·  " .. entry.time,
      on_click = function()
        if entry.actions and entry.actions[1] then notify.invoke(entry.id, entry.actions[1].key) else island.open("notif") end
      end,
      right = kit.cross(function() notify.dismiss(entry.id) end),
    }, function() end
  end

  return kit.page(W, {
    ui.Flex { direction = "row", gap = kit.GAP, width = W, table.unpack(quick) },
    kit.slider_line { width = W, icon = "󰃠",
      fraction = function() return state.brightness.level end, set = system.set_brightness,
      visible = function() return state.brightness.present end },
    kit.section(function() local n = notify.count:get() return n .. (n == 1 and " notification" or " notifications") end,
      function() return notify.count:get() > 0 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = recent, delegate = notice },
    kit.empty("No notifications", function() return notify.count:get() == 0 end),
    ui.Item {
      width = W, height = S(28),
      theme.text { text = "Pull down for all settings  ·  swipe up to close", size = config.fontSize - 5, color = C.faint,
        anchors = { center_in = true } },
    },
  })
end

return page
