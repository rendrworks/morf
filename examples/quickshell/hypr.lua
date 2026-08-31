-- Workspace and monitor state from Hyprland, mirroring the `Connections` and
-- `Process` blocks in `line/modules/line/Line.qml` and `Numbers.qml`.
--
-- The original listens on Hyprland's event socket through `Quickshell.Hyprland`
-- and re-reads `hyprctl monitors -j` / `hyprctl workspaces -j` whenever an
-- interesting event lands. This does the same over the two raw sockets
-- Hyprland exposes:
--
--   $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock  events
--   $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock   commands
--
-- The event stream is a line protocol, `name>>data`, read through a line
-- parser. A command is one connection per request: write `j/monitors`, read
-- until the compositor closes the stream. Neither socket is watched by the
-- engine's poll loop, so both are drained from `poll`, on the ribbon's tick,
-- with a one millisecond read timeout — long enough for the kernel to hand
-- over a reply that is already queued, short enough not to cost a frame. A
-- zero timeout is not usable: the OS rejects it (`cannot set a 0 duration
-- timeout`) rather than polling.
--
-- If the sockets are unavailable (no Hyprland, or a build that does not expose
-- them) everything falls back to forking `hyprctl`, which is what this module
-- did before, on a slower timer.
--
-- Nothing here is engine surface: the engine supplies sockets, a line parser,
-- JSON, processes and a timer, and a shell plugin decides what a workspace is.

local core = require("mold.core")
local io = require("mold.io")
local mold = require("mold")

local hypr = {}

-- Ten pills, matching the ten rows the original always draws.
local ROW_COUNT = 10
hypr.ROW_COUNT = ROW_COUNT

-- Bumped whenever a refresh changes anything, so bindings can depend on the
-- workspace set without every row being its own signal.
hypr.revision = mold.signal("quickshell.hypr.revision", 0)

local rows = {}
for index = 1, ROW_COUNT do
  rows[index] = { id = index, active = index == 1, windows = 0 }
end

-- Monitor geometry, kept for the side the bar hangs on. `mold.screens` only
-- reports the output this process draws to, so the neighbours have to come
-- from the compositor.
local monitors = {}
local monitor_order = {}

-- Which output this shell follows. `MOLD_MONITOR` wins, then the output the
-- surface was placed on, then whichever monitor Hyprland reports as focused.
local monitor_name = core.env("MOLD_MONITOR")
if not monitor_name or monitor_name == "" then
  local screen = (mold.screens or {})[1]
  monitor_name = screen and screen.name or nil
end

-- `Workspace.qml` mirrors the ribbon to the right edge on every monitor that
-- sits left of the main one; `positionMode` overrides that either way.
local MAIN_MONITOR = core.env("MOLD_MAIN_MONITOR") or "eDP-1"
local POSITION_MODE = core.env("MOLD_BAR_SIDE") or "auto"

local pending_badge = nil
local last_active_id = nil
local refresh_pending = false

--- The first workspace of the block of ten the given id falls in.
local function workspace_base(id)
  if not id or id <= 0 then return 1 end
  return id - ((id - 1) % 10)
end

--- Records that the badge should show a workspace, as `showWorkspaceId` does.
local function request_badge(id, force)
  if not id or id <= 0 then return end
  pending_badge = { id = id, force = force and true or false }
end

--- Hands the pending badge request to the ribbon, once.
function hypr.take_badge()
  local request = pending_badge
  pending_badge = nil
  return request
end

-- ---------------------------------------------------------------- sockets --

local SIGNATURE = core.env("HYPRLAND_INSTANCE_SIGNATURE")

--- Every directory Hyprland is known to keep its sockets in, newest first.
local function socket_directories()
  local directories = {}
  if not SIGNATURE or SIGNATURE == "" then return directories end
  local runtime = core.env("XDG_RUNTIME_DIR")
  if runtime and runtime ~= "" then
    directories[#directories + 1] = runtime .. "/hypr/" .. SIGNATURE
  end
  directories[#directories + 1] = "/tmp/hypr/" .. SIGNATURE
  return directories
end

-- Current builds name the sockets `.socket2.sock` and `.socket.sock`; some
-- drop the suffix. Whichever one connects also names the command socket.
local SUFFIXES = { ".sock", "" }

local events = nil
local command_path = nil
local event_lines = io.split_parser("\n")

-- `MOLD_HYPR_TRANSPORT=hyprctl` forces the fallback, which is the only way to
-- exercise it on a machine whose sockets are perfectly healthy.
local FORCE_HYPRCTL = core.env("MOLD_HYPR_TRANSPORT") == "hyprctl"

local function connect_events()
  if FORCE_HYPRCTL then return nil end
  for _, directory in ipairs(socket_directories()) do
    for _, suffix in ipairs(SUFFIXES) do
      local ok, socket = pcall(io.socket, directory .. "/.socket2" .. suffix)
      if ok and socket then
        command_path = directory .. "/.socket" .. suffix
        return socket
      end
    end
  end
  return nil
end

events = connect_events()

-- One read never takes more than this, and a drain never spends more than a
-- handful of them, so a stalled compositor cannot hold up a frame.
local RECEIVE_LIMIT = 64 * 1024
local READ_TIMEOUT_MS = 1
local FIRST_READ_TIMEOUT_MS = 4
local READS_PER_TICK = 6

-- At most one request per key in flight, so a busy tick cannot pile up
-- connections. Replies are handled after the drain loop, never inside it: a
-- handler may start the next request.
local requests = {}

local function begin_request(key, payload, on_reply)
  if not command_path or requests[key] then return false end
  local ok, socket = pcall(io.socket, command_path)
  if not ok or not socket then
    -- The compositor went away. Fall back to `hyprctl` from here on.
    command_path = nil
    return false
  end
  local sent = pcall(socket.send, socket, payload)
  if sent then sent = pcall(socket.flush, socket) end
  if not sent then
    pcall(socket.close, socket)
    return false
  end
  requests[key] = { socket = socket, buffer = "", on_reply = on_reply, fresh = true }
  return true
end

local function drain_requests()
  local keys = nil
  for key in pairs(requests) do
    keys = keys or {}
    keys[#keys + 1] = key
  end
  if not keys then return end
  local finished = nil
  for _, key in ipairs(keys) do
    local request = requests[key]
    local done = false
    for index = 1, READS_PER_TICK do
      -- The first read of a fresh request waits a little longer, so a reply
      -- the compositor is already writing lands on the tick it was asked for
      -- rather than one tick later.
      local timeout = (request.fresh and index == 1) and FIRST_READ_TIMEOUT_MS or READ_TIMEOUT_MS
      local ok, chunk = pcall(request.socket.receive, request.socket, RECEIVE_LIMIT, timeout)
      request.fresh = false
      if not ok then chunk = "" end
      if chunk == nil then break end
      if chunk == "" then
        done = true
        break
      end
      request.buffer = request.buffer .. chunk
    end
    if done then
      finished = finished or {}
      finished[#finished + 1] = key
    end
  end
  if not finished then return end
  -- Replies are handled after the drain, never inside it: a handler may start
  -- the next request, and adding a key while traversing is not allowed.
  for _, key in ipairs(finished) do
    local request = requests[key]
    requests[key] = nil
    pcall(request.socket.close, request.socket)
    if request.on_reply then request.on_reply(request.buffer) end
  end
end

-- ------------------------------------------------------------------ state --

local pending = { monitors = nil, workspaces = nil }

--- Rebuilds the ten rows once both queries have reported.
local function rebuild()
  if not pending.monitors or not pending.workspaces then return end
  local ok_monitors, decoded_monitors = pcall(io.json.decode, pending.monitors)
  local ok_workspaces, workspaces = pcall(io.json.decode, pending.workspaces)
  if not ok_monitors or not ok_workspaces then return end
  if type(decoded_monitors) ~= "table" or type(workspaces) ~= "table" then return end

  monitors = {}
  monitor_order = {}
  for _, entry in ipairs(decoded_monitors) do
    if type(entry.name) == "string" then
      local record = {
        name = entry.name,
        x = entry.x or 0,
        y = entry.y or 0,
        width = entry.width or 0,
        height = entry.height or 0,
        focused = entry.focused and true or false,
        active_id = entry.activeWorkspace and entry.activeWorkspace.id or nil,
      }
      monitors[entry.name] = record
      monitor_order[#monitor_order + 1] = record
    end
  end

  -- Without a name to match, the focused monitor is the one to follow.
  local monitor
  if monitor_name then monitor = monitors[monitor_name] end
  if not monitor then
    for _, record in ipairs(monitor_order) do
      if record.focused then monitor = record break end
    end
  end
  monitor = monitor or monitor_order[1]
  if not monitor then return end

  local active = monitor.active_id or 1
  local base = workspace_base(active)
  local changed = false
  for offset = 0, ROW_COUNT - 1 do
    local id = base + offset
    local windows = 0
    for _, workspace in ipairs(workspaces) do
      if workspace.id == id and workspace.monitor == monitor.name then
        windows = math.max(windows, workspace.windows or 0)
      end
    end
    local row = rows[offset + 1]
    if row.id ~= id or row.active ~= (id == active) or row.windows ~= windows then
      row.id, row.active, row.windows = id, id == active, windows
      changed = true
    end
  end
  if changed then
    hypr.revision:set(hypr.revision:get() + 1)
  end
  -- `Numbers.qml` shows the badge from `onActiveWorkspaceChanged`, which is
  -- exactly this edge.
  if active ~= last_active_id then
    last_active_id = active
    request_badge(active, false)
  end
end

-- ------------------------------------------------------------ hyprctl path --

-- The process views are built once, at load, rather than on first use. A view
-- created later, from inside a timer callback, is not picked up by the running
-- service loop and never reports a line of output.
local COMMANDS = {
  monitors = { "hyprctl", "monitors", "-j" },
  workspaces = { "hyprctl", "workspaces", "-j" },
}
-- `hyprctl` is a system binary and must not inherit this process's dynamic
-- linker search path. Launching mold through a nixGL-style wrapper replaces
-- LD_LIBRARY_PATH with nix store paths, and a child that picks those up fails
-- to load its own libstdc++ and exits before printing a line.
local CHILD_ENVIRONMENT = { LD_LIBRARY_PATH = "" }

local views = {}
for key, command in pairs(COMMANDS) do
  views[key] = {
    process = io.process_view { command = command, environment = CHILD_ENVIRONMENT },
    buffer = "",
    busy = false,
  }
end

--- Starts one hyprctl query, reusing its process view across refreshes.
---
--- Spawning raises when the binary is not on PATH, and this runs at module
--- load, so an unguarded start would abort `require` and take the whole shell
--- down on a machine without hyprctl. A query that cannot start simply leaves
--- the workspace rows at their defaults.
local function query(key)
  local view = views[key]
  if not view or view.busy then return end
  view.buffer = ""
  -- Reassigning the command is what makes a finished view runnable again.
  local ok = pcall(function()
    view.process:set_command(COMMANDS[key])
    view.process:start()
  end)
  view.busy = ok
end

-- How long one drain will wait on a query, and how many times. Process output
-- is delivered while `next` waits, so a purely non-blocking drain never
-- advances. The budget is small and only spent while a query is actually in
-- flight, which keeps the frame loop responsive.
local DRAIN_SLICE_MS = 2
local DRAIN_SLICES = 12

--- Collects whatever the running queries have produced.
local function drain_processes()
  for key, view in pairs(views) do
    if view.busy then
      for _ = 1, DRAIN_SLICES do
        local event = view.process:next(DRAIN_SLICE_MS)
        if not event then
          -- Nothing yet; leave the rest for the next tick.
          break
        end
        if event.kind == "stdout" then
          view.buffer = view.buffer .. (event.data or "")
        elseif event.kind == "exit" then
          pending[key] = view.buffer
          view.busy = false
          rebuild()
          break
        end
      end
    end
  end
end

-- ------------------------------------------------------------------ events --

-- The events `Line.qml` refreshes on, plus the v2 spellings and the monitor
-- events the mirrored layout depends on.
local WATCHED = {
  workspace = true,
  workspacev2 = true,
  movewindow = true,
  movewindowv2 = true,
  openwindow = true,
  closewindow = true,
  moveworkspace = true,
  moveworkspacev2 = true,
  createworkspace = true,
  createworkspacev2 = true,
  destroyworkspace = true,
  destroyworkspacev2 = true,
  focusedmon = true,
  focusedmonv2 = true,
  monitoradded = true,
  monitoraddedv2 = true,
  monitorremoved = true,
}

--- `showFocusedMonitorEvent` from `Numbers.qml`: `monitorname,workspaceid`.
local function focused_monitor_event(data)
  local comma = string.find(data, ",", 1, true)
  if not comma then return end
  if string.sub(data, 1, comma - 1) ~= monitor_name then return end
  local id = tonumber(string.sub(data, comma + 1))
  if id then request_badge(id, true) end
end

local function handle_event(name, data)
  if name == "focusedmonv2" then
    focused_monitor_event(data)
  end
  if WATCHED[name] then
    refresh_pending = true
  end
end

local function drain_events()
  if not events then return end
  for _ = 1, READS_PER_TICK do
    local ok, chunk = pcall(events.receive, events, RECEIVE_LIMIT, READ_TIMEOUT_MS)
    if not ok or chunk == "" then
      -- The compositor closed the stream, or the socket faulted. Everything
      -- keeps working on the `hyprctl` path from here.
      pcall(events.close, events)
      events = nil
      return
    end
    if chunk == nil then return end
    for _, line in ipairs(event_lines:push(chunk)) do
      local at = string.find(line, ">>", 1, true)
      if at then
        handle_event(string.sub(line, 1, at - 1), string.sub(line, at + 2))
      end
    end
  end
end

-- ----------------------------------------------------------------- queries --

local function issue_refresh()
  if not refresh_pending then return end
  refresh_pending = false
  if command_path then
    begin_request("monitors", "j/monitors", function(text)
      pending.monitors = text
      rebuild()
    end)
    begin_request("workspaces", "j/workspaces", function(text)
      pending.workspaces = text
      rebuild()
    end)
  else
    query("monitors")
    query("workspaces")
  end
end

--- Asks for a refresh. Coalesced: the tick issues at most one pair of queries.
function hypr.refresh()
  refresh_pending = true
end

--- Advances the sockets and any in-flight query. Call from a timer.
function hypr.poll()
  drain_events()
  issue_refresh()
  drain_requests()
  drain_processes()
end

--- Reports whether the event socket is carrying the state.
function hypr.using_sockets()
  return events ~= nil and command_path ~= nil
end

-- ---------------------------------------------------------------- dispatch --

-- Hyprland 0.56 replaced the flat `dispatch workspace 3` request with a Lua
-- one, `hl.dsp.focus{ workspace = "3" }`, and answers the old spelling with an
-- error rather than acting on it. Neither form can be detected up front, so
-- the first dispatch tries the classic request and switches for good if the
-- compositor rejects it.
local dispatch_form = "classic"

local function dispatch_payload(target)
  if dispatch_form == "classic" then
    return "/dispatch workspace " .. target
  end
  return '/dispatch hl.dsp.focus{ workspace = "' .. target .. '" }'
end

--- Switches workspace, matching `dispatchWorkspace` in the original. Accepts
--- an id or a relative token such as `r+1`.
function hypr.dispatch(target)
  local text = tostring(target)
  if not string.match(text, "^[%w%+%-_:]+$") then return end
  if command_path then
    local retry = dispatch_form == "classic"
    begin_request("dispatch", dispatch_payload(text), function(reply)
      if retry and string.sub(reply, 1, 5) == "error" then
        dispatch_form = "lua"
        hypr.dispatch(text)
      end
    end)
    return
  end
  local view = views.dispatch
  if not view then
    view = {
      process = io.process_view {
        command = { "hyprctl", "dispatch", "workspace", "1" },
        environment = CHILD_ENVIRONMENT,
      },
      buffer = "",
      busy = false,
    }
    views.dispatch = view
  end
  -- A finished child is only cleared by reading its exit event, and starting
  -- while one is still held is silently refused — so without this drain every
  -- workspace switch after the first would be dropped.
  if view.busy then
    for _ = 1, DRAIN_SLICES do
      local event = view.process:next(DRAIN_SLICE_MS)
      if not event then break end
      if event.kind == "exit" then
        view.busy = false
        break
      end
    end
  end
  if view.busy then return end
  local ok = pcall(function()
    view.process:set_command { "hyprctl", "dispatch", "workspace", text }
    view.process:start()
  end)
  view.busy = ok
end

-- -------------------------------------------------------------- accessors --

--- Restricts the reported workspaces to one output.
function hypr.set_monitor(name)
  if not name or name == "" then return end
  monitor_name = name
end

--- The output this shell follows, once one is known.
function hypr.monitor()
  return monitor_name
end

function hypr.rows()
  hypr.revision:get()
  return rows
end

function hypr.row(index)
  hypr.revision:get()
  return rows[index]
end

function hypr.active_index()
  hypr.revision:get()
  for index, row in ipairs(rows) do
    if row.active then return index end
  end
  return 1
end

function hypr.active_id()
  hypr.revision:get()
  for _, row in ipairs(rows) do
    if row.active then return row.id end
  end
  return 1
end

--- The row a workspace id occupies, if it is in the current block of ten.
function hypr.index_of(id)
  hypr.revision:get()
  for index, row in ipairs(rows) do
    if row.id == id then return index end
  end
  return nil
end

--- `barOnRight` from `Workspace.qml`: the main monitor keeps the ribbon on the
--- left, everything left of it mirrors to the right edge, and an explicit
--- `MOLD_BAR_SIDE` decides on its own.
function hypr.bar_on_right()
  if POSITION_MODE == "left" then return false end
  if POSITION_MODE == "right" then return true end
  if not monitor_name or monitor_name == MAIN_MONITOR then return false end
  local this = monitors[monitor_name]
  if not this then return false end
  local main = monitors[MAIN_MONITOR] or monitor_order[1]
  if not main then return false end
  return (this.x + this.width / 2) < (main.x + main.width / 2)
end

-- Kicked off at load so the first result is in hand before the ribbon draws.
hypr.refresh()
hypr.poll()

return hypr
