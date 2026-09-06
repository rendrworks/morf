-- A polkit authentication agent, in the configuration.
--
-- When a program asks polkit for something it is not allowed, polkit asks the
-- session's *agent* to authenticate the person, and the agent shows the
-- dialog. There is nothing privileged about the agent itself: it registers
-- with the authority, is called back at its own object path, and hands the
-- password to `polkit-agent-helper-1` -- a small root helper, behind a socket
-- on current systems and setuid on older ones, that runs PAM and tells the
-- authority the result. The agent never sees a verdict
-- directly; it sees the helper say SUCCESS or FAILURE.
--
-- So this is three things on `morf.dbus.serve`: a nameless object on the
-- system bus (the authority calls back a unique name, and the system bus
-- would not grant an unprivileged process a well-known one), one method that
-- must not answer until the person has, and a helper driven over stdio. What
-- the dialog looks like is the shell's business; the request it draws comes
-- through `on_request`.

local morf = require("morf")

local polkit_agent = {}

local AUTHORITY = "org.freedesktop.PolicyKit1"
local AUTHORITY_PATH = "/org/freedesktop/PolicyKit1/Authority"
local AUTHORITY_INTERFACE = "org.freedesktop.PolicyKit1.Authority"
local AGENT_INTERFACE = "org.freedesktop.PolicyKit1.AuthenticationAgent"
local HELPER = "/usr/lib/polkit-1/polkit-agent-helper-1"
local HELPER_SOCKET = "/run/polkit/agent-helper.socket"
local CANCELLED = "org.freedesktop.PolicyKit1.Error.Cancelled"
local FAILED = "org.freedesktop.PolicyKit1.Error.Failed"

--- uid -> login name, read once from the passwd file the helper needs it in.
local function usernames()
  local names = {}
  local ok, text = pcall(function() return morf.file("/etc/passwd"):read() end)
  if not ok or type(text) ~= "string" then return names end
  for line in text:gmatch("[^\n]+") do
    local name, uid = line:match("^([^:]+):[^:]*:(%d+):")
    if name then names[tonumber(uid)] = name end
  end
  return names
end

--- The identities polkit offers, as `{ kind, uid, name }`.
local function identities_of(list, names)
  local out = {}
  for _, identity in ipairs(list or {}) do
    local kind, details = identity[1], identity[2] or {}
    if kind == "unix-user" and details.uid then
      local uid = tonumber(details.uid)
      out[#out + 1] = { kind = kind, uid = uid, name = names[uid] or tostring(uid) }
    elseif kind == "unix-group" and details.gid then
      out[#out + 1] = { kind = kind, gid = tonumber(details.gid), name = "group " .. tostring(details.gid) }
    end
  end
  return out
end

--- Registers with the authority and starts answering.
---
--- `options.on_request(request)` is called for every authentication polkit
--- asks for. The request carries `action_id`, `message`, `icon`, `details`,
--- `cookie`, `identities`, and `prompt` once the helper has asked; the shell
--- answers with `request.answer(password)` or gives up with
--- `request.cancel()`. `on_done(request, ok, why)` says how it ended.
---
--- `options.subject` chooses what the agent answers for: `"session"` (the
--- default) is this login session, `"process"` is this process only, which is
--- what a test wants when another agent already serves the session.
function polkit_agent.serve(options)
  options = options or {}
  local on_request = options.on_request or function() end
  local on_done = options.on_done or function() end
  local path = options.path or "/org/morf/PolkitAgent"

  local service, outcome = morf.dbus.serve("system", "", path, false)
  if outcome ~= "owned" then
    return nil, "could not serve on the system bus: " .. tostring(outcome)
  end

  local agent = { pending = {}, names = usernames() }

  local function finish(request, ok, error_name, message)
    if not agent.pending[request.cookie] then return end
    agent.pending[request.cookie] = nil
    if request.helper then
      pcall(function() request.helper:kill() end)
      request.helper = nil
    end
    if request.socket then
      pcall(function() request.socket:close() end)
      request.socket = nil
    end
    if ok then
      service:reply(request.call_id, nil)
    else
      service:reply_error(request.call_id, error_name or FAILED, message or "authentication failed")
    end
    on_done(request, ok, message)
  end

  --- Feeds the helper's lines to the request: prompts out, verdict back.
  local function helper_line(request, text)
    -- A character class rather than `%u`: the underscore in `PAM_TEXT_INFO`
    -- is not a letter, and `%u+` stopped at it and matched nothing useful.
    local style, rest = text:match("^(PAM_[A-Z_]+)%s?(.*)$")
    if style == "PAM_PROMPT_ECHO_OFF" or style == "PAM_PROMPT_ECHO_ON" then
      request.prompt = rest
      request.echo = style == "PAM_PROMPT_ECHO_ON"
      on_request(request)
    elseif style == "PAM_TEXT_INFO" or style == "PAM_ERROR_MSG" then
      request.info = rest
      on_request(request)
    elseif text == "SUCCESS" then
      finish(request, true)
    elseif text == "FAILURE" then
      finish(request, false, FAILED, "authentication failed")
    end
  end

  --- Drives the helper for one request: user and cookie in, prompts out,
  --- verdict back.
  ---
  --- Two ways to reach it. Since polkit 124 the helper is a root service
  --- behind `/run/polkit/agent-helper.socket`, which takes the user name and
  --- the cookie as its first two lines and identifies the caller from the
  --- socket; older systems have it as a setuid program that takes the user
  --- on its command line and the cookie on stdin. The lines it speaks are
  --- the same either way.
  local function run_helper(request, user)
    local buffer = ""
    local function feed(data)
      buffer = buffer .. data
      while true do
        local newline = buffer:find("\n", 1, true)
        if not newline then break end
        helper_line(request, buffer:sub(1, newline - 1))
        buffer = buffer:sub(newline + 1)
      end
    end
    local ok, socket = pcall(morf.socket, HELPER_SOCKET)
    if ok and socket then
      request.socket = socket
      socket:send(user .. "\n" .. request.cookie .. "\n")
      socket:flush()
    else
      local helper = morf.process(HELPER, { user })
      request.helper = helper
      -- The cookie is the first line; the helper is the one that tells the
      -- authority, and this is how it knows which request it is answering.
      helper:write(request.cookie .. "\n")
    end
    -- Polled rather than awaited, twenty milliseconds at a time, and stopped
    -- the moment the helper is gone: a handle is what makes that possible.
    local tick
    tick = morf.timer(20, function()
      if not request.socket and not request.helper then
        if tick then tick:cancel() end
        return
      end
      if request.socket then
        for _ = 1, 32 do
          local data = request.socket:receive(4096, 1)
          if data == nil then break end
          if data == "" then
            -- The far end closed: a helper that leaves without a verdict
            -- failed, whatever it said.
            request.socket:close()
            request.socket = nil
            if agent.pending[request.cookie] then
              finish(request, false, FAILED, "the helper went away")
            end
            tick:cancel()
            return
          end
          feed(data)
          -- A verdict in that data closed the socket; there is nothing more
          -- to read from a handle that is gone.
          if not request.socket then
            tick:cancel()
            return
          end
        end
        return
      end
      local helper = request.helper
      for _ = 1, 32 do
        local event = helper:next()
        if not event then break end
        if event.kind == "stdout" then
          feed(event.data)
        elseif event.kind == "stderr" then
          request.info = event.data
          on_request(request)
        elseif event.kind == "exit" then
          request.helper = nil
          if agent.pending[request.cookie] then
            finish(request, false, FAILED, "the helper exited")
          end
          tick:cancel()
          return
        end
      end
    end, true)
  end

  service:on_call(function(call)
    if call.interface ~= AGENT_INTERFACE then
      service:reply_error(call.id, "org.freedesktop.DBus.Error.UnknownInterface",
        "no interface " .. tostring(call.interface))
      return
    end
    local a = call.arguments
    if call.member == "BeginAuthentication" then
      local request = {
        call_id = call.id,
        action_id = a[1], message = a[2], icon = a[3], details = a[4] or {},
        cookie = a[5], identities = identities_of(a[6], agent.names),
      }
      local user = request.identities[1]
      if not user or user.kind ~= "unix-user" then
        service:reply_error(call.id, FAILED, "no user to authenticate as")
        return
      end
      request.user = user.name
      function request.answer(password)
        if request.socket then
          request.socket:send(tostring(password) .. "\n")
          request.socket:flush()
        elseif request.helper then
          request.helper:write(tostring(password) .. "\n")
        end
      end
      function request.cancel()
        finish(request, false, CANCELLED, "cancelled by the user")
      end
      agent.pending[request.cookie] = request
      on_request(request)
      run_helper(request, user.name)
    elseif call.member == "CancelAuthentication" then
      local request = agent.pending[a[1]]
      if request then finish(request, false, CANCELLED, "cancelled") end
      service:reply(call.id, nil)
    else
      service:reply_error(call.id, "org.freedesktop.DBus.Error.UnknownMethod",
        "no method " .. tostring(call.member))
    end
  end)

  -- Register. The subject says whose requests this agent answers.
  local subject
  if options.subject == "process" then
    subject = { "unix-process", {
      pid = { signature = "u", value = morf.process_id },
      ["start-time"] = { signature = "t", value = 0 },
    } }
  else
    subject = { "unix-session", { ["session-id"] = morf.env("XDG_SESSION_ID") or "" } }
  end
  -- Registered from the service's own connection, not a proxy: the
  -- authority remembers the unique name that registered and calls *that*
  -- name back at `path`. A proxy is another connection with another name,
  -- and an agent registered through one is called where nothing answers.
  local ok, err = pcall(function()
    -- The path travels as a plain string here; the interface was written
    -- before anyone minded the difference.
    service:call(AUTHORITY, AUTHORITY_PATH, AUTHORITY_INTERFACE, "RegisterAuthenticationAgent", {
      { signature = "(sa{sv})", value = subject },
      morf.env("LANG") or "C",
      path,
    })
  end)
  if not ok then
    service:close()
    return nil, "the authority refused the agent: " .. tostring(err)
  end

  function agent.close()
    pcall(function()
      service:call(AUTHORITY, AUTHORITY_PATH, AUTHORITY_INTERFACE, "UnregisterAuthenticationAgent", {
        { signature = "(sa{sv})", value = subject },
        path,
      })
    end)
    service:close()
  end

  return agent
end

return polkit_agent
