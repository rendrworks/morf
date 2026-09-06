-- The media popup: the track that just started, for a moment, under the
-- pill. Art, title, artist, a pause button and the position bar, gone
-- again after `mediaPopupDuration`.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local media = require("media")

local S = theme.S
local C = theme.color
local m = media.state

local mediapopup = {}

mediapopup.shown = morf.signal("chillpill.mediapopup.shown", false)
mediapopup.quiet = morf.signal("chillpill.mediapopup.quiet", false)

local clock = morf.elapsed_timer()
local last_revision = m.revision

--- Called on the shell's tick: shows on a new track, hides after a while.
function mediapopup.tick()
  if m.revision ~= last_revision then
    last_revision = m.revision
    if m.present and m.title ~= "" and not mediapopup.quiet:get() then
      mediapopup.shown:set(true)
      clock:restart()
    end
  end
  if mediapopup.shown:get() and clock:elapsed_ms() >= config.mediaPopupDuration then
    mediapopup.shown:set(false)
  end
end

local WIDTH = S(640)
local HEIGHT = S(150)
local ART = S(90)

function mediapopup.build(top)
  local bar_width = WIDTH - S(40) - ART - S(24) - S(90) - S(120)
  local pill = theme.box {
    width = WIDTH, height = HEIGHT, radius = 36,
    opacity = function() return mediapopup.shown:get() and 1 or 0 end,
    translate_y = function() return mediapopup.shown:get() and 0 or -S(16) end,
    behavior = { opacity = { duration = 160 }, translate_y = { duration = 200, easing = "out_quad" } },
    ui.Row {
      gap = S(24), align = "center", height = HEIGHT,
      anchors = { left = true, left_margin = S(30) },
      ui.ClipRect {
        width = ART, height = ART, radius = S(14),
        color = C.button,
        ui.Image {
          anchors = { fill = true },
          source = function() return m.art end,
          fill_mode = "preserve_aspect_crop",
          visible = function() return m.art ~= "" end,
        },
        theme.icon {
          text = "󰎆", size = 30, color = C.faint, anchors = { center_in = true },
          visible = function() return m.art == "" end,
        },
      },
      ui.Column {
        gap = S(8),
        theme.text {
          text = function() return m.title end, size = 20, font_weight = 700,
          width = WIDTH - S(30) - ART - S(24) - S(110), elide = "right",
        },
        theme.text {
          text = function() return m.artist end, size = 14, color = C.dim,
          width = WIDTH - S(30) - ART - S(24) - S(110), elide = "right",
        },
        ui.Row {
          gap = S(12), align = "center",
          theme.text {
            size = 13, color = C.dim,
            text = function()
              local _ = morf.clock and morf.clock:get()
              return media.clock(media.position())
            end,
          },
          theme.meter {
            width = bar_width, height = S(5),
            fraction = function()
              local _ = morf.clock and morf.clock:get()
              return media.progress()
            end,
          },
          theme.text { size = 13, color = C.dim, text = function() return media.clock(m.length) end },
        },
      },
    },
    theme.button {
      width = S(52), height = S(52), radius = 26,
      color = C.card, hover_color = C.hover,
      border_color = C.edge, border_width = 1,
      anchors = { right = true, right_margin = S(30), top = true, top_margin = (HEIGHT - S(52)) / 2 },
      on_click = media.play_pause,
      theme.icon {
        text = function() return m.status == "Playing" and "󰏤" or "󰐊" end,
        size = 18, anchors = { center_in = true },
      },
    },
  }
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = top },
    visible = function() return mediapopup.shown:get() end,
    pill,
  }
end

return mediapopup
