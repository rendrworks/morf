-- The tray: every status notifier item on the bus, as an icon that
-- activates on a click and opens its menu on a right click.
--
-- The host is the engine's `morf.status_notifier`; the watcher the items
-- register with is served from Lua when nobody else runs one.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

local S = theme.S
local C = theme.color

local tray = {}

tray.items = morf.list_model({})
tray.count = morf.signal("panacea.tray.count", 0)

local watcher_ok, tray_watcher = pcall(require, "lib.tray_watcher")
if watcher_ok then
  local served = tray_watcher.serve and tray_watcher.serve()
  tray.watcher = served
end

local menus_ok, dbusmenu = pcall(require, "lib.dbusmenu")

local subscribed = pcall(morf.status_notifier.subscribe, function(items)
  local rows = {}
  for _, item in ipairs(items) do
    local properties = item.properties or {}
    local address = item.service .. item.path
    rows[#rows + 1] = {
      id = address,
      service = item.service,
      path = item.path,
      icon = properties.IconName or "",
      title = properties.Title or properties.Id or "",
      status = properties.Status or "",
    }
  end
  tray.items:replace(rows, "id")
  tray.count:set(#rows)
end)
tray.available = subscribed

--- Sends the item a click, which is what its left click means.
local function activate(row)
  local ok, proxy = pcall(morf.dbus.proxy, "session", row.service, row.path, "org.kde.StatusNotifierItem", 1500)
  if not ok then return end
  pcall(proxy.call_with, proxy, "Activate", {
    { signature = "i", value = 0 }, { signature = "i", value = 0 },
  })
end

--- Opens the item's menu as the shell's own menu.
local function open_menu(row)
  if not menus_ok then return activate(row) end
  local menu = dbusmenu.for_item(row.service .. row.path)
  if not menu then return activate(row) end
  local ok, entries = pcall(menu.entries)
  if ok and type(entries) == "table" and morf.menu then
    pcall(morf.menu, entries)
  end
end

--- The row of icons, `size` each, for quick settings.
function tray.build(size)
  local function icon(row)
    return theme.button {
      width = size + S(12), height = size + S(12), radius = 10,
      color = "transparent",
      on_click = function() activate(row) end,
      row.icon ~= "" and ui.Icon { name = row.icon, width = size, height = size, anchors = { center_in = true } }
        or theme.icon { text = "󰀻", size = config.iconSize - 2, anchors = { center_in = true }, color = C.muted },
      ui.MouseArea {
        anchors = { fill = true },
        accepted_buttons = "right",
        on_clicked = function() open_menu(row) end,
      },
    }
  end
  return ui.Row {
    gap = S(4), align = "center",
    visible = function() return tray.count:get() > 0 end,
    theme.label { text = "Tray" },
    ui.Item { width = S(6), height = 1 },
    ui.Repeater { as = "row", gap = S(2), model = tray.items, delegate = icon },
  }
end

return tray
