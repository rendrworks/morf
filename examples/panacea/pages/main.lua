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

local function small_button(glyph, on_click, lit)
  return theme.button {
    width = S(50), height = S(50), radius = 25,
    border_width = 1, border_color = function() return lit and lit() and C.on_edge or C.edge end,
    color = function() return lit and lit() and C.on_tint or C.card end,
    on_click = on_click,
    theme.icon { text = glyph, size = config.iconSize - 2, anchors = { center_in = true },
      color = function() return lit and lit() and C.fg or C.muted end },
  }
end

function page.on_open()
  bars.start()
end

function page.build(island)
  local notify = require("notify")
  local tiles = require("tiles")
  local bar = require("bar_glyphs")
  local m = media.state
  local BTN = S(50)

  -- The system row, as GNOME lays it out: the battery at the left -- the
  -- strip's own glyph and figure land on it -- and the actions at the
  -- right: a screenshot, the notifications, the settings, the lock, power.
  local actions = ui.Row {
    gap = GAP, align = "center",
    small_button("󰹑", function() tiles.all.screenshot.toggle() end),
    small_button(function() return notify.silent:get() and "󰂛" or "󰂚" end, function() island.open("notif") end,
      function() return notify.count:get() > 0 end),
    small_button("󰒓", function() island.open("settings") end),
    small_button("󰌾", function() island.lock() end),
    small_button("󰐥", function() island.open("power") end),
  }
  local battery_w = W - (BTN + GAP) * 5
  local system_row = ui.Item {
    width = W, height = BTN,
    tiles.pill(tiles.all.battery, battery_w, island, page.slots),
    ui.Item { anchors = { right = true, top = true }, actions },
  }

  -- Sliders, the volume and the brightness, each an icon, a track and a
  -- figure: what a phone puts first.
  local function slider_row(glyph, on_glyph, fraction, set, present)
    return ui.Row {
      gap = S(12), align = "center", height = S(28),
      visible = present,
      ui.Item { width = S(6), height = 1 },
      ui.Item {
        width = S(24), height = S(24),
        theme.icon { text = glyph, size = config.iconSize, anchors = { center_in = true } },
        on_glyph and ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = on_glyph } or nil,
      },
      theme.slider {
        width = W - S(6) - S(24) - S(12) * 3 - S(44), height = S(24), track = S(10), knob = S(16), color = C.on,
        fraction = fraction, set = set,
      },
      theme.text { text = function() return math.floor(fraction() * 100 + 0.5) .. "%" end,
        size = config.fontSize - 3, color = C.muted, width = S(44), horizontal_alignment = "right" },
    }
  end
  local sliders = ui.Flex {
    direction = "column", gap = S(6), width = W,
    slider_row(bar.volume_glyph, system.toggle_mute, function() return state.volume.level end, system.set_volume,
      function() return state.volume.available end),
    slider_row("󰃠", nil, function() return state.brightness.level end, system.set_brightness,
      function() return state.brightness.present end),
  }

  -- Now playing, when something is: the card opens the player, and its
  -- equaliser is the seek bar.
  local player = theme.card {
    width = W, height = S(88),
    visible = function() return m.present end,
    ui.MouseArea {
      anchors = { fill = true }, z = -1, cursor = "pointer",
      on_clicked = function() island.open("media") end,
    },
    ui.Row {
      gap = S(12), align = "center", height = S(60),
      anchors = { left = true, left_margin = S(10) },
      ui.ClipRect {
        width = S(44), height = S(44), radius = S(10), color = C.card_hover,
        ui.Image { anchors = { fill = true }, source = function() return m.art end, fill_mode = "preserve_aspect_crop",
          visible = function() return m.art ~= "" end },
        theme.icon { text = "󰎆", anchors = { center_in = true }, color = C.muted, visible = function() return m.art == "" end },
      },
      ui.Column {
        gap = S(2),
        theme.text { text = function() return m.title end, font_weight = 700, width = W - S(220), elide = "right" },
        theme.text { text = function() return m.artist end, size = config.fontSize - 3, color = C.muted, width = W - S(220), elide = "right" },
      },
    },
    ui.Row {
      gap = S(2), align = "center", height = S(60),
      anchors = { right = true, right_margin = S(8) },
      small_button("󰒮", media.previous),
      small_button(function() return m.status == "Playing" and "󰏤" or "󰐊" end, media.play_pause),
      small_button("󰒭", media.next),
    },
    ui.Item {
      anchors = { left = true, bottom = true, left_margin = S(10), bottom_margin = S(6) },
      bars.build {
        width = W - S(20), height = S(18),
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
    local summary = theme.text { text = entry.summary ~= "" and entry.summary or entry.app, font_weight = 700,
      size = config.fontSize - 2, width = W - S(120), elide = "right" }
    local body = theme.text { text = entry.body, size = config.fontSize - 4, color = C.muted,
      width = W - S(120), elide = "right", visible = entry.body ~= "" }
    local node = theme.card {
      width = W, height = S(56),
      enter = { opacity = 0, translate_x = S(24) },
      opacity = 1, translate_x = 0,
      behavior = { opacity = theme.motion.fade, translate_x = theme.motion.move },
      ui.Row {
        gap = S(12), align = "center", height = S(56),
        anchors = { left = true, left_margin = S(12) },
        ui.Rect {
          width = S(32), height = S(32), radius = S(16),
          color = entry.urgency >= 2 and C.crit_tint or C.card_hover,
          theme.icon { text = notify.app_glyph(entry), size = config.iconSize - 2, anchors = { center_in = true },
            color = entry.urgency >= 2 and C.crit or C.fg },
        },
        ui.Column { gap = S(2), summary, body },
      },
      theme.text { text = entry.time, size = config.fontSize - 5, color = C.faint,
        anchors = { right = true, top = true, right_margin = S(40), top_margin = S(10) } },
      ui.Item {
        width = S(28), height = S(28),
        anchors = { right = true, top = true, right_margin = S(8), top_margin = S(14) },
        theme.text { text = "×", size = config.fontSize + 2, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() notify.dismiss(entry.id) end },
      },
      ui.MouseArea {
        anchors = { fill = true }, z = -1, cursor = "pointer",
        on_clicked = function()
          if entry.actions and entry.actions[1] then notify.invoke(entry.id, entry.actions[1].key) else island.open("notif") end
        end,
      },
    }
    return node, function(next)
      summary.text = next.summary ~= "" and next.summary or next.app
      body.text = next.body
      body.visible = next.body ~= ""
    end
  end
  local notices = ui.Flex {
    direction = "column", width = W, gap = S(6),
    visible = function() return notify.count:get() > 0 end,
    ui.Item {
      width = W, height = S(28),
      ui.Row {
        gap = S(8), align = "center", anchors = { left = true, left_margin = S(4) },
        theme.label { text = "Notifications" },
        theme.text { text = function() return tostring(notify.count:get()) end, size = config.fontSize - 5, color = C.faint },
      },
      ui.MouseArea { x = 0, y = 0, width = W - S(80), height = S(28), cursor = "pointer",
        on_clicked = function() island.open("notif") end },
      theme.button {
        height = S(26), radius = 13, color = "transparent",
        anchors = { right = true },
        on_click = notify.clear,
        theme.text { text = "Clear", size = config.fontSize - 4, color = C.muted,
          anchors = { center_in = true } },
        width = S(64),
      },
    },
    ui.Repeater { as = "column", gap = S(6), model = recent, delegate = notice },
  }

  local tray = require("tray")
  return ui.Flex {
    direction = "column",
    width = W,
    gap = S(12),
    system_row,
    sliders,
    tiles.build(island, W, page.slots),
    player,
    notices,
    tray.build(S(config.iconSize + 2)),
  }
end

return page
