-- Notifications, drawn by the shell that receives them.
--
-- The server is `examples/lib/notifications.lua`; this is what a shell does
-- with what it hands over. Run it, then `notify-send "hello" "there"` -- or
-- anything else on the desktop that notifies -- and it appears here. Clicking
-- an entry dismisses it, which tells the sender so.

local morf = require("morf")
local ui = require("morf.ui")
local notifications = require("lib.notifications")

morf.surface.height = 44
morf.surface.layer = "overlay"

-- A list model rather than a signal: a signal holds one value, and a list of
-- notifications is many. `replace` reconciles by id, so a notification that
-- updated in place keeps its row and a dismissed one leaves.
local shown = morf.list_model({})
local count = morf.signal("notifications.count", 0)

-- Not replacing: a demo that stole the name from the desktop's real daemon
-- would take every notification with it. A shell meant to *be* the daemon
-- leaves `replace` at its default.
local server, why = notifications.serve {
  replace = false,
  default_timeout_ms = 8000,
  on_change = function(list)
    local rows = {}
    for index, entry in ipairs(list) do
      rows[index] = {
        id = entry.id, app = entry.app, summary = entry.summary,
        body = entry.body, urgency = entry.urgency,
      }
    end
    shown:replace(rows, "id")
    count:set(#list)
  end,
}
local status = morf.signal("notifications.status", server and "" or tostring(why))
server = server or { dismiss = function() end }

-- One line per notification, newest first, and a count so the bar says
-- something when there are none.
ui.Rect {
  color = "#101418",
  ui.Row {
    gap = 18,
    anchors = { left = true, margins = 12 },
    ui.Text {
      color = "#8a94a0",
      font_size = 16,
      text = function()
        if status:get() ~= "" then return status:get() end
        local n = count:get()
        return n == 0 and "no notifications" or (n .. " notification" .. (n == 1 and "" or "s"))
      end,
    },
    ui.Repeater {
      model = shown,
      delegate = function(entry)
        return ui.MouseArea {
          on_clicked = function() server.dismiss(entry.id) end,
          cursor = "pointer",
          -- A card arrives from the right and fades in: `enter` is where its
          -- first frame starts, and the behaviors carry it to where it sits.
          enter = { opacity = 0, translate_x = 32 },
          behavior = {
            opacity = { duration = 220, easing = "out_cubic" },
            translate_x = { kind = "spring", stiffness = 260, damping = 22 },
          },
          ui.Rect {
            color = entry.urgency == 2 and "#5a1e1e" or "#1e2a36",
            radius = 8,
            ui.Inset {
              margin = 6,
              ui.Row {
                gap = 8,
                ui.Text { text = entry.app, color = "#8a94a0", font_size = 14, font_style = "italic" },
                ui.Text { text = entry.summary, color = "#ffffff", font_size = 16 },
                ui.Text { text = entry.body, color = "#c0c8d0", font_size = 14, line_height = 1.5 },
              },
            },
          },
        }
      end,
    },
  },
}
