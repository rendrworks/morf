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
  local properties = mold.dbus.proxy(
    "session",
    bus_name,
    "/org/mpris/MediaPlayer2",
    "org.freedesktop.DBus.Properties"
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
    watch = function(_, callback)
      properties:subscribe("PropertiesChanged", function(change)
        if change[1] == "org.mpris.MediaPlayer2.Player" then callback(change[2], change[3]) end
      end)
    end,
  }
end

return Mpris
