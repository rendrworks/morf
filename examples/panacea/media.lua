-- The player, over MPRIS.
--
-- Every player on the session bus is `org.mpris.MediaPlayer2.<name>`, and the
-- one that is playing wins; failing that the first that has a track. The
-- proxy is read on a timer -- title, artist, art, status, length, position
-- -- and the position is carried forward between reads from the frame clock
-- so the bar under the title moves smoothly.

local morf = require("morf")
local core = require("morf.core")
local proc = require("proc")
local config = require("config")

local media = {}

media.state = morf.state {
  present = false,
  player = "",
  title = "",
  artist = "",
  album = "",
  art = "",          -- a local file, or ""
  status = "Stopped",
  length = 0,        -- seconds
  position = 0,      -- seconds, at the last read
  can_go_next = true,
  can_go_previous = true,
  revision = 0,      -- bumps when the track changes
}
local state = media.state

local MPRIS_PATH = "/org/mpris/MediaPlayer2"
local PLAYER = "org.mpris.MediaPlayer2.Player"

local bus = nil
do
  local ok, proxy = pcall(morf.dbus.proxy, "session", "org.freedesktop.DBus",
    "/org/freedesktop/DBus", "org.freedesktop.DBus", 1000)
  if ok then bus = proxy end
end

local proxies = {}
local function player_proxy(name)
  if proxies[name] == nil then
    local ok, proxy = pcall(morf.dbus.proxy, "session", name, MPRIS_PATH, PLAYER, 800)
    proxies[name] = ok and proxy or false
  end
  return proxies[name] or nil
end

local function property(proxy, name)
  local ok, value = pcall(proxy.get, proxy, name)
  if ok then return value end
  return nil
end

--- Every player on the bus.
---
--- A call's reply is the list of what the method returned, so the one
--- array `ListNames` gives back is `reply[1]`.
local function players()
  if not bus then return {} end
  local ok, reply = pcall(bus.call, bus, "ListNames")
  if not ok or type(reply) ~= "table" then return {} end
  local names = reply[1]
  if type(names) ~= "table" then return {} end
  local found = {}
  for _, name in ipairs(names) do
    if type(name) == "string" and name:sub(1, 23) == "org.mpris.MediaPlayer2." then
      found[#found + 1] = name
    end
  end
  table.sort(found)
  return found
end

-- ------------------------------------------------------------ album art --

local cache_dir = (core.env("XDG_CACHE_HOME") or (config.home .. "/.cache")) .. "/panacea/art"
local art_fetching = {}
local art_ready = {}
proc.sh("mkdir -p '" .. cache_dir .. "'")

local function hash(text)
  local h = 2166136261
  for index = 1, #text do
    h = (h ~ text:byte(index)) * 16777619 % 4294967296
  end
  return string.format("%08x", h)
end

--- A path for the art URL: a file URL as is, anything remote once curl
--- has fetched it into the cache.
local function art_path(url)
  if type(url) ~= "string" or url == "" then return "" end
  if url:sub(1, 7) == "file://" then return url:sub(8):gsub("%%20", " ") end
  if url:sub(1, 1) == "/" then return url end
  if url:sub(1, 4) == "http" then
    local path = cache_dir .. "/" .. hash(url)
    if art_ready[url] then return path end
    if not art_fetching[url] then
      art_fetching[url] = true
      proc.exec({ "curl", "-sL", "--max-time", "10", "-o", path, url }, function(_, success)
        art_fetching[url] = nil
        if success then
          art_ready[url] = true
          -- The track has not changed, but its picture has arrived.
          if state.art == "" then state.art = path end
        end
      end)
    end
    return ""
  end
  return ""
end

-- ---------------------------------------------------------------- polling --

local position_clock = morf.elapsed_timer()
local last_track = nil

local function read(name)
  local proxy = player_proxy(name)
  if not proxy then return false end
  local metadata = property(proxy, "Metadata")
  if type(metadata) ~= "table" then return false end
  local status = property(proxy, "PlaybackStatus") or "Stopped"
  local title = metadata["xesam:title"] or ""
  local artists = metadata["xesam:artist"]
  local artist = ""
  if type(artists) == "table" then
    artist = table.concat(artists, ", ")
  elseif type(artists) == "string" then
    artist = artists
  end
  local album = metadata["xesam:album"] or ""
  local length = tonumber(metadata["mpris:length"]) or 0
  local position = tonumber(property(proxy, "Position")) or 0
  local art = art_path(metadata["mpris:artUrl"])

  local track = name .. "|" .. title .. "|" .. artist
  if track ~= last_track then
    last_track = track
    state.revision = state.revision + 1
  end
  state.present = title ~= "" or status == "Playing"
  state.player = name
  state.title = title
  state.artist = artist
  state.album = album
  state.art = art
  state.status = status
  state.length = length / 1000000
  state.position = position / 1000000
  state.can_go_next = property(proxy, "CanGoNext") ~= false
  state.can_go_previous = property(proxy, "CanGoPrevious") ~= false
  position_clock:restart()
  return true
end

local function poll()
  local names = players()
  -- Playing beats paused beats anything else; among equals, keep the one
  -- shown, so a second player starting does not steal the card.
  local best, best_rank = nil, -1
  for _, name in ipairs(names) do
    local proxy = player_proxy(name)
    if proxy then
      local status = property(proxy, "PlaybackStatus")
      local rank = status == "Playing" and 3 or status == "Paused" and 2 or 1
      if name == state.player and rank == best_rank then rank = rank + 0.5 end
      if rank > best_rank then best, best_rank = name, rank end
    end
  end
  if not best or not read(best) then
    state.present = false
    state.player = ""
    state.status = "Stopped"
  end
end

poll()
morf.timer(1000, poll, true)
media.poll = poll

--- Where the track is now, in seconds, carried forward from the last read.
function media.position()
  if state.status ~= "Playing" then return state.position end
  return math.min(state.length > 0 and state.length or math.huge,
    state.position + position_clock:elapsed_ms() / 1000)
end

function media.progress()
  if state.length <= 0 then return 0 end
  return media.position() / state.length
end

--- `3:19`.
function media.clock(seconds)
  seconds = math.max(0, math.floor(tonumber(seconds) or 0))
  return string.format("%d:%02d", math.floor(seconds / 60), seconds % 60)
end

-- ------------------------------------------------------------------ verbs --

local function verb(name)
  return function()
    local proxy = state.player ~= "" and player_proxy(state.player) or nil
    if not proxy then return end
    pcall(proxy.call, proxy, name)
    morf.timer(150, poll, false)
  end
end

media.play_pause = verb("PlayPause")
media.next = verb("Next")
media.previous = verb("Previous")

--- Seeks to a fraction of the track.
function media.seek(fraction)
  local proxy = state.player ~= "" and player_proxy(state.player) or nil
  if not proxy or state.length <= 0 then return end
  local track_id = nil
  local metadata = property(proxy, "Metadata")
  if type(metadata) == "table" then track_id = metadata["mpris:trackid"] end
  if type(track_id) ~= "string" then return end
  local micros = math.floor(math.max(0, math.min(1, fraction)) * state.length * 1000000)
  pcall(proxy.call_with, proxy, "SetPosition", {
    { signature = "o", value = track_id },
    { signature = "x", value = micros },
  })
  state.position = micros / 1000000
  position_clock:restart()
  morf.timer(200, poll, false)
end

return media
