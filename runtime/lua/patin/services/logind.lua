local mold = require("mold")

local Logind = {}

function Logind.new(session_path)
  assert(session_path and session_path ~= "", "logind session path is required")
  local session = mold.dbus.proxy(
    "system",
    "org.freedesktop.login1",
    session_path,
    "org.freedesktop.login1.Session"
  )
  return {
    active = function() return session:get("Active") end,
    idle_hint = function() return session:get("IdleHint") end,
    locked_hint = function() return session:get("LockedHint") end,
    lock = function() return session:call("Lock") end,
    unlock = function() return session:call("Unlock") end,
    activate = function() return session:call("Activate") end,
  }
end

return Logind
