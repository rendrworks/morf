-- Notifications: the history, do not disturb, clear.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
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
    local node = kit.row {
      width = W,
      icon = notify.app_glyph(entry),
      accent = entry.urgency >= 2 and C.crit or C.on,
      active = function() return entry.urgency >= 2 end,
      tint = C.crit_tint, edge = C.crit:alpha(0.5),
      title = entry.summary ~= "" and entry.summary or entry.app,
      subtitle = (entry.body ~= "" and entry.body or entry.app) .. "  ·  " .. entry.time,
      on_click = function()
        if entry.actions and entry.actions[1] then notify.invoke(entry.id, entry.actions[1].key) end
      end,
      right = kit.cross(function() notify.dismiss(entry.id) end),
    }
    return node, function() end
  end
  return kit.page(W, {
    kit.switch_row {
      width = W, icon = "󰂛", title = "Do not disturb",
      subtitle = function() return notify.silent:get() and "Only urgent ones come through" or "Off" end,
      on = function() return notify.silent:get() end,
      set = function(on) notify.silent:set(on) end,
    },
    kit.section(function() local n = notify.count:get() return n .. (n == 1 and " notification" or " notifications") end, function() return notify.count:get() > 0 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = notify.history, delegate = row },
    kit.empty(function() return notify.silent:get() and "Do not disturb is on" or "Nothing yet" end,
      function() return notify.count:get() == 0 end),
    kit.action { width = W, label = "Clear all", icon = "󰎟", on_click = notify.clear,
      visible = function() return notify.count:get() > 0 end },
  })
end

return page
