-- The screen recorder, over wf-recorder: frame rate, folder, whether the
-- system's sound and the microphone go in, start and stop.

local morf = require("morf")
local ui = require("morf.ui")
local io = require("morf.io")
local config = require("config")
local theme = require("theme")
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
  local function row(label, node)
    return ui.Row {
      gap = S(16), align = "center",
      ui.Item { width = S(70), height = S(30), theme.label { text = label, anchors = { left = true, top = true, top_margin = S(8) } } },
      node,
    }
  end
  local function switch(glyph, label, get, set)
    return theme.card {
      width = W, height = S(44),
      ui.Row {
        gap = S(10), align = "center", height = S(44),
        anchors = { left = true, left_margin = S(12) },
        theme.icon { text = glyph, size = config.iconSize - 2, color = C.muted },
        theme.text { text = label, size = config.fontSize - 1 },
      },
      ui.Item {
        anchors = { right = true, top = true, right_margin = S(10), top_margin = S(10) },
        theme.toggle(get, set),
      },
    }
  end
  return ui.Column {
    gap = S(12),
    row("FPS", theme.chips({ 30, 60, 120 }, function() return state.fps end, function(v) state.fps = v end)),
    row("Folder", theme.chips({
      { label = "~/Videos", value = config.home .. "/Videos" },
      { label = "~/Pictures", value = config.home .. "/Pictures" },
      { label = "~/Desktop", value = config.home .. "/Desktop" },
      { label = "~/", value = config.home },
    }, function() return state.folder end, function(v) state.folder = v end)),
    switch("󰕾", "System audio", function() return state.audio end, function(v) state.audio = v end),
    switch("󰍬", "Microphone", function() return state.mic end, function(v) state.mic = v end),
    theme.button {
      width = W, height = S(46),
      color = function() return state.running and C.card or C.crit_tint end,
      hover_color = function() return state.running and C.card_hover or C.crit:alpha(0.32) end,
      on_click = function()
        if state.running then page.stop() else page.start() end
        island.close()
      end,
      ui.Row {
        gap = S(10), align = "center", height = S(46), anchors = { center_in = true },
        ui.Rect { width = S(10), height = S(10), radius = S(5), color = C.crit },
        theme.text { text = function() return state.running and "Stop recording" or "Start recording" end, font_weight = 700 },
      },
    },
    theme.button {
      width = W, height = S(44),
      on_click = function()
        require("system").launch({ "xdg-open", state.folder })
        island.close()
      end,
      ui.Row {
        gap = S(10), align = "center", height = S(44), anchors = { center_in = true },
        theme.icon { text = "󰉋", size = config.iconSize - 2 },
        theme.text { text = "Open recordings folder", font_weight = 700, size = config.fontSize - 1 },
      },
    },
  }
end

return page
