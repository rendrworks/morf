-- Sound: the output and its volume, the input, and every stream playing,
-- each with its own slider, over pactl.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
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
      local parts = {}
      for part in (output .. "\n--\n"):gmatch("(.-)\n%-%-\n") do parts[#parts + 1] = part end
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
  local function slider_row(values)
    local glyph, title, subtitle = values.glyph, values.title, values.subtitle
    return theme.card {
      width = W, height = S(64),
      ui.Column {
        gap = S(6),
        anchors = { left = true, top = true, margins = S(12) },
        ui.Item {
          width = W - S(24), height = S(18),
          theme.text { text = title, font_weight = 700, size = config.fontSize - 1, anchors = { left = true }, width = W - S(140), elide = "right" },
          theme.text { text = subtitle, size = config.fontSize - 5, color = C.muted, anchors = { right = true } },
        },
        ui.Row {
          gap = S(8), align = "center",
          ui.Item {
            width = S(22), height = S(22),
            theme.icon { text = glyph, size = config.iconSize - 3, anchors = { center_in = true },
              color = function() return values.muted() and C.muted or C.fg end },
            ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = values.toggle },
          },
          theme.slider { width = W - S(100), height = S(20), track = S(8), knob = S(14), color = C.fg,
            fraction = values.fraction, set = values.set },
          theme.text { text = function() return string.format("%d%%", math.floor(values.fraction() * 100 + 0.5)) end,
            size = config.fontSize - 4, color = C.muted },
        },
      },
    }
  end
  local function sink_row(row)
    return theme.button {
      width = W, height = S(40),
      color = function() return row.default and C.on_tint or C.card end,
      on_click = function()
        proc.exec({ "pactl", "set-default-sink", row.name }, function() page.refresh() system.poll_volume() end)
      end,
      ui.Row {
        gap = S(10), align = "center", height = S(40), anchors = { left = true, left_margin = S(12) },
        theme.icon { text = "󰓃", size = config.iconSize - 3, color = row.default and C.on or C.muted },
        theme.text { text = row.description, size = config.fontSize - 2, width = W - S(60), elide = "right" },
      },
    }
  end
  local function stream_row(row)
    local level = morf.signal("panacea.audio.stream." .. row.index, row.level)
    return slider_row {
      glyph = row.muted and "󰝟" or "󰕾",
      title = row.app, subtitle = row.title,
      muted = function() return row.muted end,
      toggle = function() proc.exec({ "pactl", "set-sink-input-mute", row.index, "toggle" }, page.refresh) end,
      fraction = function() return level:get() end,
      set = function(value) level:set(value) set_stream(row.index, value) end,
    }
  end
  return ui.Column {
    gap = S(8),
    slider_row {
      glyph = function() return require("bar_glyphs").volume_glyph() end,
      title = "Output", subtitle = function()
        local n = page.sinks:len()
        for i = 1, n do
          local sink = page.sinks:get(i)
          if sink and sink.default then return sink.description end
        end
        return ""
      end,
      muted = function() return state.volume.muted end,
      toggle = system.toggle_mute,
      fraction = function() return state.volume.level end,
      set = system.set_volume,
    },
    slider_row {
      glyph = function() return page.mic.muted and "󰍭" or "󰍬" end,
      title = "Input", subtitle = function() return page.mic.name end,
      muted = function() return page.mic.muted end,
      toggle = function() proc.exec({ "pactl", "set-source-mute", "@DEFAULT_SOURCE@", "toggle" }, page.refresh) end,
      fraction = function() return page.mic.level end,
      set = function(value)
        page.mic.level = value
        proc.exec({ "pactl", "set-source-volume", "@DEFAULT_SOURCE@", string.format("%d%%", math.floor(value * 100 + 0.5)) })
      end,
    },
    theme.label { text = "Outputs", visible = function() return page.sinks:len() > 1 end },
    ui.Repeater { as = "column", gap = S(4), model = page.sinks, delegate = sink_row,
      visible = function() return page.sinks:len() > 1 end },
    theme.label { text = "Playing", visible = function() return page.streams:len() > 0 end },
    ui.Repeater { as = "column", gap = S(6), model = page.streams, delegate = stream_row },
  }
end

return page
