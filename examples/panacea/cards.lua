-- Notification cards: fresh notifications, each a small capsule under the
-- collapsed pill, gone after the timeout. A click opens the first action
-- the sender offered, or dismisses.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local notify = require("notify")

local S = theme.S
local C = theme.color

local cards = {}

local WIDTH = S(config.panelW * 0.8)

local function card(row)
  local summary = theme.text { text = row.summary ~= "" and row.summary or row.app, font_weight = 700,
    width = WIDTH - S(90), elide = "right" }
  local body = theme.text {
    text = row.body, size = config.fontSize - 2, color = C.muted,
    width = WIDTH - S(90), wrap = true, max_lines = 3,
    visible = row.body ~= "",
  }
  local node = ui.Rect {
    color = C.bg,
    radius = S(config.cornerR),
    width = WIDTH,
    enter = { opacity = 0, translate_y = -S(30), scale = 0.92 },
    opacity = 1, translate_y = 0, scale = 1,
    transform_origin_y = 0,
    behavior = { opacity = theme.motion.fade, translate_y = theme.motion.move, scale = theme.motion.move },
    ui.Row {
      gap = S(14), align = "center",
      ui.Item { width = S(2), height = 1 },
      ui.Rect {
        width = S(36), height = S(36), radius = S(18),
        color = row.urgency >= 2 and C.crit_tint or C.card,
        theme.icon { text = notify.app_glyph(row), size = config.iconSize - 1, anchors = { center_in = true },
          color = row.urgency >= 2 and C.crit or C.fg },
      },
      ui.Column {
        gap = S(4),
        ui.Item { width = 1, height = S(14) },
        summary,
        body,
        ui.Item { width = 1, height = S(14) },
      },
      ui.Item { width = S(2), height = 1 },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_clicked = function()
        if row.actions and row.actions[1] then
          notify.invoke(row.id, row.actions[1].key)
        else
          notify.dismiss(row.id)
        end
      end,
    },
  }
  return node, function(next)
    summary.text = next.summary ~= "" and next.summary or next.app
    body.text = next.body
    body.visible = next.body ~= ""
  end
end

function cards.build()
  local island = require("island")
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = S(config.pillH) + S(10) },
    -- Under the strip only; a page has the room.
    visible = function() return island.page:get() == "" end,
    ui.Repeater {
      as = "column",
      gap = S(8),
      model = notify.popups,
      delegate = card,
    },
  }
end

return cards
