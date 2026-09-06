-- Notifications: the history, do not disturb, clear.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local notify = require("notify")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Notifications"
page.icon = "󰂚"
page.subtitle = function()
  local n = notify.count:get()
  if notify.silent:get() then return "Do not disturb is on" end
  return n == 0 and "Nothing yet" or (n .. (n == 1 and " notification" or " notifications"))
end

function page.build(island)
  local W = theme.page_w()
  local function row(entry)
    local summary = theme.text { text = entry.summary ~= "" and entry.summary or entry.app, font_weight = 700,
      size = config.fontSize - 1, width = W - S(110), elide = "right" }
    local meta = theme.text { text = entry.app .. "  " .. entry.time, size = config.fontSize - 5, color = C.muted }
    local body = theme.text { text = entry.body, size = config.fontSize - 3, color = C.muted,
      width = W - S(110), wrap = true, max_lines = 3, visible = entry.body ~= "" }
    local node = theme.card {
      width = W,
      enter = { opacity = 0, translate_x = S(24) },
      opacity = 1, translate_x = 0,
      behavior = { opacity = theme.motion.fade, translate_x = theme.motion.move },
      ui.Inset {
        margin = S(12),
        ui.Row {
          gap = S(12), align = "start",
          ui.Rect {
            width = S(32), height = S(32), radius = S(16),
            color = entry.urgency >= 2 and C.crit_tint or C.card_hover,
            theme.icon { text = notify.app_glyph(entry), size = config.iconSize - 2, anchors = { center_in = true },
              color = entry.urgency >= 2 and C.crit or C.fg },
          },
          ui.Column { gap = S(3), summary, meta, body },
        },
      },
      ui.Item {
        width = S(28), height = S(28),
        anchors = { right = true, top = true, right_margin = S(8), top_margin = S(8) },
        theme.text { text = "×", size = config.fontSize + 2, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() notify.dismiss(entry.id) end },
      },
      ui.MouseArea {
        anchors = { fill = true }, z = -1, cursor = "pointer",
        on_clicked = function()
          if entry.actions and entry.actions[1] then notify.invoke(entry.id, entry.actions[1].key) end
        end,
      },
    }
    return node, function(next)
      summary.text = next.summary ~= "" and next.summary or next.app
      body.text = next.body
      body.visible = next.body ~= ""
    end
  end

  return ui.Flex {
    direction = "column", width = W, gap = S(10),
    theme.switch_row {
      width = W, icon = "󰂛", title = "Do not disturb",
      subtitle = function() return notify.silent:get() and "Only urgent ones come through" or "Off" end,
      on = function() return notify.silent:get() end,
      set = function(on) notify.silent:set(on) end,
    },
    ui.Item {
      width = W, height = S(28),
      visible = function() return notify.count:get() > 0 end,
      theme.label { text = function() return notify.count:get() .. " notifications" end,
        anchors = { left = true, left_margin = S(4), top = true, top_margin = S(8) } },
      theme.button {
        width = S(64), height = S(26), radius = 13, color = "transparent",
        anchors = { right = true },
        on_click = notify.clear,
        theme.text { text = "Clear", size = config.fontSize - 4, color = C.muted, anchors = { center_in = true } },
      },
    },
    ui.Repeater { as = "column", gap = S(6), model = notify.history, delegate = row },
    theme.text {
      size = config.fontSize - 2, color = C.faint,
      text = function() return notify.silent:get() and "Do not disturb is on" or "Nothing yet" end,
      visible = function() return notify.count:get() == 0 end,
    },
  }
end

return page
