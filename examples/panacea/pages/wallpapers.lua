-- Wallpapers: a carousel of the pictures in the wallpaper folder, the
-- chosen one large in the middle and its neighbours behind it, set with
-- Enter or a click through hyprpaper.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local proc = require("proc")
local notify = require("notify")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Wallpapers"
page.icon = "󰸉"

local dirs = {
  config.expand(config.wallpaperDir or ""),
  config.home .. "/.config/hypr/wallpaper",
  config.home .. "/Pictures/wallpapers",
  config.home .. "/Pictures/Wallpapers",
}

local files = {}
page.state = morf.state { index = 1, count = 0, name = "", dir = "" }
local state = page.state
local shown = morf.list_model({})

page.subtitle = function()
  if state.count == 0 then return "No pictures in " .. (state.dir ~= "" and state.dir or dirs[2]) end
  return state.name .. "  ·  " .. state.index .. " / " .. state.count
end

function page.width() return theme.panel_w(1.6) end

local function name_of(path)
  return (path:match("([^/]+)$") or path):gsub("%.%w+$", "")
end

-- The five around the chosen one: two each side.
local function refill()
  local rows = {}
  for offset = -2, 2 do
    local index = state.index + offset
    if index >= 1 and index <= #files then
      rows[#rows + 1] = { id = files[index], path = files[index], offset = offset, name = name_of(files[index]) }
    end
  end
  shown:replace(rows, "id")
  state.count = #files
  state.name = files[state.index] and name_of(files[state.index]) or ""
end

local function select(index)
  if #files == 0 then return end
  state.index = math.max(1, math.min(#files, index))
  refill()
end

--- Reads the first folder that has pictures in it.
function page.load()
  local wanted = {}
  for _, dir in ipairs(dirs) do
    if dir ~= "" then wanted[#wanted + 1] = "'" .. dir .. "'" end
  end
  proc.exec({ "sh", "-c",
    "for d in " .. table.concat(wanted, " ") .. "; do [ -d \"$d\" ] || continue; "
    .. "found=$(find \"$d\" -maxdepth 1 -type f \\( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.webp' \\) | sort); "
    .. "[ -n \"$found\" ] && { echo \"DIR $d\"; echo \"$found\"; break; }; done" },
    function(output)
      files = {}
      for line in output:gmatch("[^\n]+") do
        local dir = line:match("^DIR (.*)$")
        if dir then state.dir = dir else files[#files + 1] = line end
      end
      state.index = math.min(math.max(1, state.index), math.max(1, #files))
      refill()
    end)
end

local function apply(path)
  if not path then return end
  proc.exec({ "hyprctl", "hyprpaper", "reload", "," .. path }, function(_, success)
    if success then
      notify.local_notice("Wallpaper", name_of(path), "Set on every output", "󰸉")
    else
      notify.local_notice("Wallpaper", "hyprpaper did not answer", "Is it running?", "󰸉")
    end
  end)
  require("island").close()
end

function page.on_open() page.load() end

function page.on_key(keysym)
  if keysym == 0xff53 then select(state.index + 1) return true end
  if keysym == 0xff51 then select(state.index - 1) return true end
  if keysym == 0xff0d or keysym == 0xff8d then apply(files[state.index]) return true end
  return false
end

function page.build(island)
  local W = page.width() - S(32)
  local H = S(230)
  local BIG_W, BIG_H = S(360), S(202)
  local SMALL_W, SMALL_H = S(220), S(124)
  local function tile(item)
    local centre = item.offset == 0
    local width = centre and BIG_W or SMALL_W
    local height = centre and BIG_H or SMALL_H
    -- The neighbours sit behind the middle one, further out and smaller,
    -- which is the parallax: the row slides as the choice moves.
    local x = W / 2 - width / 2 + item.offset * S(190)
    return ui.Item {
      width = width, height = height,
      x = x, y = (H - height) / 2,
      z = centre and 2 or (1 - math.abs(item.offset)),
      opacity = centre and 1 or (item.offset == 1 or item.offset == -1) and 0.7 or 0.35,
      enter = { opacity = 0, scale = 0.8 },
      behavior = { x = theme.motion.move, y = theme.motion.move, width = theme.motion.move, height = theme.motion.move,
        opacity = theme.motion.fade },
      ui.ClipRect {
        anchors = { fill = true }, radius = S(14), color = C.card,
        ui.Image { anchors = { fill = true }, source = item.path, source_width = BIG_W * 2, fill_mode = "preserve_aspect_crop" },
      },
      ui.Rect {
        anchors = { fill = true }, radius = S(14), color = "transparent",
        border_width = centre and S(2) or 0, border_color = C.fg:alpha(0.8),
      },
      ui.MouseArea {
        anchors = { fill = true }, cursor = "pointer",
        on_clicked = function()
          if centre then apply(item.path) else select(state.index + item.offset) end
        end,
      },
    }
  end
  return ui.Column {
    gap = S(8),
    ui.Item {
      width = W, height = H,
      ui.Repeater { model = shown, delegate = tile },
      ui.MouseArea {
        anchors = { fill = true }, z = -1,
        on_wheel = function(_, _, _, _, _, steps_y)
          if steps_y ~= 0 then select(state.index + steps_y) end
        end,
      },
    },
    ui.Item {
      width = W, height = S(20),
      theme.text { text = "← → to browse, Enter or a click to set", size = config.fontSize - 4, color = C.faint,
        anchors = { center_in = true } },
    },
  }
end

return page
