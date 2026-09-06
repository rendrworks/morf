-- The clipboard manager, over cliphist.
--
-- `cliphist list` is one line per entry, the id and a preview separated by
-- a tab; `cliphist decode <id>` gives the entry back, which `wl-copy` puts
-- on the clipboard. An image entry is previewed by decoding it into the
-- cache once it is selected. Same keys as the launcher.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local field = require("field")
local proc = require("proc")

local S = theme.S
local C = theme.color
local K = field.keys

local cliphist = {}

local WIDTH = S(720)
local PAD = S(24)
local INNER = WIDTH - PAD * 2
local ROW_HEIGHT = S(56)
local ROWS = 8
local THUMB = S(140)

local cache_dir = (core.env("XDG_CACHE_HOME") or (config.home .. "/.cache")) .. "/chillpill/clips"
proc.sh("mkdir -p '" .. cache_dir .. "'")

-- --------------------------------------------------------------- entries --

local entries = {}
cliphist.state = morf.state { selected = 1, first = 1, count = 0, total = 0, error = "", thumb = "" }
local state = cliphist.state
local shown = morf.list_model({})
local matches = entries

local function is_image(preview)
  return preview:match("^%[%[ binary data") ~= nil
end

local function refill()
  local rows = {}
  local last = math.min(#matches, state.first + ROWS - 1)
  for index = state.first, last do
    local entry = matches[index]
    rows[#rows + 1] = { id = entry.id, preview = entry.preview, image = entry.image, index = index }
  end
  shown:replace(rows, "id")
  state.count = #matches
  state.total = #entries
end

local thumbs = {}

--- Decodes an image entry into the cache, once, and shows it.
local function preview_image(entry)
  if not entry or not entry.image then
    state.thumb = ""
    return
  end
  local path = cache_dir .. "/" .. entry.id .. ".png"
  if thumbs[entry.id] then
    state.thumb = path
    return
  end
  proc.sh("cliphist decode " .. entry.id .. " > '" .. path .. "'", function(_, success)
    if success then
      thumbs[entry.id] = true
      if matches[state.selected] == entry then state.thumb = path end
    end
  end)
end

local function select(index)
  if #matches == 0 then
    state.selected, state.first = 1, 1
    state.thumb = ""
    refill()
    return
  end
  index = math.max(1, math.min(#matches, index))
  state.selected = index
  if index < state.first then state.first = index end
  if index > state.first + ROWS - 1 then state.first = index - ROWS + 1 end
  refill()
  preview_image(matches[index])
end

local function search(text)
  text = text:lower()
  if text == "" then
    matches = entries
  else
    matches = {}
    for _, entry in ipairs(entries) do
      if entry.preview:lower():find(text, 1, true) then matches[#matches + 1] = entry end
    end
  end
  state.first = 1
  select(1)
end

local function load()
  proc.exec({ "cliphist", "list" }, function(output, success)
    if not success then
      state.error = "cliphist is not installed, or has nothing yet"
      entries = {}
      search("")
      return
    end
    state.error = ""
    entries = {}
    for line in output:gmatch("[^\n]+") do
      local id, preview = line:match("^(%d+)\t(.*)$")
      if id then
        entries[#entries + 1] = { id = id, preview = preview, image = is_image(preview) }
      end
    end
    search("")
  end)
end

local function copy(entry)
  if not entry then return end
  proc.sh("cliphist decode " .. entry.id .. " | wl-copy")
  cliphist.close()
end

local query = field.new {
  placeholder = "search clips...",
  on_change = search,
  on_submit = function() copy(matches[state.selected]) end,
  on_escape = function() cliphist.close() end,
  on_key = function(keysym)
    if keysym == K.DOWN or keysym == K.TAB then select(state.selected + 1) return true end
    if keysym == K.UP then select(state.selected - 1) return true end
    if keysym == K.PAGE_DOWN then select(state.selected + ROWS) return true end
    if keysym == K.PAGE_UP then select(state.selected - ROWS) return true end
    if keysym == K.HOME then select(1) return true end
    if keysym == K.END then select(#matches) return true end
    if keysym == K.DELETE then
      local entry = matches[state.selected]
      if entry then
        proc.sh("cliphist list | grep '^" .. entry.id .. "\t' | cliphist delete", function() load() end)
      end
      return true
    end
    return false
  end,
}

-- ------------------------------------------------------------------- rows --

local function row(item)
  local selected = function() return item.index == state.selected end
  return ui.Item {
    width = INNER, height = ROW_HEIGHT,
    enter = { opacity = 0, translate_x = S(30) },
    opacity = 1, translate_x = 0,
    behavior = { opacity = theme.motion.fade, translate_x = theme.motion.spring },
    ui.Row {
      gap = S(14), align = "center", height = ROW_HEIGHT,
      anchors = { left = true, left_margin = S(18) },
      theme.icon { text = item.image and "󰋩" or "󰆏", size = 14, color = C.dim },
      theme.text {
        text = item.image and item.preview:gsub("%[%[ binary data ", ""):gsub(" %]%]", "") or item.preview,
        size = 15, width = INNER - S(80), elide = "right",
        color = function() return selected() and C.text or C.dim end,
      },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_entered = function() select(item.index) end,
      on_clicked = function() copy(matches[item.index]) end,
    },
  }
end

-- ---------------------------------------------------------------- surface --

local HEIGHT = PAD * 2 + S(36) + S(16) + S(56) + S(16) + ROW_HEIGHT * ROWS + S(16) + THUMB

local root
local window
window = morf.window.layer {
  namespace = "chillpill-cliphist",
  layer = "overlay",
  keyboard_focus = "exclusive",
  width = WIDTH,
  height = HEIGHT,
  visible = false,
  root = (function()
    root = theme.box {
    width = WIDTH, height = HEIGHT, radius = 36,
    opacity = 0, scale = 0.96,
    behavior = { opacity = theme.motion.fade, scale = theme.motion.spring },
    ui.Inset {
      margin = PAD,
      ui.Column {
        gap = S(16),
        ui.Item {
          width = INNER, height = S(36),
          theme.text { text = "Clipboard History", size = 22, font_weight = 700, anchors = { left = true, top = true, top_margin = S(2) } },
          theme.text {
            size = 14, color = C.dim, anchors = { right = true, top = true, top_margin = S(8) },
            text = function()
              return string.format("%d / %d (%d)", math.min(state.selected, state.count), state.count, state.total)
            end,
          },
        },
        field.node(query, { width = INNER, height = S(56), radius = 16 }),
        ui.Item {
          width = INNER, height = ROW_HEIGHT * ROWS,
          ui.Rect {
            width = INNER, height = ROW_HEIGHT, radius = S(14), color = C.hover,
            visible = function() return state.count > 0 end,
            translate_y = function() return (state.selected - state.first) * ROW_HEIGHT end,
            behavior = { translate_y = theme.motion.spring },
          },
          ui.Repeater { as = "column", model = shown, delegate = row },
          theme.text {
            size = 15, color = C.faint, anchors = { center_in = true },
            text = function() return state.error ~= "" and state.error or "No clips" end,
            visible = function() return state.count == 0 end,
          },
          ui.MouseArea {
            anchors = { fill = true },
            z = -1,
            on_wheel = function(_, _, _, _, _, steps_y)
              if steps_y == 0 then return end
              state.first = math.max(1, math.min(math.max(1, #matches - ROWS + 1), state.first + steps_y))
              refill()
            end,
          },
        },
        ui.Item {
          width = INNER, height = THUMB,
          ui.ClipRect {
            width = THUMB * 16 / 9, height = THUMB, radius = S(12),
            color = "transparent",
            anchors = { left = true },
            visible = function() return state.thumb ~= "" end,
            ui.Image {
              anchors = { fill = true },
              source = function() return state.thumb end,
              fill_mode = "preserve_aspect_fit",
            },
          },
        },
      },
    },
    ui.MouseArea {
      anchors = { fill = true },
      z = -2,
      on_key_pressed = function(keysym, text) query.handle(keysym, text) end,
    },
  }
    return root
  end)(),
}

local motion = theme.surface_motion(window, root)
cliphist.open_signal = morf.signal("chillpill.cliphist.open", false)

function cliphist.open()
  query.clear()
  state.thumb = ""
  load()
  motion.open()
  cliphist.open_signal:set(true)
end

function cliphist.close()
  motion.close()
  cliphist.open_signal:set(false)
end

function cliphist.toggle()
  if cliphist.open_signal:get() then cliphist.close() else cliphist.open() end
  return cliphist.open_signal:get()
end

return cliphist
