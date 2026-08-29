local mold = require("mold")

local StatusNotifier = {}

local interfaces = {
  "org.kde.StatusNotifierItem",
  "org.freedesktop.StatusNotifierItem",
}

local function item_proxy(address, interface)
  return mold.dbus.proxy("session", address.service, address.path, interface)
end

local function call_item(address, method, signature, values)
  local last_error
  for _, interface in ipairs(interfaces) do
    local proxy = item_proxy(address, interface)
    local ok, result = pcall(proxy.call_with, proxy, method, {
      signature = signature,
      value = values,
    })
    if ok then return result end
    last_error = result
  end
  error(last_error)
end

local function get_item(address, property)
  local last_error
  for _, interface in ipairs(interfaces) do
    local proxy = item_proxy(address, interface)
    local ok, result = pcall(proxy.get, proxy, property)
    if ok then return result end
    last_error = result
  end
  error(last_error)
end

local function wrap(address)
  return {
    service = address.service,
    path = address.path,
    get = function(_, property) return get_item(address, property) end,
    activate = function(_, x, y) return call_item(address, "Activate", "(ii)", { x or 0, y or 0 }) end,
    secondary_activate = function(_, x, y)
      return call_item(address, "SecondaryActivate", "(ii)", { x or 0, y or 0 })
    end,
    context_menu = function(_, x, y)
      return call_item(address, "ContextMenu", "(ii)", { x or 0, y or 0 })
    end,
    scroll = function(_, delta, orientation)
      return call_item(address, "Scroll", "(is)", { delta, orientation or "vertical" })
    end,
    menu = function()
      local path = get_item(address, "Menu")
      if not path or path == "/" then return nil end
      local proxy = mold.dbus.proxy("session", address.service, path, "com.canonical.dbusmenu")
      return {
        layout = function(_, properties)
          return proxy:call_with("GetLayout", {
            signature = "(iias)",
            value = { 0, -1, properties or {
              "label", "enabled", "visible", "type", "toggle-type", "toggle-state", "icon-name",
            } },
          })
        end,
        event = function(_, id, name, data, timestamp)
          return proxy:call_with("Event", {
            signature = "(isvu)",
            value = {
              id,
              name or "clicked",
              data or { signature = "s", value = "" },
              timestamp or 0,
            },
          })
        end,
      }
    end,
  }
end

function StatusNotifier.subscribe(callback)
  return mold.status_notifier.subscribe(function(addresses)
    local items = {}
    for index, address in ipairs(addresses) do items[index] = wrap(address) end
    callback(items)
  end)
end

return StatusNotifier
