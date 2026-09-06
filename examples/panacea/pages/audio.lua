-- Sound: the output and its volume, the input, and every stream playing,
-- each with its own slider, over pactl.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
local system = require("system")
local proc = require("proc")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

page.title = "Sound"
page.icon = "󰕾"
page.subtitle = function()
  local n = page.sinks:len()
  for i = 1, n do
    local sink = page.sinks:get(i)
    if sink and sink.default then return sink.description end
  end
  return "Output, input, and what is playing"
end

page.sinks = morf.list_model({})
page.streams = morf.list_model({})
page.mic = morf.state { level = 0, muted = false, name = "" }

local function parse_volume(text)
  local percent = text:match("(%d+)%%")
  return percent and tonumber(percent) / 100 or 0
end

function page.refresh()
  proc.sh("pactl get-default-sink; echo --; pactl list sinks; echo --; pactl list sink-inputs; echo --; " ..
    "pactl get-default-source; echo --; pactl get-source-volume @DEFAULT_SOURCE@; pactl get-source-mute @DEFAULT_SOURCE@",
    function(output, success)
      if not success then return end
      local parts = proc.sections(output)
      local default_sink = proc.trim(parts[1] or "")
      -- Sinks.
      local sinks = {}
      for block in (parts[2] or ""):gmatch("Sink #(.-)\n\n") do
        local name = block:match("Name:%s*(%S+)")
        local description = block:match("Description:%s*([^\n]+)")
        local volume = block:match("Volume:[^\n]*")
        local muted = block:match("Mute:%s*(%a+)") == "yes"
        if name then
          sinks[#sinks + 1] = { id = name, name = name, description = description or name,
            level = parse_volume(volume or ""), muted = muted, default = name == default_sink }
        end
      end
      page.sinks:replace(sinks, "id")
      -- Streams.
      local streams = {}
      for block in ((parts[3] or "") .. "\n\n"):gmatch("Sink Input #(%d+)(.-)\n\n") do
        local index, rest = block, nil
      end
      for index, rest in ((parts[3] or "") .. "\n\n"):gmatch("Sink Input #(%d+)(.-)\n\n") do
        local app = rest:match('application%.name = "([^"]*)"') or rest:match('media%.name = "([^"]*)"') or ("Stream " .. index)
        local title = rest:match('media%.name = "([^"]*)"') or ""
        local volume = rest:match("Volume:[^\n]*")
        local muted = rest:match("Mute:%s*(%a+)") == "yes"
        streams[#streams + 1] = { id = index, index = index, app = app, title = title,
          level = parse_volume(volume or ""), muted = muted }
      end
      page.streams:replace(streams, "id")
      -- Microphone.
      page.mic.name = proc.trim(parts[4] or "")
      page.mic.level = parse_volume(parts[5] or "")
      page.mic.muted = (parts[5] or ""):match("Mute:%s*yes") ~= nil
    end)
end

function page.on_open()
  page.refresh()
end

morf.timer(2000, function()
  if require("island").page:get() == "audio" then page.refresh() end
end, true)

local function set_stream(index, level)
  proc.exec({ "pactl", "set-sink-input-volume", index, string.format("%d%%", math.floor(level * 100 + 0.5)) })
end

function page.build(island)
  local W = theme.page_w()
  local function sink_row(row)
    return kit.row {
      width = W, icon = "󰓃", title = row.description, subtitle = row.default and "In use" or "Available",
      active = function() return row.default end,
      on_click = function()
        proc.exec({ "pactl", "set-default-sink", row.name }, function() page.refresh() system.poll_volume() end)
      end,
    }
  end
  local function stream_row(row)
    local level = morf.signal("panacea.audio.stream." .. row.index, row.level)
    return kit.slider_card {
      width = W, title = row.app .. (row.title ~= "" and ("  ·  " .. row.title) or ""),
      value = function() return string.format("%d%%", math.floor(level:get() * 100 + 0.5)) end,
      icon = row.muted and "󰝟" or "󰕾", icon_color = row.muted and C.muted or C.fg,
      on_icon = function() proc.exec({ "pactl", "set-sink-input-mute", row.index, "toggle" }, page.refresh) end,
      fraction = function() return level:get() end,
      set = function(value) level:set(value) set_stream(row.index, value) end,
    }
  end
  return kit.page(W, {
    kit.slider_card {
      width = W, title = "Output",
      value = function() return string.format("%d%%", math.floor(state.volume.level * 100 + 0.5)) end,
      icon = function() return require("bar_glyphs").volume_glyph() end,
      icon_color = function() return state.volume.muted and C.muted or C.fg end,
      on_icon = system.toggle_mute,
      fraction = function() return state.volume.level end, set = system.set_volume,
    },
    kit.slider_card {
      width = W, title = "Input",
      value = function() return string.format("%d%%", math.floor(page.mic.level * 100 + 0.5)) end,
      icon = function() return page.mic.muted and "󰍭" or "󰍬" end,
      icon_color = function() return page.mic.muted and C.muted or C.fg end,
      on_icon = function() proc.exec({ "pactl", "set-source-mute", "@DEFAULT_SOURCE@", "toggle" }, page.refresh) end,
      fraction = function() return page.mic.level end,
      set = function(value)
        page.mic.level = value
        proc.exec({ "pactl", "set-source-volume", "@DEFAULT_SOURCE@", string.format("%d%%", math.floor(value * 100 + 0.5)) })
      end,
    },
    kit.section("Outputs", function() return page.sinks:len() > 1 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = page.sinks, delegate = sink_row,
      visible = function() return page.sinks:len() > 1 end },
    kit.section("Playing", function() return page.streams:len() > 0 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = page.streams, delegate = stream_row },
  })
end

return page
