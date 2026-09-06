-- The screen recorder, over wf-recorder: frame rate, folder, whether the
-- system's sound and the microphone go in, start and stop.

local morf = require("morf")
local ui = require("morf.ui")
local io = require("morf.io")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
local proc = require("proc")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Screen recording"
page.icon = "󰑊"

page.state = morf.state {
  running = false,
  elapsed = "00:00",
  file = "",
  fps = config.recordFps,
  folder = config.recordDir,
  audio = false,
  mic = false,
  available = true,
}
local state = page.state

local process = nil
local started = morf.elapsed_timer()

local function tick()
  if not state.running then return end
  local seconds = math.floor(started:elapsed_ms() / 1000)
  state.elapsed = string.format("%02d:%02d", math.floor(seconds / 60), seconds % 60)
  require("island").recording:set(state.elapsed)
end
morf.timer(500, tick, true)

function page.start()
  if state.running then return end
  local name = "panacea-" .. theme.clock:format("%Y%m%d-%H%M%S") .. ".mp4"
  local path = state.folder .. "/" .. name
  local command = { "wf-recorder", "-f", path, "-r", tostring(state.fps) }
  if state.audio then
    command[#command + 1] = "--audio"
  end
  if state.mic then
    -- wf-recorder takes one audio source; the microphone wins when both
    -- are asked for, since it is the one people forget to switch on.
    command[#command + 1] = "--audio=@DEFAULT_AUDIO_SOURCE@"
  end
  local ok, view = pcall(io.process_view, { command = command, environment = { LD_LIBRARY_PATH = "" } })
  if not ok then
    state.available = false
    return
  end
  proc.sh("mkdir -p '" .. state.folder .. "'")
  local began = pcall(view.start, view)
  if not began then
    state.available = false
    return
  end
  process = view
  state.file = name
  state.running = true
  started:restart()
  state.elapsed = "00:00"
  require("island").recording:set("00:00")
end

function page.stop()
  if not state.running then return end
  -- SIGINT ends the file cleanly; `kill` is what the view offers, and
  -- wf-recorder closes its file on it as well.
  if process then pcall(process.kill, process) end
  process = nil
  state.running = false
  require("island").recording:set("")
  require("notify").local_notice("Recorder", "Recording saved", state.file, "󰑊")
end

page.subtitle = function()
  if not state.available then return "wf-recorder is not installed" end
  if state.running then return "Recording · " .. state.elapsed .. " · " .. state.file end
  return "Ready to record"
end

function page.build(island)
  local W = theme.page_w()
  return kit.page(W, {
    kit.choice_row { width = W, title = "Frame rate",
      options = { 30, 60, 120 }, current = function() return state.fps end, choose = function(v) state.fps = v end },
    kit.choice_row { width = W, title = "Folder",
      options = {
        { label = "Videos", value = config.home .. "/Videos" },
        { label = "Pictures", value = config.home .. "/Pictures" },
        { label = "Desktop", value = config.home .. "/Desktop" },
        { label = "Home", value = config.home },
      }, current = function() return state.folder end, choose = function(v) state.folder = v end },
    kit.switch_row { width = W, icon = "󰕾", title = "System audio", subtitle = "What the speakers play",
      on = function() return state.audio end, set = function(v) state.audio = v end },
    kit.switch_row { width = W, icon = "󰍬", title = "Microphone", subtitle = "Your voice over it",
      on = function() return state.mic end, set = function(v) state.mic = v end },
    kit.action {
      width = W, kind = "danger",
      icon = function() return state.running and "󰓛" or "󰑊" end,
      label = function() return state.running and "Stop recording" or "Start recording" end,
      on_click = function()
        if state.running then page.stop() else page.start() end
        island.close()
      end,
    },
    kit.action { width = W, icon = "󰉋", label = "Open recordings folder",
      on_click = function() system.launch("xdg-open '" .. state.folder .. "'") end },
  })
end

return page
