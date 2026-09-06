-- Quick settings: the clock and date, now playing, the tile grid from
-- `tiles.lua` in the settings' order, and the small buttons for the lock,
-- notifications and settings.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local media = require("media")
local bars = require("bars")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

-- The clock stays a clock here, large; the day becomes the date.
page.big = true
page.subtitle = function() return theme.clock:format("%A, " .. config.clockDateFmt) end

local W = theme.page_w()
local GAP = S(8)

page.slots = {}

function page.on_open()
  bars.start()
end

function page.build(island)
  local kit = require("kit")
  local notify = require("notify")
  local tiles = require("tiles")
  local bar = require("bar_glyphs")
  local m = media.state
  local BTN = S(50)

  -- The system row: the battery at the left -- the strip's own glyph and
  -- figure land on it -- and the round buttons at the right.
  local actions = ui.Row {
    gap = GAP, align = "center",
    kit.icon_button("󰹑", function() tiles.all.screenshot.toggle() end),
    kit.icon_button(function() return notify.silent:get() and "󰂛" or "󰂚" end, function() island.open("notif") end,
      function() return notify.count:get() > 0 end),
    kit.icon_button("󰒓", function() island.open("settings") end),
    kit.icon_button("󰌾", function() island.lock() end),
    kit.icon_button("󰐥", function() island.open("power") end),
  }
  local system_row = ui.Item {
    width = W, height = kit.ROW,
    tiles.pill(tiles.all.battery, W - (BTN + GAP) * 5, island, page.slots),
    ui.Item { anchors = { right = true, top = true, top_margin = (kit.ROW - BTN) / 2 }, actions },
  }

  -- Now playing, when something is: the card opens the player, and its
  -- equaliser is the seek bar.
  local player = theme.card {
    width = W, height = S(88),
    border_width = 1, border_color = C.edge,
    visible = function() return m.present end,
    ui.MouseArea {
      anchors = { fill = true }, z = -1, cursor = "pointer",
      on_clicked = function() island.open("media") end,
    },
    ui.Row {
      gap = S(12), align = "center", height = S(60),
      anchors = { left = true, left_margin = kit.PAD },
      ui.ClipRect {
        width = kit.CIRCLE, height = kit.CIRCLE, radius = S(10), color = C.card_hover,
        ui.Image { anchors = { fill = true }, source = function() return m.art end, fill_mode = "preserve_aspect_crop",
          visible = function() return m.art ~= "" end },
        theme.icon { text = "󰎆", anchors = { center_in = true }, color = C.muted, visible = function() return m.art == "" end },
      },
      ui.Column {
        gap = S(2),
        theme.text { text = function() return m.title end, font_weight = 700, size = config.fontSize - 1, width = W - S(230), elide = "right" },
        theme.text { text = function() return m.artist end, size = config.fontSize - 4, color = C.muted, width = W - S(230), elide = "right" },
      },
    },
    ui.Row {
      gap = S(2), align = "center", height = S(60),
      anchors = { right = true, right_margin = S(8) },
      kit.icon_button("󰒮", media.previous),
      kit.icon_button(function() return m.status == "Playing" and "󰏤" or "󰐊" end, media.play_pause),
      kit.icon_button("󰒭", media.next),
    },
    ui.Item {
      anchors = { left = true, bottom = true, left_margin = kit.PAD, bottom_margin = S(6) },
      bars.build {
        width = W - kit.PAD * 2, height = S(16),
        playing = function() return m.status == "Playing" end,
        progress = function()
          local _ = morf.clock and morf.clock:get()
          return media.progress()
        end,
        seek = media.seek,
      },
    },
  }

  -- The latest notifications under the toggles, as the shade has them: a
  -- few, compact; the rest are a tap away on their page.
  local RECENT = 4
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

  local tray = require("tray")
  return kit.page(W, {
    system_row,
    kit.slider_line { width = W, icon = bar.volume_glyph, on_icon = system.toggle_mute,
      fraction = function() return state.volume.level end, set = system.set_volume,
      visible = function() return state.volume.available end },
    kit.slider_line { width = W, icon = "󰃠",
      fraction = function() return state.brightness.level end, set = system.set_brightness,
      visible = function() return state.brightness.present end },
    tiles.build(island, W, page.slots),
    player,
    kit.section(function() local n = notify.count:get() return n .. (n == 1 and " notification" or " notifications") end, function() return notify.count:get() > 0 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = recent, delegate = notice },
    tray.build(S(config.iconSize + 2)),
  })
end

return page
