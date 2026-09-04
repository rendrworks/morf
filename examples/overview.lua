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

local theme = morf.theme {
  ink = morf.color("#0d1015"):alpha(0.95),
  tile = "#1a1f28",
  text = "#e6e9ef",
  muted = "#8b93a5",
  -- The hover tint takes the desktop's accent when it has one.
  tile_hover = function(t)
    local accent = morf.prefers.accent_color
    return accent and t.tile:mix(accent, 0.25) or "#242b36"
  end,
}

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

-- One tile per window, keyed by the window's identifier. The Repeater is
-- the grid: `as = "grid"` lays the tiles out in rows of COLS, and when the
-- model is replaced with the current window list it adds, drops and moves
-- tiles by identity. A tile that stays keeps its node, and with it the
-- thumbnail it already has; a tile that goes releases its capture.
local tiles = {}
local model = morf.list_model({})

local heading = ui.Text {
  x = PAD, y = 14, width = W - PAD * 2,
  text = "windows", font_size = 15, color = theme.muted,
}

local function capture_name(identifier) return "tile-" .. identifier end


local function tile_for(item)
  -- Hover is a state that chooses itself: `when` reads the signal, and the
  -- transition eases the colour either way. No handler writes a colour.
  local hovered = morf.signal("overview.hover." .. item.identifier, false)
  local frame = ui.Rect {
    width = CELL_W, height = CELL_H, radius = 10, color = theme.tile,
    states = {
      default = { property_changes = { color = theme.tile } },
      hovered = {
        when = function() return hovered:get() end,
        property_changes = { color = theme.tile_hover },
      },
    },
    transitions = { { from = "*", to = "*", duration = 120, easing = "out_quad" } },
  }
  -- `source` is filled in from the capture. Until then it names nothing,
  -- which an Image treats as having nothing to draw.
  local shot = ui.Image {
    width = CELL_W, height = CELL_H,
    fill_mode = "preserve_aspect_fit", visible = false,
  }
  local caption = ui.Text {
    y = CELL_H + 6, width = CELL_W,
    text = "", font_size = 12, color = theme.text,
  }
  local tile = { frame = frame, shot = shot, caption = caption, identifier = item.identifier }
  local function describe(window)
    tile.title = window.title
    tile.app_id = window.app_id
    caption.text = (window.app_id ~= "" and window.app_id or "?") .. " · " .. window.title
  end
  describe(item)
  tiles[item.identifier] = tile
  -- Clicking claims the pointer over the tile, which an overview should: it
  -- is a thing you are looking at deliberately, and a click on a window in
  -- it means that window rather than whatever is behind the grid.
  local node = ui.MouseArea {
    width = CELL_W, height = CELL_H + LABEL,
    cursor = "pointer",
    -- A tile arrives a little small and clear: where its first frame
    -- starts, and the behaviors carry it to its place.
    enter = { opacity = 0, scale = 0.96 },
    behavior = {
      opacity = { duration = 180, easing = "out_quad" },
      scale = { kind = "spring", stiffness = 300, damping = 24 },
    },
    on_entered = function() hovered:set(true) end,
    on_exited = function() hovered:set(false) end,
    on_clicked = function()
      if tile.title then focus(tile.title, tile.app_id) end
    end,
    frame, shot, caption,
  }
  -- The updater: a window that changed its title keeps its tile and its
  -- picture, and only the caption is rewritten.
  return node, describe
end

local grid = ui.Repeater {
  as = "grid",
  x = PAD, y = 48,
  columns = COLS, row_gap = GAP, column_gap = GAP,
  model = model,
  delegate = tile_for,
}

--- Points the model at whatever the compositor currently reports.
local function relayout()
  local windows = morf.windows
  heading.text = #windows .. " window" .. (#windows == 1 and "" or "s")
  local rows, present = {}, {}
  for _, window in ipairs(windows) do
    rows[#rows + 1] = { identifier = window.identifier, title = window.title, app_id = window.app_id }
    present[window.identifier] = true
  end
  model:replace(rows, "identifier")
  for identifier in pairs(tiles) do
    if not present[identifier] then
      tiles[identifier] = nil
      morf.screencopy.release("gpu:capture/" .. capture_name(identifier))
    end
  end
end

--- Asks for a fresh picture of one tile.
---
--- One at a time, round-robin over the model's order. Capturing nine windows
--- at once asks the compositor for nine full-size copies in a frame, which
--- is a stutter for a grid nobody is watching that closely.
local next_index = 1
local function refresh()
  relayout()
  local count = model:len()
  if count == 0 then return end
  if next_index > count then next_index = 1 end
  local item = model:get(next_index)
  next_index = next_index + 1
  local tile = item and tiles[item.identifier]
  if not tile then return end
  local wanted = tile.identifier
  morf.screencopy.capture_window(wanted, function(frame, err)
    -- The window may have closed while the capture was in flight. Dropping
    -- a late picture is right: the alternative is a thumbnail under the
    -- wrong name.
    if err or tiles[wanted] ~= tile then return end
    tile.shot.source = frame.source
    tile.shot.visible = true
  end, { gpu = true, name = capture_name(wanted) })
end

ui.Item {
  width = W, height = H,
  ui.Rect { width = W, height = H, radius = 18, color = theme.ink },
  heading,
  grid,
  ui.Timer {
    interval = 700,
    ["repeat"] = true,
    running = true,
    on_triggered = refresh,
  },
}
