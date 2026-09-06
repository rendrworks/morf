-- Notifications: the server, the popups at the top, and the list the
-- control center shows.
--
-- The server is `examples/lib/notifications.lua`, which takes the
-- `org.freedesktop.Notifications` name and hands over a list. This keeps two
-- views of it: `popups`, the ones still fresh enough to float under the
-- pill, and `history`, everything since the shell started, newest first,
-- until someone clears it.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local notifications = require("lib.notifications")

local S = theme.S
local C = theme.color

local notify = {}

notify.history = morf.list_model({})
notify.popups = morf.list_model({})
notify.count = morf.signal("chillpill.notify.count", 0)
notify.silent = morf.signal("chillpill.notify.silent", false)

local history = {}
local popups = {}
local seen = {}

local function push_models()
  notify.history:replace(history, "id")
  notify.popups:replace(popups, "id")
  notify.count:set(#history)
end

local function stamp()
  local now = require("bar").clock:format("%H:%M")
  return now
end

--- Removes one popup; the history keeps it.
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
  default_timeout_ms = config.notificationDisplayTime,
  on_change = function(list)
    for _, entry in ipairs(list) do
      if not seen[entry.id] then
        seen[entry.id] = true
        local row = {
          id = entry.id,
          app = entry.app,
          icon = entry.icon,
          summary = entry.summary,
          body = entry.body,
          urgency = entry.urgency,
          time = stamp(),
          actions = entry.actions,
        }
        table.insert(history, 1, row)
        while #history > config.maxNotificationsInStack do table.remove(history) end
        if not notify.silent:get() or entry.urgency >= 2 then
          table.insert(popups, 1, row)
          while #popups > 4 do table.remove(popups) end
          -- Its own timer, independent of the server's: the popup goes
          -- while the entry stays listed until it is cleared.
          local linger = config.notificationDisplayTime
          if entry.urgency >= 2 then linger = linger * 3 end
          morf.timer(linger, function() drop_popup(row.id) end, false)
        end
      end
    end
    push_models()
  end,
}
notify.server = server
notify.error = why

--- Forgets one entry everywhere, and tells the sender.
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

--- Sends one from the shell itself, without going over the bus.
function notify.local_notice(app, summary, body, glyph)
  local id = -(#history + 1) - math.floor(morf.elapsed_timer():elapsed_ms())
  local row = {
    id = id, app = app, icon = "", glyph = glyph, summary = summary, body = body or "",
    urgency = 1, time = stamp(), actions = {},
  }
  table.insert(history, 1, row)
  table.insert(popups, 1, row)
  while #popups > 4 do table.remove(popups) end
  morf.timer(config.notificationDisplayTime, function() drop_popup(id) end, false)
  push_models()
end

-- ------------------------------------------------------------------ glyphs --

--- A Nerd Font glyph for an application, when there is no icon to show.
local function app_glyph(row)
  if row.glyph then return row.glyph end
  local app = (row.app or ""):lower()
  if app:find("git") then return "󰊢" end
  if app:find("spotify") or app:find("music") then return "󰎆" end
  if app:find("firefox") or app:find("zen") or app:find("brave") or app:find("chrom") then return "󰖟" end
  if app:find("discord") or app:find("telegram") or app:find("signal") or app:find("slack") then return "󰭹" end
  if app:find("mail") or app:find("thunderbird") then return "󰇮" end
  if app:find("network") or app:find("bandwidth") then return "󰢾" end
  if app:find("wallpaper") then return "󰸉" end
  if app:find("screenshot") or app:find("grim") then return "󰹑" end
  if app:find("battery") or app:find("power") then return "󰁹" end
  if app:find("timer") then return "󱎫" end
  if app:find("volume") then return "󰕾" end
  return "󰂚"
end
notify.app_glyph = app_glyph

--- The picture on the left of a row: the icon it named, else a glyph.
function notify.badge(row, size)
  local icon = row.icon or ""
  if icon:sub(1, 1) == "/" or icon:sub(1, 7) == "file://" then
    local path = icon:gsub("^file://", "")
    return ui.ClipRect {
      width = size, height = size, radius = S(8),
      ui.Image { anchors = { fill = true }, source = path, fill_mode = "preserve_aspect_crop" },
    }
  elseif icon ~= "" then
    return ui.Icon { name = icon, width = size, height = size }
  end
  return ui.Item {
    width = size, height = size,
    theme.icon { text = app_glyph(row), size = size * 0.8 / theme.scale, anchors = { center_in = true } },
  }
end

-- ------------------------------------------------------------------ popups --

local POPUP_WIDTH = S(560)

--- One floating notification: a badge, a bold summary and its body.
local function popup(row)
  local summary = theme.text { text = row.summary ~= "" and row.summary or row.app, size = 16, font_weight = 700 }
  local body = theme.text {
    text = row.body,
    size = 14,
    color = C.dim,
    width = POPUP_WIDTH - S(120),
    wrap = true,
    max_lines = 3,
    visible = row.body ~= "",
  }
  local node = theme.box {
    radius = 24,
    width = POPUP_WIDTH,
    enter = { opacity = 0, translate_y = -S(60), scale = 0.9 },
    opacity = 1,
    translate_y = 0,
    scale = 1,
    transform_origin_y = 0,
    behavior = { opacity = theme.motion.fade, translate_y = theme.motion.spring, scale = theme.motion.spring },
    ui.Row {
      gap = S(22),
      align = "center",
      ui.Item { width = S(4), height = 1 },
      ui.Item { width = S(28), height = S(28), notify.badge(row, S(28)) },
      ui.Column {
        gap = S(6),
        ui.Item { width = 1, height = S(16) },
        summary,
        body,
        ui.Item { width = 1, height = S(16) },
      },
      ui.Item { width = S(4), height = 1 },
    },
    ui.MouseArea {
      anchors = { fill = true },
      cursor = "pointer",
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

--- The stack of fresh popups under the pill.
function notify.build(top)
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = top },
    ui.Repeater {
      as = "column",
      gap = S(10),
      model = notify.popups,
      delegate = popup,
    },
  }
end

return notify
