-- Now playing: the art, the track, the transport, and the equaliser, which
-- is also the seek bar.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local media = require("media")
local bars = require("bars")

local S = theme.S
local C = theme.color
local m = media.state

local page = {}

page.title = function() return m.title ~= "" and m.title or "Nothing playing" end
page.icon = "󰎆"
page.subtitle = function()
  if not m.present then return "Start something and it shows here" end
  return m.artist ~= "" and m.artist or m.player:gsub("^org%.mpris%.MediaPlayer2%.", "")
end

function page.on_open()
  bars.start()
  media.poll()
end

function page.on_key(keysym)
  if keysym == 0x20 then media.play_pause() return true end
  if keysym == 0xff53 then media.next() return true end
  if keysym == 0xff51 then media.previous() return true end
  return false
end

local function transport(glyph, size, on_click, big)
  local D = big and S(52) or S(40)
  return theme.button {
    width = D, height = D, radius = D / 2,
    color = big and C.on_tint or C.card,
    on_click = on_click,
    theme.icon { text = glyph, size = size, anchors = { center_in = true } },
  }
end

function page.build(island)
  local W = theme.page_w()
  local ART = S(120)
  return ui.Column {
    gap = S(14),
    ui.Row {
      gap = S(16), align = "center",
      ui.ClipRect {
        width = ART, height = ART, radius = S(16), color = C.card,
        ui.Image { anchors = { fill = true }, source = function() return m.art end, fill_mode = "preserve_aspect_crop",
          visible = function() return m.art ~= "" end },
        theme.icon { text = "󰎆", size = 40, color = C.muted, anchors = { center_in = true },
          visible = function() return m.art == "" end },
      },
      ui.Column {
        gap = S(8),
        theme.text { text = function() return m.album ~= "" and m.album or " " end, size = config.fontSize - 3, color = C.muted,
          width = W - ART - S(16), elide = "right" },
        ui.Row {
          gap = S(10), align = "center",
          transport("󰒮", config.iconSize, media.previous),
          transport(function() return m.status == "Playing" and "󰏤" or "󰐊" end, config.iconSize + 4, media.play_pause, true),
          transport("󰒭", config.iconSize, media.next),
        },
        ui.Row {
          gap = S(8), align = "center",
          theme.text { size = config.fontSize - 4, color = C.muted,
            text = function()
              local _ = morf.clock and morf.clock:get()
              return media.clock(media.position())
            end },
          theme.text { text = "/", size = config.fontSize - 4, color = C.faint },
          theme.text { text = function() return media.clock(m.length) end, size = config.fontSize - 4, color = C.muted },
          theme.text { size = config.fontSize - 5, color = C.faint,
            text = function()
              local source = bars.source:get()
              return source == "" and "no equaliser source" or ("equaliser: " .. source)
            end },
        },
      },
    },
    -- The equaliser is the seek bar: click or drag along it.
    bars.build {
      width = W, height = S(70),
      playing = function() return m.status == "Playing" end,
      progress = function()
        local _ = morf.clock and morf.clock:get()
        return media.progress()
      end,
      seek = media.seek,
    },
  }
end

return page
