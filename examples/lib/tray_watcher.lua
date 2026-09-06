-- The tray watcher, in the configuration.
--
-- A system tray has two halves and morf shipped one. The *host* draws the
-- icons; the *watcher* is the registry the icons announce themselves to, and
-- without one there is nothing to host. On a session with another panel the
-- other panel runs it; on a bare session nobody does, and the tray is simply
-- empty. This is the watcher: `org.kde.StatusNotifierWatcher`, three methods,
-- three properties, three signals, served from Lua on `morf.dbus.serve`.
--
-- An item registers by sending its object path, or its bus name, and the
-- watcher records who sent it: the address a host needs is `sender/path`.
-- When the sender leaves the bus its items go with it, which is the whole job
-- -- a registry that lists programs that have quit is a tray full of icons
-- nothing answers.

local morf = require("morf")

local tray_watcher = {}

local BUS_NAME = "org.kde.StatusNotifierWatcher"
local PATH = "/StatusNotifierWatcher"
local INTERFACE = "org.kde.StatusNotifierWatcher"
local PROPERTIES = "org.freedesktop.DBus.Properties"
local PROTOCOL_VERSION = 0

local function typed(signature, value)
  return { signature = signature, value = value }
end

--- Starts the watcher. Returns it, or nil and why -- usually that another
--- panel already runs one, which is fine: the host will find that one.
function tray_watcher.serve(options)
  options = options or {}
  local service, outcome = morf.dbus.serve("session", BUS_NAME, PATH,
    options.replace == true)
  if outcome ~= "owned" then
    return nil, "the watcher name is " .. outcome
  end

  local watcher = { items = {}, hosts = {} }
  local order = {}

  local function item_list()
    local list = {}
    for _, address in ipairs(order) do list[#list + 1] = address end
    return list
  end

  local function properties()
    return {
      RegisteredStatusNotifierItems = typed("as", item_list()),
      IsStatusNotifierHostRegistered = next(watcher.hosts) ~= nil,
      ProtocolVersion = typed("i", PROTOCOL_VERSION),
    }
  end

  local function add_item(sender, argument)
    -- An item may register with a path ("/StatusNotifierItem") or with a bus
    -- name; either way the thing to talk to is the sender, and the path is
    -- the given one or the conventional one.
    local path = argument
    if not argument or argument == "" or argument:sub(1, 1) ~= "/" then
      path = "/StatusNotifierItem"
    end
    local address = sender .. path
    if watcher.items[address] then return end
    watcher.items[address] = { sender = sender, path = path }
    order[#order + 1] = address
    service:emit(PATH, INTERFACE, "StatusNotifierItemRegistered", address)
  end

  local function drop_sender(name)
    local removed = false
    for index = #order, 1, -1 do
      local address = order[index]
      if watcher.items[address] and watcher.items[address].sender == name then
        watcher.items[address] = nil
        table.remove(order, index)
        service:emit(PATH, INTERFACE, "StatusNotifierItemUnregistered", address)
        removed = true
      end
    end
    if watcher.hosts[name] then
      watcher.hosts[name] = nil
    end
    return removed
  end

  service:on_call(function(call)
    local m = call.member
    if call.interface == PROPERTIES then
      if m == "GetAll" then
        service:reply(call.id, properties())
      elseif m == "Get" then
        local value = properties()[call.arguments[2]]
        if value == nil then
          service:reply_error(call.id, "org.freedesktop.DBus.Error.InvalidArgs",
            "no property " .. tostring(call.arguments[2]))
        else
          service:reply(call.id, typed("v", value))
        end
      else
        service:reply_error(call.id, "org.freedesktop.DBus.Error.PropertyReadOnly",
          "watcher properties are read only")
      end
    elseif m == "RegisterStatusNotifierItem" then
      add_item(call.sender, call.arguments[1])
      service:reply(call.id, nil)
    elseif m == "RegisterStatusNotifierHost" then
      local first = next(watcher.hosts) == nil
      watcher.hosts[call.sender] = true
      service:reply(call.id, nil)
      if first then
        service:emit(PATH, INTERFACE, "StatusNotifierHostRegistered", nil)
      end
    else
      service:reply_error(call.id, "org.freedesktop.DBus.Error.UnknownMethod",
        "no method " .. tostring(m) .. " on " .. INTERFACE)
    end
  end)

  -- Items that quit. The bus says so through NameOwnerChanged with an empty
  -- new owner, and that is the moment an icon nothing answers must go.
  local bus = morf.dbus.proxy("session", "org.freedesktop.DBus",
    "/org/freedesktop/DBus", "org.freedesktop.DBus")
  bus:subscribe("NameOwnerChanged", function(args)
    local name, new_owner = args[1], args[3]
    if type(name) == "string" and new_owner == "" then
      drop_sender(name)
    end
  end)

  --- The registered addresses, each `sender/path`.
  function watcher.list()
    return item_list()
  end

  function watcher.close()
    service:close()
  end

  return watcher
end

return tray_watcher
