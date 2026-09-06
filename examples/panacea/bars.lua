-- The equaliser: bars that follow the sound.
--
-- Levels come from cava when it is installed -- a real spectrum, one bar
-- per band -- and otherwise from pw-record (or parec) reading the output's
-- monitor, which gives one loudness that the bars share with a little
-- spread between them, so the picture still moves with the music. Either
-- way the numbers arrive as lines on a child's stdout and land in one list
-- of signals.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local io = require("morf.io")
local config = require("config")
local theme = require("theme")
local proc = require("proc")

local S = theme.S
local C = theme.color

local bars = {}

bars.COUNT = 24
bars.source = morf.signal("panacea.bars.source", "")
bars.levels = {}
for index = 1, bars.COUNT do
  bars.levels[index] = morf.signal("panacea.bars." .. index, 0)
end

local cache_dir = (core.env("XDG_CACHE_HOME") or (config.home .. "/.cache")) .. "/panacea"

local function set_levels(values)
  for index = 1, bars.COUNT do
    local value = values[index] or 0
    bars.levels[index]:set(math.max(0, math.min(1, value)))
  end
end

--- One line from cava: `12;40;55;...;` at ascii_max_range 100.
local function on_cava_line(line)
  local values = {}
  for number in line:gmatch("(%d+);") do values[#values + 1] = tonumber(number) / 100 end
  if #values > 0 then set_levels(values) end
end

-- The loudness from parec: one number a line, smoothed, spread across the
-- bars by a fixed shape so the middle is tallest and the ends quieter.
local shape = {}
for index = 1, bars.COUNT do
  local t = (index - 0.5) / bars.COUNT
  shape[index] = 0.35 + 0.65 * math.sin(t * math.pi) * (0.8 + 0.2 * math.sin(index * 1.7))
end
local smoothed = 0
-- The loudest recent moment, decaying, so quiet music still fills the
-- bars: a level is loudness over that peak rather than over full scale.
local peak = 0.02

local function on_level_line(line)
  local total, count = 0, 0
  for number in line:gmatch("%d+") do
    local sample = tonumber(number) - 128
    total = total + sample * sample
    count = count + 1
  end
  if count == 0 then return end
  local rms = math.sqrt(total / count) / 128
  peak = math.max(peak * 0.992, rms, 0.005)
  local level = math.min(1, rms / peak)
  smoothed = smoothed + (level - smoothed) * 0.5
  local values = {}
  for index = 1, bars.COUNT do
    values[index] = smoothed * shape[index] * (0.85 + 0.3 * math.random())
  end
  set_levels(values)
end

local started = false

--- Starts whichever source there is, once.
function bars.start()
  if started then return end
  started = true
  proc.sh("mkdir -p '" .. cache_dir .. "'; command -v cava >/dev/null && echo cava || "
    .. "(command -v pw-record >/dev/null && echo pw-record) || (command -v parec >/dev/null && echo parec)",
    function(output)
      local which = proc.trim(output)
      if which == "cava" then
        local conf = cache_dir .. "/cava.conf"
        local ok, file = pcall(io.file, conf)
        if ok then
          pcall(file.write, file, table.concat({
            "[general]", "bars = " .. bars.COUNT, "framerate = 30", "autosens = 1",
            "[output]", "method = raw", "raw_target = /dev/stdout", "data_format = ascii",
            "ascii_max_range = 100", "bar_delimiter = 59", "frame_delimiter = 10",
            "[smoothing]", "noise_reduction = 60", "",
          }, "\n"))
        end
        proc.stream({ "cava", "-p", conf }, on_cava_line, { retry_ms = 10000 })
        bars.source:set("cava")
      elseif which == "pw-record" then
        proc.stream({ "sh", "-c",
          "pw-record --raw --rate 8000 --channels 1 --format u8 -P '{ stream.capture.sink=true }' - 2>/dev/null"
          .. " | od -An -v -tu1 -w320" },
          on_level_line, { retry_ms = 10000 })
        bars.source:set("pw-record")
      elseif which == "parec" then
        proc.stream({ "sh", "-c",
          "parec --raw --format=u8 --rate=8000 --channels=1 -d @DEFAULT_MONITOR@ 2>/dev/null | od -An -v -tu1 -w320" },
          on_level_line, { retry_ms = 10000 })
        bars.source:set("parec")
      else
        bars.source:set("")
      end
    end)
end

--- The bars, `width` wide and `height` tall, coloured `color`. Reads the
--- levels; `playing()` says whether to show anything at all, and
--- `progress()` and `seek(fraction)` make the whole strip a scrubber: the
--- bars left of the position are lit, the rest dim.
function bars.build(values)
  local width, height = values.width, values.height
  local count = bars.COUNT
  local gap = S(3)
  local bar_w = (width - gap * (count - 1)) / count
  local nodes = {}
  for index = 1, count do
    local x = (index - 1) * (bar_w + gap)
    nodes[index] = ui.Rect {
      x = x,
      width = bar_w,
      radius = bar_w / 2,
      y = function()
        local level = values.playing() and bars.levels[index]:get() or 0.06
        return height - math.max(S(3), level * height)
      end,
      height = function()
        local level = values.playing() and bars.levels[index]:get() or 0.06
        return math.max(S(3), level * height)
      end,
      color = function()
        local colour = values.color or C.on
        if not values.progress then return colour end
        local lit = (x + bar_w / 2) / width <= values.progress()
        return lit and colour or colour:alpha(0.3)
      end,
      behavior = { y = { duration = 60, easing = "out_quad" }, height = { duration = 60, easing = "out_quad" }, color = { duration = 120 } },
    }
  end
  nodes[#nodes + 1] = ui.MouseArea {
    anchors = { fill = true },
    cursor = values.seek and "pointer" or "default",
    on_pressed = function(_, _, local_x) if values.seek then values.seek(local_x / width) end end,
    on_dragged = function(_, _, _, _, local_x) if values.seek then values.seek(local_x / width) end end,
  }
  return ui.Item { width = width, height = height, table.unpack(nodes) }
end

return bars
