-- Child processes, drained on a tick.
--
-- A process view has to exist before the service loop starts, so this keeps
-- a pool of them from module load and hands one to each command as it comes.
-- Output is collected from `proc.tick()`, which `init.lua` runs on a short
-- timer; a command that finishes calls back with its stdout and whether it
-- succeeded. Long-lived commands that print a line at a time -- `pactl
-- subscribe`, `cliphist` on a pipe -- go through `proc.stream`.
--
-- `LD_LIBRARY_PATH` is cleared for every child. morf may be running under a
-- nixGL-style wrapper that points it at store paths, and a system binary
-- that inherits those fails to load its own libraries.

local io = require("morf.io")

local proc = {}

local CHILD_ENVIRONMENT = { LD_LIBRARY_PATH = "" }
local POOL_SIZE = 16
local DRAIN_SLICE_MS = 1
local DRAIN_SLICES = 8

local pool = {}
local queue = {}
local streams = {}

for index = 1, POOL_SIZE do
  pool[index] = {
    process = io.process_view { command = { "true" }, environment = CHILD_ENVIRONMENT },
    busy = false,
    buffer = "",
    on_done = nil,
  }
end

local function start(slot, command, on_done)
  slot.buffer = ""
  slot.on_done = on_done
  local ok = pcall(function()
    slot.process:set_command(command)
    slot.process:start()
  end)
  slot.busy = ok
  if not ok and on_done then on_done("", false) end
end

--- Runs `command` (a list) and calls `on_done(stdout, success)` when it
--- exits. Commands beyond the pool wait their turn.
function proc.exec(command, on_done)
  for _, slot in ipairs(pool) do
    if not slot.busy then
      start(slot, command, on_done)
      return
    end
  end
  queue[#queue + 1] = { command = command, on_done = on_done }
end

--- Runs a shell line through `sh -c`.
function proc.sh(line, on_done)
  proc.exec({ "sh", "-c", line }, on_done)
end

--- Keeps `command` running and calls `on_line(line)` for each line it
--- prints. Restarted a while after it exits, since the thing it watches
--- may have gone away and come back.
function proc.stream(command, on_line, options)
  options = options or {}
  local stream = {
    process = io.process_view { command = command, environment = CHILD_ENVIRONMENT },
    command = command,
    on_line = on_line,
    buffer = "",
    running = false,
    retry_ms = options.retry_ms or 5000,
    down_since = nil,
    clock = require("morf").elapsed_timer(),
  }
  streams[#streams + 1] = stream
  local ok = pcall(function()
    stream.process:set_command(command)
    stream.process:start()
  end)
  stream.running = ok
  if not ok then stream.down_since = stream.clock:elapsed_ms() end
  return stream
end

local function drain_slot(slot)
  for _ = 1, DRAIN_SLICES do
    local event = slot.process:next(DRAIN_SLICE_MS)
    if not event then return end
    if event.kind == "stdout" then
      slot.buffer = slot.buffer .. (event.data or "")
    elseif event.kind == "exit" then
      slot.busy = false
      local on_done, buffer = slot.on_done, slot.buffer
      slot.on_done, slot.buffer = nil, ""
      if on_done then on_done(buffer, event.success) end
      local next_job = table.remove(queue, 1)
      if next_job then start(slot, next_job.command, next_job.on_done) end
      return
    end
  end
end

local function drain_stream(stream)
  if not stream.running then
    if stream.down_since and stream.clock:elapsed_ms() - stream.down_since >= stream.retry_ms then
      stream.down_since = nil
      local ok = pcall(function()
        stream.process:set_command(stream.command)
        stream.process:start()
      end)
      stream.running = ok
      if not ok then stream.down_since = stream.clock:elapsed_ms() end
    end
    return
  end
  for _ = 1, DRAIN_SLICES do
    local event = stream.process:next(DRAIN_SLICE_MS)
    if not event then return end
    if event.kind == "stdout" then
      stream.buffer = stream.buffer .. (event.data or "")
      while true do
        local line, rest = stream.buffer:match("^(.-)\n(.*)$")
        if not line then break end
        stream.buffer = rest
        stream.on_line(line)
      end
    elseif event.kind == "exit" then
      stream.running = false
      stream.down_since = stream.clock:elapsed_ms()
      return
    end
  end
end

--- Collects what every running command has printed. Cheap when idle: a
--- slot that is not busy is not read.
function proc.tick()
  for _, slot in ipairs(pool) do
    if slot.busy then drain_slot(slot) end
  end
  for _, stream in ipairs(streams) do drain_stream(stream) end
end

--- Whitespace off both ends.
function proc.trim(text)
  return (tostring(text or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

return proc
