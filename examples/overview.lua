-- Every window on the machine, as a live thumbnail.
--
--     oslo make run --example examples/overview.lua
--
-- Two protocols meeting. `ext-foreign-toplevel-list-v1` says what windows
-- exist — `morf.windows`, each with a title, an application and a stable
-- identifier. `ext-image-copy-capture-v1` turns one of those identifiers into
-- pixels. Neither is much use alone: a list with no pictures is a menu, and
-- pictures with no list is a screenshot.
--
-- The older `wlr-screencopy` cannot do this at all. It captures *outputs*, and
-- a window cannot be got out of an output capture — cropping to its rectangle
-- gives whatever is on top there, which is frequently some other window. That
-- is the whole reason the newer protocol is worth the extra negotiation.
--
-- Needs a compositor with `ext-image-copy-capture-v1` and the toplevel source:
-- KDE, niri, COSMIC, Hyprland. `wayland-smoke` prints `capture ext+window`
-- where both are present.
--
-- Clicking a tile focuses that window, and that part is *not* portable. Neither
-- capture protocol can focus or close anything — they describe and they copy,
-- and acting on a window is the compositor's own business. So the click goes
-- out over Hyprland's control socket, and on another compositor it does
-- nothing.
--
-- Worse, the two do not even agree on what a window is called: the protocol
-- reports identifiers like `18000003` and Hyprland addresses like
-- `0x55a0efd56080`, and there is no mapping between them. So the click looks
-- the window up by title and application, which is approximate by nature —
-- two windows with the same title in the same application are genuinely
-- indistinguishable from out here.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local io = require("morf.io")

local screen = morf.screens[1]
local SW = (screen and screen.width) or 1920
local SH = (screen and screen.height) or 1080

local W = math.min(SW - 160, 1600)
local H = math.min(SH - 160, 1000)

morf.surface.width = W
morf.surface.height = H
morf.surface.layer = "overlay"
morf.surface.anchors = { top = true }
morf.surface.margin_top = 80
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1

local COLS = 3
local GAP = 20
local PAD = 28
local LABEL = 26
local CELL_W = math.floor((W - PAD * 2 - GAP * (COLS - 1)) / COLS)
local CELL_H = math.floor(CELL_W * 0.62)

local INK = "#0d1015f2"
local TILE = "#1a1f28"
local TEXT = "#e6e9ef"
local MUTED = "#8b93a5"

-- One tile per grid slot, built once and re-pointed as windows come and go.
-- Rebuilding the tree each refresh would throw away the thumbnails with it,
-- and a grid that flickers every second is worse than one that lags a frame.
--- One request on Hyprland's control socket, and its reply.
---
--- The socket answers once and closes, so this connects each time. That is
--- cheaper than it reads: a hundred of these round trips take a millisecond and
--- a half, and none of it is a process.
local SIGNATURE = core.env("HYPRLAND_INSTANCE_SIGNATURE")
local RUNTIME = core.env("XDG_RUNTIME_DIR")

local function hypr(command)
  if not (SIGNATURE and RUNTIME) then return nil end
  local ok, socket = pcall(io.socket, RUNTIME .. "/hypr/" .. SIGNATURE .. "/.socket.sock")
  if not (ok and socket) then return nil end
  socket:send(command)
  socket:flush()
  local reply = socket:receive(65536, 20)
  socket:close()
  return reply
end

--- Focuses the window a tile is showing, if the compositor can be asked.
local function focus(title, app_id)
  local listing = hypr("j/clients")
  if not listing then return end
  local ok, clients = pcall(io.json.decode, listing)
  if not (ok and clients) then return end
  for _, client in ipairs(clients) do
    if client.title == title and client.class == app_id then
      hypr("dispatch focuswindow address:" .. client.address)
      return
    end
  end
end

local tiles = {}
local children = { width = W, height = H }
children[#children + 1] = ui.Rect { width = W, height = H, radius = 18, color = INK }

local heading = ui.Text {
  x = PAD, y = 14, width = W - PAD * 2,
  text = "windows", font_size = 15, color = MUTED,
}
children[#children + 1] = heading

local SLOTS = COLS * 3
for slot = 1, SLOTS do
  local column = (slot - 1) % COLS
  local row = math.floor((slot - 1) / COLS)
  local x = PAD + column * (CELL_W + GAP)
  local y = 48 + row * (CELL_H + LABEL + GAP)

  local frame = ui.Rect {
    x = x, y = y, width = CELL_W, height = CELL_H,
    radius = 10, color = TILE, visible = false,
  }
  -- `source` is filled in from the capture. Until then it names nothing, which
  -- an Image treats as having nothing to draw.
  local shot = ui.Image {
    x = x, y = y, width = CELL_W, height = CELL_H,
    fill_mode = "preserve_aspect_fit", visible = false,
  }
  local caption = ui.Text {
    x = x, y = y + CELL_H + 6, width = CELL_W,
    text = "", font_size = 12, color = TEXT, visible = false,
  }
  local tile = { frame = frame, shot = shot, caption = caption, identifier = nil }
  -- Clicking claims the pointer over the tile, which an overview should: it is
  -- a thing you are looking at deliberately, and a click on a window in it
  -- means that window rather than whatever is behind the grid.
  local hit = ui.MouseArea {
    x = x, y = y, width = CELL_W, height = CELL_H + LABEL,
    on_clicked = function()
      if tile.title then focus(tile.title, tile.app_id) end
    end,
  }
  tiles[slot] = tile
  children[#children + 1] = frame
  children[#children + 1] = shot
  children[#children + 1] = caption
  children[#children + 1] = hit
end

--- Points the tiles at whatever the compositor currently reports.
local function relayout()
  local windows = morf.windows
  heading.text = #windows .. " window" .. (#windows == 1 and "" or "s")
  for slot, tile in ipairs(tiles) do
    local window = windows[slot]
    local shown = window ~= nil
    tile.frame.visible = shown
    tile.caption.visible = shown
    if shown then
      tile.title = window.title
      tile.app_id = window.app_id
      tile.caption.text = (window.app_id ~= "" and window.app_id or "?")
        .. " · " .. window.title
      -- A tile that changed which window it holds must drop the old picture,
      -- or it shows the wrong window until the next capture lands.
      if tile.identifier ~= window.identifier then
        tile.identifier = window.identifier
        tile.shot.visible = false
      end
    else
      tile.identifier = nil
      tile.title = nil
      tile.app_id = nil
      tile.shot.visible = false
    end
  end
end

--- Asks for a fresh picture of one tile.
---
--- One at a time, round-robin. Capturing nine windows at once asks the
--- compositor for nine full-size copies in a frame, which is a stutter for a
--- grid nobody is watching that closely.
local next_slot = 1
local function refresh()
  relayout()
  local scanned = 0
  while scanned < #tiles do
    local tile = tiles[next_slot]
    next_slot = next_slot % #tiles + 1
    scanned = scanned + 1
    if tile.identifier then
      local wanted = tile.identifier
      morf.screencopy.capture_window(wanted, function(frame, err)
        -- The window may have closed, or moved to another tile, while the
        -- capture was in flight. Dropping a late picture is right: the
        -- alternative is a thumbnail of one window under another's name.
        if err or tile.identifier ~= wanted then return end
        tile.shot.source = frame.source
        tile.shot.visible = true
      end)
      return
    end
  end
end

children[#children + 1] = ui.Timer {
  interval = 700,
  ["repeat"] = true,
  running = true,
  on_triggered = refresh,
}

ui.Item(children)
