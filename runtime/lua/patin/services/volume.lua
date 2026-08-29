local mold = require("mold")

local Volume = {}

function Volume.new()
  local pipewire = mold.pipewire.connect()
  local sink
  for _, node in ipairs(pipewire:nodes()) do
    if node.media_class == "Audio/Sink" then
      sink = node
      break
    end
  end
  assert(sink, "PipeWire has no audio sink")

  return {
    node = sink,
    level = function() return pipewire:volume(sink.id).level end,
    muted = function() return pipewire:volume(sink.id).muted end,
    set_level = function(_, level)
      local volume = pipewire:volume(sink.id)
      pipewire:set_volume(sink.id, level, volume.muted)
    end,
    set_muted = function(_, muted)
      local volume = pipewire:volume(sink.id)
      pipewire:set_volume(sink.id, volume.level, muted)
    end,
  }
end

return Volume
