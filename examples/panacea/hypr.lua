-- Workspaces, from Hyprland's own sockets.
--
-- Two sockets, both under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`:
-- `.socket2.sock` streams events as `name>>data` lines, and `.socket.sock`
-- answers one request per connection -- `j/monitors`, `j/workspaces`, or a
-- Lua expression after `eval`. Neither is watched by the engine's loop, so
-- both are drained from `hypr.tick()` with a one-millisecond read timeout.
--
-- The pill follows one monitor: whichever `MORF_MONITOR` names, else the
-- focused one, and it moves when focus does. A workspace is "occupied" when
-- it has windows, which is what the disc under a number means.

local morf = require("morf")
local io = require("morf.io")
local core = require("morf.core")
local config = require("config")

local hypr = {}

hypr.state = morf.state {
  rows = {},         -- { id, label, exists, active, urgent }
  active = 0,
  active_label = "1",
  monitor = "",
  keymap = "",       -- "US", "RU": the main keyboard's layout, shortened
}
-- The same rows as a plain list, for logic that walks them.
hypr.rows = {}

local pinned = core.env("MORF_MONITOR")
if pinned == "" then pinned = nil end

-- ------------------------------------------------------------------ paths --

local signature = core.env("HYPRLAND_INSTANCE_SIGNATURE")

local function find_sockets()
  if not signature or signature == "" then return nil, nil end
  local runtime = core.env("XDG_RUNTIME_DIR")
  local directories = {}
  if runtime and runtime ~= "" then directories[#directories + 1] = runtime .. "/hypr/" .. signature end
  directories[#directories + 1] = "/tmp/hypr/" .. signature
  for _, directory in ipairs(directories) do
    for _, suffix in ipairs { ".sock", "" } do
      local ok, socket = pcall(io.socket, directory .. "/.socket2" .. suffix)
      if ok and socket then
        return socket, directory .. "/.socket" .. suffix
      end
    end
  end
  return nil, nil
end

local events, command_path = find_sockets()
hypr.available = command_path ~= nil

local RECEIVE_LIMIT = 64 * 1024
local READ_TIMEOUT_MS = 1
local READS_PER_TICK = 6

-- ---------------------------------------------------------------- requests --

-- One connection per request, at most one per key in flight.
local requests = {}

local function request(key, payload, on_reply)
  if not command_path or requests[key] then return false end
  local ok, socket = pcall(io.socket, command_path)
  if not ok or not socket then return false end
  local sent = pcall(socket.send, socket, payload)
  if sent then sent = pcall(socket.flush, socket) end
  if not sent then
    pcall(socket.close, socket)
    return false
  end
  requests[key] = { socket = socket, buffer = "", on_reply = on_reply }
  return true
end

local function drain_requests()
  local finished = nil
  for key, entry in pairs(requests) do
    for _ = 1, READS_PER_TICK do
      local ok, chunk = pcall(entry.socket.receive, entry.socket, RECEIVE_LIMIT, READ_TIMEOUT_MS)
      if not ok then chunk = "" end
      if chunk == nil then break end
      if chunk == "" then
        finished = finished or {}
        finished[#finished + 1] = key
        break
      end
      entry.buffer = entry.buffer .. chunk
    end
  end
  if not finished then return end
  for _, key in ipairs(finished) do
    local entry = requests[key]
    requests[key] = nil
    pcall(entry.socket.close, entry.socket)
    if entry.on_reply then entry.on_reply(entry.buffer) end
  end
end

--- One request, answered now: for the few things needed before the first
--- frame, such as the gap the compositor keeps between the reserved zone
--- and the windows. Waits at most `timeout_ms`.
function hypr.ask(payload, timeout_ms)
  if not command_path then return nil end
  local ok, socket = pcall(io.socket, command_path)
  if not ok or not socket then return nil end
  local sent = pcall(socket.send, socket, payload)
  if sent then sent = pcall(socket.flush, socket) end
  local reply = ""
  if sent then
    for _ = 1, 8 do
      local received, chunk = pcall(socket.receive, socket, RECEIVE_LIMIT, timeout_ms or 100)
      if not received or chunk == nil or chunk == "" then break end
      reply = reply .. chunk
    end
  end
  pcall(socket.close, socket)
  return reply ~= "" and reply or nil
end

--- The compositor's outer gap on the top edge, in pixels; 0 when unknown.
function hypr.gaps_out()
  local reply = hypr.ask("j/getoption general:gaps_out", 150)
  if not reply then return 0 end
  local ok, option = pcall(io.json.decode, reply)
  if not ok or type(option) ~= "table" then return 0 end
  -- `css` is top, right, bottom, left; `int` is one number for all.
  local top = tostring(option.css or ""):match("^%s*(%d+)")
  return tonumber(top) or tonumber(option["int"]) or 0
end

--- Sends one Lua expression to the compositor. The reply is not needed.
function hypr.eval(expression)
  return request("eval:" .. expression, "eval " .. expression, nil)
end

-- ------------------------------------------------------------------- state --

local pending = { monitors = nil, workspaces = nil }
local last_fingerprint = nil

local function label_of(workspace)
  local name = tostring(workspace.name or "")
  -- `laptop:2`, `samsung:10`: the part after the colon is what the person
  -- calls it.
  local short = name:match(":([^:]+)$") or name
  if short == "" then short = tostring(workspace.id) end
  return short
end

local function rebuild()
  if not pending.monitors or not pending.workspaces then return end
  local ok_monitors, monitors = pcall(io.json.decode, pending.monitors)
  local ok_workspaces, workspaces = pcall(io.json.decode, pending.workspaces)
  pending.monitors, pending.workspaces = nil, nil
  if not ok_monitors or not ok_workspaces then return end
  if type(monitors) ~= "table" or type(workspaces) ~= "table" then return end

  local monitor = nil
  for _, entry in ipairs(monitors) do
    if pinned and entry.name == pinned then monitor = entry end
  end
  if not monitor then
    for _, entry in ipairs(monitors) do
      if entry.focused then monitor = entry end
    end
  end
  monitor = monitor or monitors[1]
  if not monitor then return end
  local active = monitor.activeWorkspace and monitor.activeWorkspace.id or 0

  local mine = {}
  for _, workspace in ipairs(workspaces) do
    if workspace.monitor == monitor.name and type(workspace.id) == "number" and workspace.id > 0 then
      mine[#mine + 1] = workspace
    end
  end
  table.sort(mine, function(a, b) return a.id < b.id end)

  -- Up to `maxWorkspaces` of them, plus any beyond that are active or have
  -- windows -- the same shape as the original's five numbers, which grow
  -- when something lives further out.
  local rows, pieces = {}, {}
  for index, workspace in ipairs(mine) do
    local windows = tonumber(workspace.windows) or 0
    local is_active = workspace.id == active
    if index <= config.maxWorkspaces or is_active or windows > 0 then
      rows[#rows + 1] = {
        id = workspace.id,
        label = label_of(workspace),
        exists = windows > 0,
        active = is_active,
        urgent = false,
      }
      pieces[#pieces + 1] = workspace.id .. ":" .. windows .. (is_active and "*" or "")
    end
  end
  local fingerprint = monitor.name .. "|" .. table.concat(pieces, ",")
  if fingerprint == last_fingerprint then return end
  last_fingerprint = fingerprint

  hypr.rows = rows
  hypr.state.rows:replace(rows, "id")
  hypr.state.active = active
  hypr.state.monitor = monitor.name
  for _, row in ipairs(rows) do
    if row.active then hypr.state.active_label = row.label end
  end
end

--- `English (US)` is `US`, `Russian` is `RU`: the first two letters of
--- what is in the brackets, or of the name.
local function short_layout(name)
  name = tostring(name or "")
  local inner = name:match("%((%a+)%)")
  if inner then return inner:sub(1, 2):upper() end
  return name:sub(1, 2):upper()
end

local function refresh_keymap()
  request("devices", "j/devices", function(reply)
    local ok, devices = pcall(io.json.decode, reply)
    if not ok or type(devices) ~= "table" then return end
    for _, keyboard in ipairs(devices.keyboards or {}) do
      if keyboard.main then
        hypr.state.keymap = short_layout(keyboard.active_keymap)
        return
      end
    end
    local first = (devices.keyboards or {})[1]
    if first then hypr.state.keymap = short_layout(first.active_keymap) end
  end)
end
hypr.refresh_keymap = refresh_keymap

local function refresh()
  request("monitors", "j/monitors", function(reply)
    pending.monitors = reply
    rebuild()
  end)
  request("workspaces", "j/workspaces", function(reply)
    pending.workspaces = reply
    rebuild()
  end)
end
hypr.refresh = refresh

-- ------------------------------------------------------------------ events --

local WATCHED = {
  workspace = true, workspacev2 = true, focusedmon = true, focusedmonv2 = true,
  openwindow = true, closewindow = true, movewindow = true, movewindowv2 = true,
  createworkspace = true, createworkspacev2 = true,
  destroyworkspace = true, destroyworkspacev2 = true,
  moveworkspace = true, moveworkspacev2 = true, renameworkspace = true,
  urgent = true, monitoradded = true, monitorremoved = true,
}

local event_buffer = ""
local refresh_wanted = false
hypr.on_event = nil

local function drain_events()
  if not events then return end
  for _ = 1, READS_PER_TICK do
    local ok, chunk = pcall(events.receive, events, RECEIVE_LIMIT, READ_TIMEOUT_MS)
    if not ok or chunk == nil then break end
    if chunk == "" then
      -- The compositor closed the stream; it is not coming back.
      pcall(events.close, events)
      events = nil
      break
    end
    event_buffer = event_buffer .. chunk
  end
  while true do
    local line, rest = event_buffer:match("^(.-)\n(.*)$")
    if not line then break end
    event_buffer = rest
    local name, data = line:match("^([^>]+)>>(.*)$")
    if name and WATCHED[name] then refresh_wanted = true end
    if name == "activelayout" then refresh_keymap() end
    if name and hypr.on_event then hypr.on_event(name, data or "") end
  end
end

--- Reads whatever the sockets have; called on the shell's tick.
function hypr.tick()
  drain_events()
  drain_requests()
  if refresh_wanted then
    refresh_wanted = false
    refresh()
  end
end

-- Without an event socket, the state still follows, just later.
if not events then morf.timer(2000, function() refresh_wanted = true end, true) end

-- ------------------------------------------------------------------- verbs --

--- Goes to workspace `id`.
function hypr.go_to(id)
  hypr.eval(string.format('hl.dispatch(hl.dsp.focus({ workspace = "%d" }))', id))
  refresh_wanted = true
end

--- The next or previous workspace among the ones shown, wrapping.
function hypr.step(by)
  local rows = hypr.rows
  local count = #rows
  if count == 0 then return end
  local index = 1
  for i, row in ipairs(rows) do
    if row.active then index = i break end
  end
  local target = rows[((index - 1 + by) % count) + 1]
  if target then hypr.go_to(target.id) end
end

refresh()
refresh_keymap()

--- Focuses a window by its address.
function hypr.focus_window(address)
  hypr.eval(string.format('hl.dispatch(hl.dsp.focus({ window = "address:%s" }))', address))
end

return hypr
