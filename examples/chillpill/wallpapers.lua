-- The wallpaper switcher.
--
-- A grid of the pictures in `wallpapersDir`, three across, a page at a
-- time so only nine are decoded at once. Arrows move, Enter or a click
-- sets the picture on every output through hyprpaper, and Escape closes.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local field = require("field")
local proc = require("proc")
local notify = require("notify")

local S = theme.S
local C = theme.color
local K = field.keys

local wallpapers = {}

local COLUMNS = 3
local PAGE_ROWS = 3
local PAGE = COLUMNS * PAGE_ROWS
local GAP = S(20)
local PAD = S(28)
local TILE_W = S(300)
local TILE_H = math.floor(TILE_W * 9 / 16)
local WIDTH = PAD * 2 + TILE_W * COLUMNS + GAP * (COLUMNS - 1)
local HEIGHT = PAD * 2 + TILE_H * PAGE_ROWS + GAP * (PAGE_ROWS - 1)

local files = {}
wallpapers.state = morf.state { selected = 1, page = 1, count = 0, error = "" }
local state = wallpapers.state
local shown = morf.list_model({})

local function name_of(path)
  return (path:match("([^/]+)$") or path):gsub("%.%w+$", "")
end

local function refill()
  local rows = {}
  local first = (state.page - 1) * PAGE + 1
  for index = first, math.min(#files, first + PAGE - 1) do
    rows[#rows + 1] = { id = files[index], path = files[index], name = name_of(files[index]), index = index }
  end
  shown:replace(rows, "id")
  state.count = #files
end

local function select(index)
  if #files == 0 then return end
  index = math.max(1, math.min(#files, index))
  state.selected = index
  local page = math.floor((index - 1) / PAGE) + 1
  if page ~= state.page then
    state.page = page
    refill()
  end
end

local function load()
  local dir = config.wallpapersDir
  proc.exec({ "sh", "-c",
    "find '" .. dir .. "' -maxdepth 1 -type f \\( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.webp' \\) | sort" },
    function(output, success)
      files = {}
      if success then
        for line in output:gmatch("[^\n]+") do files[#files + 1] = line end
      end
      state.error = #files == 0 and ("No pictures in " .. dir) or ""
      state.page = 1
      state.selected = 1
      refill()
    end)
end

local function apply(path)
  if not path then return end
  -- `reload ,<path>` is every output at once.
  proc.exec({ "hyprctl", "hyprpaper", "reload", "," .. path }, function(_, success)
    if success then
      notify.local_notice("Wallpaper", "Wallpaper changed", name_of(path), "󰸉")
    else
      notify.local_notice("Wallpaper", "hyprpaper did not answer", "Is it running?", "󰸉")
    end
  end)
  wallpapers.close()
end

local function handle_key(keysym)
  if keysym == K.ESCAPE then wallpapers.close() return end
  if keysym == K.RETURN or keysym == K.KP_ENTER then apply(files[state.selected]) return end
  if keysym == K.RIGHT then select(state.selected + 1) end
  if keysym == K.LEFT then select(state.selected - 1) end
  if keysym == K.DOWN then select(state.selected + COLUMNS) end
  if keysym == K.UP then select(state.selected - COLUMNS) end
  if keysym == K.PAGE_DOWN then select(state.selected + PAGE) end
  if keysym == K.PAGE_UP then select(state.selected - PAGE) end
end

local function tile(item)
  local selected = function() return item.index == state.selected end
  return ui.Item {
    width = TILE_W, height = TILE_H,
    ui.ClipRect {
      anchors = { fill = true }, radius = S(14),
      color = C.card,
      ui.Image {
        anchors = { fill = true },
        source = item.path,
        source_width = TILE_W * 2,
        fill_mode = "preserve_aspect_crop",
      },
    },
    ui.Rect {
      anchors = { fill = true }, radius = S(14), color = "transparent",
      border_width = function() return selected() and S(2) or 0 end,
      border_color = C.text,
    },
    theme.text {
      text = item.name, size = 14,
      anchors = { left = true, bottom = true, left_margin = S(14), bottom_margin = S(10) },
      visible = selected,
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_entered = function() state.selected = item.index end,
      on_clicked = function() apply(item.path) end,
    },
  }
end

local window = morf.window.layer {
  namespace = "chillpill-wallpapers",
  layer = "overlay",
  keyboard_focus = "exclusive",
  width = WIDTH,
  height = HEIGHT,
  visible = false,
  root = theme.box {
    width = WIDTH, height = HEIGHT, radius = 36,
    enter = { opacity = 0, scale = 0.98 },
    opacity = 1, scale = 1,
    behavior = { opacity = { duration = 120 }, scale = { duration = 160, easing = "out_quad" } },
    ui.Inset {
      margin = PAD,
      ui.Item {
        width = WIDTH - PAD * 2, height = HEIGHT - PAD * 2,
        ui.Repeater { as = "grid", columns = COLUMNS, gap = GAP, model = shown, delegate = tile },
        theme.text {
          size = 15, color = C.faint, anchors = { center_in = true },
          text = function() return state.error end,
          visible = function() return state.error ~= "" end,
        },
        ui.MouseArea {
          anchors = { fill = true },
          z = -1,
          on_wheel = function(_, _, _, _, _, steps_y)
            if steps_y ~= 0 then select(state.selected + steps_y * COLUMNS) end
          end,
        },
      },
    },
    ui.MouseArea {
      anchors = { fill = true },
      z = -2,
      on_key_pressed = function(keysym) handle_key(keysym) end,
    },
  },
}

wallpapers.open_signal = morf.signal("chillpill.wallpapers.open", false)

function wallpapers.open()
  load()
  window:open()
  wallpapers.open_signal:set(true)
end

function wallpapers.close()
  window:close()
  wallpapers.open_signal:set(false)
end

function wallpapers.toggle()
  if wallpapers.open_signal:get() then wallpapers.close() else wallpapers.open() end
  return wallpapers.open_signal:get()
end

return wallpapers
