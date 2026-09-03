-- A tray item's menu, read off the bus and handed to `morf.menu`.
--
-- Every tray icon with a right-click menu publishes it as
-- `com.canonical.dbusmenu`: a tree of ids with properties, fetched with
-- `GetLayout`, and a `clicked` event sent back with `Event`. The vocabulary
-- is nearly `morf.menu`'s already -- label, enabled, visible, separator,
-- checkmark or radio, submenu -- so this is a translation, not a model. The
-- item's address comes from the tray host; the menu's path is the item's own
-- `Menu` property.

local morf = require("morf")

local dbusmenu = {}

local ITEM_INTERFACE = "org.kde.StatusNotifierItem"
local MENU_INTERFACE = "com.canonical.dbusmenu"

local function i(value) return { signature = "i", value = value } end
local function u(value) return { signature = "u", value = value } end

--- A label as a person should see it: `_File` is "File", `__` is "_".
local function strip_mnemonic(label)
  return (tostring(label or ""):gsub("__", "\0"):gsub("_", ""):gsub("%z", "_"))
end

--- One dbusmenu node -- `{ id, properties, children }` as the engine decodes
--- the `(ia{sv}av)` struct -- as a `morf.menu` entry.
local function convert(node, on_click)
  local id, props, children = node[1], node[2] or {}, node[3] or {}
  local entry = {
    id = tostring(id),
    text = strip_mnemonic(props.label),
    separator = props.type == "separator",
    enabled = props.enabled ~= false,
    visible = props.visible ~= false,
  }
  if type(props["icon-name"]) == "string" and props["icon-name"] ~= "" then
    entry.icon = props["icon-name"]
  end
  local toggle = props["toggle-type"]
  if toggle == "checkmark" then
    entry.button_type = "checkbox"
  elseif toggle == "radio" then
    entry.button_type = "radio"
  end
  if toggle then
    local state = props["toggle-state"]
    entry.checked = state == 1 and "checked" or state == 0 and "unchecked" or "partial"
  end
  if props["children-display"] == "submenu" or #children > 0 then
    entry.children = {}
    for index, child in ipairs(children) do
      entry.children[index] = convert(child, on_click)
    end
  elseif not entry.separator then
    entry.on_triggered = function() on_click(id) end
  end
  return entry
end

--- Opens the menu published at `path` by `service`.
---
--- Returns an object with `entries()`, which fetches the current tree as
--- `morf.menu` entries, and `click(id)`, which tells the application. Both
--- talk to the bus when called; a menu is read when it is about to be shown,
--- not kept in step forever.
function dbusmenu.open(service, path, options)
  options = options or {}
  local proxy = morf.dbus.proxy("session", service, path, MENU_INTERFACE,
    options.timeout_ms or 2000)
  local menu = { service = service, path = path, revision = 0 }

  function menu.click(id)
    -- The timestamp is for ordering events, not reading a clock, and the
    -- sandbox keeps `os` out of a configuration's reach; zero is what every
    -- other shell sends here too.
    proxy:call_with("Event", { i(id), "clicked", { signature = "v", value = "" }, u(0) })
  end

  function menu.entries()
    -- Applications build submenus lazily; asking first is what makes a lazy
    -- one fill in. Ignored when the application does not care.
    pcall(function() proxy:call_with("AboutToShow", i(0)) end)
    local reply = proxy:call_with("GetLayout", { i(0), i(-1), { signature = "as", value = {} } })
    -- `(u (ia{sv}av))`: revision, then the root node whose children are the
    -- menu. The root itself has no label and is not shown.
    menu.revision = reply[1]
    local root = reply[2] or {}
    local entries = {}
    for index, child in ipairs(root[3] or {}) do
      entries[index] = convert(child, menu.click)
    end
    return entries
  end

  return menu
end

--- The menu of a tray item, found through the item's own `Menu` property.
---
--- `address` is what the tray host hands out: `sender/path`. Returns nil when
--- the item publishes no menu, which many do not.
function dbusmenu.for_item(address, options)
  local service, path = address:match("^([^/]+)(/.*)$")
  if not service then return nil, "not an item address: " .. tostring(address) end
  local item = morf.dbus.proxy("session", service, path, ITEM_INTERFACE)
  local ok, menu_path = pcall(function() return item:get("Menu") end)
  if not ok or type(menu_path) ~= "string" or menu_path == "" or menu_path == "/" then
    return nil, "the item publishes no menu"
  end
  return dbusmenu.open(service, menu_path, options)
end

return dbusmenu
