local mold = require("mold")

local Mpris = {}

function Mpris.new(bus_name)
  assert(bus_name and bus_name ~= "", "MPRIS bus name is required")
  local player = mold.dbus.proxy(
    "session",
    bus_name,
    "/org/mpris/MediaPlayer2",
    "org.mpris.MediaPlayer2.Player"
  )
  return {
    playback_status = function() return player:get("PlaybackStatus") end,
    position = function() return player:get("Position") end,
    volume = function() return player:get("Volume") end,
    set_volume = function(_, value) return player:set("Volume", value) end,
    play_pause = function() return player:call("PlayPause") end,
    play = function() return player:call("Play") end,
    pause = function() return player:call("Pause") end,
    next = function() return player:call("Next") end,
    previous = function() return player:call("Previous") end,
  }
end

return Mpris
