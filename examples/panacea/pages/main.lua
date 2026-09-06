-- Quick settings: the clock and date, Wi-Fi, Bluetooth and sound tiles,
-- the recorder and the battery, coffee mode, and the small buttons for the
-- lock, notifications and settings.

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

local W = S(config.panelW) - S(16) * 2
local GAP = S(8)
local TILE_H = S(58)

-- Coffee mode: the idle inhibitor. `morf.idle`? No: a `systemd-inhibit`
-- child that lives while the switch is on.
page.coffee = morf.signal("panacea.coffee", false)
local coffee_process = nil

local function set_coffee(on)
  page.coffee:set(on)
  if on and not coffee_process then
    local io = require("morf.io")
    local ok, process = pcall(io.process_view, {
      command = { "systemd-inhibit", "--what=idle", "--who=panacea", "--why=Coffee mode", "sleep", "infinity" },
      environment = { LD_LIBRARY_PATH = "" },
    })
    if ok then
      pcall(process.start, process)
      coffee_process = process
    end
  elseif not on and coffee_process then
    pcall(coffee_process.kill, coffee_process)
    coffee_process = nil
  end
end

page.slots = {}

--- A tile: a round icon, a title, a line under it, a tint when on. With
--- `slot`, the icon and the title are left empty for a piece of the strip
--- to land on, and the page says where.
local function tile(values)
  local width = values.width
  local on = values.on or function() return false end
  local tint = values.tint or C.on_tint
  local icon_on = values.icon_color or C.on
  return theme.button {
    width = width, height = TILE_H,
    color = function() return on() and tint or C.card end,
    hover_color = function() return on() and tint or C.card_hover end,
    border_width = 1,
    border_color = function() return on() and (values.edge or C.on_edge) or C.edge end,
    on_click = values.on_click,
    ui.Row {
      gap = S(12), align = "center", height = TILE_H,
      anchors = { left = true, left_margin = S(10) },
      ui.Rect {
        width = S(38), height = S(38), radius = S(19),
        color = function() return on() and icon_on or C.card_hover end,
        behavior = { color = theme.motion.hover },
        values.slot and (function()
          local icon_slot = ui.Item { width = S(config.iconSize), height = S(config.iconSize), anchors = { center_in = true } }
          page.slots[values.slot .. "_glyph"] = { node = icon_slot, size = config.iconSize }
          return icon_slot
        end)() or theme.icon { text = values.icon, size = config.iconSize, anchors = { center_in = true },
          color = function() return on() and C.fg or C.muted end },
      },
      ui.Column {
        gap = S(2),
        values.slot and (function()
          local title_slot = ui.Item { width = width - S(72), height = S((config.fontSize - 1) * 1.3) }
          page.slots[values.slot .. "_text"] = { node = title_slot, size = config.fontSize - 1, weight = 700 }
          return title_slot
        end)() or theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1,
          width = width - S(72), elide = "right" },
        theme.text { text = values.subtitle, size = config.fontSize - 4, color = C.muted,
          width = width - S(72), elide = "right" },
      },
    },
  }
end

local function small_button(glyph, on_click, lit)
  return theme.button {
    width = S(44), height = S(44), radius = 12,
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
  local third = math.floor((W - GAP * 2) / 3)
  local half = math.floor((W - GAP) / 2)
  local notify = require("notify")
  local m = media.state

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

  local bar = require("bar_glyphs")
  local tiles1 = ui.Row {
    gap = GAP,
    tile {
      width = third,
      icon = bar.network_glyph,
      title = function()
        if state.network.kind == "none" then return "Wi-Fi" end
        return state.network.name ~= "" and state.network.name or "Wi-Fi"
      end,
      subtitle = function()
        if state.network.kind == "wifi" then return state.network.strength .. "%" end
        if state.network.kind == "wired" then return "Wired" end
        return "Off"
      end,
      on = function() return state.network.kind ~= "none" end,
      on_click = function() island.open("wifi") end,
    },
    tile {
      width = third,
      icon = "󰂯",
      title = "Bluetooth",
      subtitle = function()
        if not state.bluetooth.powered then return "Off" end
        if state.bluetooth.connected ~= "" then return state.bluetooth.connected end
        return "On"
      end,
      on = function() return state.bluetooth.powered end,
      on_click = function() island.open("bt") end,
    },
    theme.card {
      width = third, height = TILE_H,
      border_width = 1, border_color = C.edge,
      ui.Column {
        gap = S(6),
        anchors = { left = true, top = true, margins = S(10) },
        ui.Item {
          width = third - S(20), height = S(16),
          theme.text { text = "Sound", font_weight = 700, size = config.fontSize - 1, anchors = { left = true } },
          theme.text { text = function() return state.volume.muted and "muted" or "" end, size = config.fontSize - 5,
            color = C.muted, anchors = { right = true } },
        },
        ui.Row {
          gap = S(6), align = "center",
          ui.Item {
            width = S(18), height = S(18),
            theme.icon { text = function() return bar.volume_glyph() end, size = config.iconSize - 4, anchors = { center_in = true } },
            ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = system.toggle_mute },
          },
          theme.slider {
            width = third - S(50), height = S(18), track = S(12), knob = S(12), color = C.fg,
            fraction = function() return state.volume.level end,
            set = system.set_volume,
          },
        },
      },
      ui.MouseArea {
        anchors = { fill = true }, z = -1, cursor = "pointer",
        on_clicked = function() island.open("audio") end,
      },
    },
  }

  local record = require("pages.record")
  local tiles2 = ui.Row {
    gap = GAP,
    tile {
      width = half,
      icon = function() return record.state.running and "󰓛" or "󰑊" end,
      icon_color = C.crit,
      tint = C.crit_tint, edge = C.crit:alpha(0.5),
      title = function()
        if record.state.running then return "Recording · " .. record.state.elapsed end
        return "Screen recording"
      end,
      subtitle = function() return record.state.running and record.state.file or "Ready to record" end,
      on = function() return record.state.running end,
      on_click = function() island.open("record") end,
    },
    tile {
      width = half,
      slot = "battery",
      icon_color = C.ok,
      tint = C.ok_tint, edge = C.ok:alpha(0.5),
      title = function() return state.battery.present and (state.battery.percent .. "%") or "Power" end,
      subtitle = function()
        if state.battery.charging then return "Charging" end
        local profile = require("pages.battery").profile:get()
        return profile ~= "" and profile or (state.battery.plugged and "Plugged in" or "On battery")
      end,
      on = function() return state.battery.charging end,
      on_click = function() island.open("battery") end,
    },
  }

  local row3 = ui.Row {
    gap = GAP,
    theme.button {
      width = W - (S(44) + GAP) * 3, height = S(44),
      color = function() return page.coffee:get() and C.coffee_tint or C.card end,
      hover_color = function() return page.coffee:get() and C.coffee_tint or C.card_hover end,
      border_width = 1,
      border_color = function() return page.coffee:get() and C.coffee:alpha(0.6) or C.edge end,
      on_click = function() set_coffee(not page.coffee:get()) end,
      ui.Row {
        gap = S(10), align = "center", height = S(44),
        anchors = { left = true, left_margin = S(12) },
        theme.icon { text = "󰅶", size = config.iconSize - 2, color = function() return page.coffee:get() and C.coffee or C.muted end },
        theme.text { text = "Coffee mode", font_weight = 700, size = config.fontSize - 1 },
      },
      ui.Item {
        anchors = { right = true, top = true, right_margin = S(10), top_margin = S(10) },
        theme.toggle(function() return page.coffee:get() end, set_coffee),
      },
    },
    small_button("󰌾", function() island.lock() end),
    small_button(function() return notify.silent:get() and "󰂛" or "󰂚" end, function() island.open("notif") end,
      function() return notify.count:get() > 0 end),
    small_button("󰒓", function() island.open("settings") end),
  }

  local tray = require("tray")
  return ui.Column {
    gap = S(10),
    player,
    tiles1,
    tiles2,
    row3,
    tray.build(S(config.iconSize + 2)),
  }
end

return page
