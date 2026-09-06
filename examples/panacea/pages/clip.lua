-- The clipboard history, over cliphist, with search.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local field = require("field")
local proc = require("proc")

local S = theme.S
local C = theme.color
local K = field.keys

local page = {}

page.title = "Clipboard"
page.icon = "󰅌"
page.subtitle = function()
  if page.state.error ~= "" then return page.state.error end
  local n = page.state.count
  return n == 0 and "Nothing here" or (n .. (n == 1 and " entry" or " entries"))
end

local ROWS = 9
local ROW_H = S(40)

local entries = {}
local matches = {}
page.state = morf.state { selected = 1, first = 1, count = 0, error = "" }
local state = page.state
local shown = morf.list_model({})

local function refill()
  local rows = {}
  local last = math.min(#matches, state.first + ROWS - 1)
  for index = state.first, last do
    local entry = matches[index]
    rows[#rows + 1] = { id = entry.id, preview = entry.preview, image = entry.image, index = index }
  end
  shown:replace(rows, "id")
  state.count = #matches
end

local function select(index)
  if #matches == 0 then
    state.selected, state.first = 1, 1
    refill()
    return
  end
  index = math.max(1, math.min(#matches, index))
  state.selected = index
  if index < state.first then state.first = index end
  if index > state.first + ROWS - 1 then state.first = index - ROWS + 1 end
  refill()
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
    entries = {}
    if not success then
      state.error = "cliphist is not installed, or has nothing yet"
    else
      state.error = ""
      for line in output:gmatch("[^\n]+") do
        local id, preview = line:match("^(%d+)\t(.*)$")
        if id then
          entries[#entries + 1] = { id = id, preview = preview, image = preview:match("^%[%[ binary data") ~= nil }
        end
      end
    end
    search("")
  end)
end

local function copy(entry)
  if not entry then return end
  proc.sh("cliphist decode " .. entry.id .. " | wl-copy")
  require("island").close()
end

local query
query = field.new {
  placeholder = "search the clipboard",
  on_change = search,
  on_submit = function() copy(matches[state.selected]) end,
  on_escape = function() require("island").close() end,
  on_key = function(keysym)
    if keysym == K.DOWN or keysym == K.TAB then select(state.selected + 1) return true end
    if keysym == K.UP then select(state.selected - 1) return true end
    if keysym == K.PAGE_DOWN then select(state.selected + ROWS) return true end
    if keysym == K.PAGE_UP then select(state.selected - ROWS) return true end
    if keysym == K.DELETE then
      local entry = matches[state.selected]
      if entry then proc.sh("cliphist list | grep '^" .. entry.id .. "\t' | cliphist delete", function() load() end) end
      return true
    end
    return false
  end,
}

function page.on_key(keysym, text) return query.handle(keysym, text) end
function page.on_open() query.clear() load() end

function page.build(island)
  local W = theme.page_w()
  local function row(item)
    return ui.Item {
      width = W, height = ROW_H,
      ui.Row {
        gap = S(10), align = "center", height = ROW_H,
        anchors = { left = true, left_margin = S(10) },
        theme.icon { text = item.image and "󰋩" or "󰆏", size = config.iconSize - 4, color = C.muted },
        theme.text {
          text = item.image and item.preview:gsub("%[%[ binary data ", ""):gsub(" %]%]", "") or item.preview,
          size = config.fontSize - 2, width = W - S(60), elide = "right",
          color = function() return item.index == state.selected and C.fg or C.muted end,
        },
      },
      ui.MouseArea {
        anchors = { fill = true }, cursor = "pointer",
        on_entered = function() select(item.index) end,
        on_clicked = function() copy(matches[item.index]) end,
      },
    }
  end
  return ui.Column {
    gap = S(8),
    field.node(query, { width = W, height = S(44), radius = 12, color = C.card }),
    ui.Item {
      width = W, height = function() return ROW_H * math.max(1, math.min(ROWS, state.count)) end,
      ui.Rect {
        width = W, height = ROW_H, radius = S(10), color = C.card_hover,
        visible = function() return state.count > 0 end,
        translate_y = function() return (state.selected - state.first) * ROW_H end,
        behavior = { translate_y = theme.motion.move },
      },
      ui.Repeater { as = "column", model = shown, delegate = row },
      theme.text {
        size = config.fontSize - 2, color = C.faint, anchors = { center_in = true },
        text = function() return state.error ~= "" and state.error or "Nothing here" end,
        visible = function() return state.count == 0 end,
      },
      ui.MouseArea {
        anchors = { fill = true }, z = -1,
        on_wheel = function(_, _, _, _, _, steps_y)
          if steps_y == 0 then return end
          state.first = math.max(1, math.min(math.max(1, #matches - ROWS + 1), state.first + steps_y))
          refill()
        end,
      },
    },
  }
end

return page
