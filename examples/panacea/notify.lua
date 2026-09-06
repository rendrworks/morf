-- Notifications: the server, and the lists the island shows.
--
-- The island is the notification daemon. What arrives is kept twice: the
-- fresh ones, which the collapsed pill shows as a card for a while, and
-- the history the notifications page lists until it is cleared. Do not
-- disturb keeps the cards away and the history growing.

local morf = require("morf")
local config = require("config")
local theme = require("theme")
local notifications = require("lib.notifications")

local notify = {}

notify.history = morf.list_model({})
notify.popups = morf.list_model({})
notify.count = morf.signal("panacea.notify.count", 0)
notify.silent = morf.signal("panacea.notify.silent", config.notifDnd)

local history = {}
local popups = {}
local seen = {}

local function push_models()
  notify.history:replace(history, "id")
  notify.popups:replace(popups, "id")
  notify.count:set(#history)
end

local function drop_popup(id)
  for index, entry in ipairs(popups) do
    if entry.id == id then
      table.remove(popups, index)
      break
    end
  end
  push_models()
end

local server, why = notifications.serve {
  replace = true,
  default_timeout_ms = config.notifTimeout,
  on_change = function(list)
    for _, entry in ipairs(list) do
      if not seen[entry.id] then
        seen[entry.id] = true
        local row = {
          id = entry.id, app = entry.app, icon = entry.icon,
          summary = entry.summary, body = entry.body, urgency = entry.urgency,
          time = theme.clock:format("%H:%M"), actions = entry.actions,
        }
        table.insert(history, 1, row)
        while #history > 60 do table.remove(history) end
        if not notify.silent:get() or entry.urgency >= 2 then
          table.insert(popups, 1, row)
          while #popups > 3 do table.remove(popups) end
          local linger = entry.urgency >= 2 and config.notifCritTimeout or config.notifTimeout
          if linger > 0 then
            morf.timer(linger, function() drop_popup(row.id) end, false)
          end
        end
      end
    end
    push_models()
  end,
}
notify.server = server
notify.error = why

function notify.dismiss(id)
  for index, entry in ipairs(history) do
    if entry.id == id then
      table.remove(history, index)
      break
    end
  end
  drop_popup(id)
  if server then server.dismiss(id) end
  push_models()
end

function notify.clear()
  local ids = {}
  for _, entry in ipairs(history) do ids[#ids + 1] = entry.id end
  history, popups = {}, {}
  for _, id in ipairs(ids) do
    if server then server.dismiss(id) end
  end
  push_models()
end

function notify.invoke(id, key)
  if server then server.invoke(id, key) end
  notify.dismiss(id)
end

--- One from the shell itself.
function notify.local_notice(app, summary, body, glyph)
  local id = -(#history + 1) - math.floor(morf.elapsed_timer():elapsed_ms())
  local row = {
    id = id, app = app, icon = "", glyph = glyph, summary = summary, body = body or "",
    urgency = 1, time = theme.clock:format("%H:%M"), actions = {},
  }
  table.insert(history, 1, row)
  table.insert(popups, 1, row)
  while #popups > 3 do table.remove(popups) end
  morf.timer(config.notifTimeout, function() drop_popup(id) end, false)
  push_models()
end

--- A glyph for an application, when it sent no icon.
function notify.app_glyph(row)
  if row.glyph then return row.glyph end
  local app = (row.app or ""):lower()
  if app:find("git") then return "󰊢" end
  if app:find("spotify") or app:find("music") then return "󰎆" end
  if app:find("firefox") or app:find("zen") or app:find("brave") or app:find("chrom") then return "󰖟" end
  if app:find("discord") or app:find("telegram") or app:find("signal") or app:find("slack") then return "󰭹" end
  if app:find("mail") or app:find("thunderbird") then return "󰇮" end
  if app:find("screenshot") or app:find("record") then return "󰹑" end
  if app:find("battery") or app:find("power") then return "󰁹" end
  if row.urgency and row.urgency >= 2 then return "󰀦" end
  return "󰂚"
end

return notify
