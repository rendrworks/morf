-- A polkit dialog, drawn by the shell.
--
-- The agent is `examples/lib/polkit_agent.lua`; this is what a shell does
-- with a request. Registered for this process only, so it can be tried while
-- the session's own agent keeps its job:
--
--   pkcheck --action-id org.freedesktop.policykit.exec --process <pid> -u
--
-- and the message shows here. `morf ipc call answer <password>` sends it on;
-- `morf ipc call cancel` gives up. A real shell draws a field instead.

local morf = require("morf")
local ui = require("morf.ui")
local polkit_agent = require("lib.polkit_agent")

morf.surface.height = 44
morf.surface.layer = "overlay"

local shown = morf.signal("polkit.shown", "no request")
local current

local agent, why = polkit_agent.serve {
  subject = "process",
  on_request = function(request)
    current = request
    local line = request.message or request.action_id or "?"
    if request.prompt then line = line .. " -- " .. request.prompt end
    if request.info then line = line .. " (" .. request.info .. ")" end
    shown:set(line .. " [as " .. tostring(request.user) .. "]")
  end,
  on_done = function(request, ok)
    shown:set((ok and "authorised: " or "refused: ") .. tostring(request.action_id))
    current = nil
  end,
}
if not agent then shown:set("no agent: " .. tostring(why)) end

morf.ipc.request = function() return shown:get() end
morf.ipc.answer = function(password)
  if not current then return "no request" end
  current.answer(password or "")
  return "sent"
end
morf.ipc.cancel = function()
  if not current then return "no request" end
  current.cancel()
  return "cancelled"
end

ui.Rect {
  color = "#101418",
  ui.Text {
    anchors = { left = true, margins = 12 },
    color = "#ffffff",
    font_size = 16,
    text = function() return shown:get() end,
  },
}
