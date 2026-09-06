-- The launcher: every desktop entry, filtered as you type, recents first;
-- a sum typed instead is worked out and copied.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local io = require("morf.io")
local config = require("config")
local theme = require("theme")
local field = require("field")
local proc = require("proc")

local S = theme.S
local C = theme.color
local K = field.keys

local page = {}

page.title = "Launcher"
page.icon = "󰀻"
page.subtitle = function()
  if page.state.result ~= "" then return "= " .. page.state.result end
  return "Apps, recents first; or type a sum"
end

local ROWS = 8
local ROW_H = S(48)

-- --------------------------------------------------------------- entries --

local entries = {}
local recents = {}
local recents_path = config.dir .. "/recents.json"

local function load_recents()
  local ok, handle = pcall(io.file, recents_path)
  if not ok then return end
  local read, text = pcall(handle.read, handle)
  if not read or type(text) ~= "string" or text == "" then return end
  local decoded, list = pcall(io.json.decode, text)
  if decoded and type(list) == "table" then recents = list end
end
load_recents()

local function save_recents()
  local ok, handle = pcall(io.file, recents_path)
  if not ok then return end
  local encoded, text = pcall(io.json.encode, recents)
  if encoded then pcall(handle.write, handle, text) end
end

local function remember(id)
  for index, known in ipairs(recents) do
    if known == id then table.remove(recents, index) break end
  end
  table.insert(recents, 1, id)
  while #recents > 12 do table.remove(recents) end
  save_recents()
end

local function entry_paths()
  local paths, seen = {}, {}
  local function add(path)
    if type(path) == "string" and path ~= "" and not seen[path] then
      seen[path] = true
      paths[#paths + 1] = path
    end
  end
  add((core.env("XDG_DATA_HOME") or (config.home .. "/.local/share")) .. "/applications")
  for directory in (core.env("XDG_DATA_DIRS") or ""):gmatch("[^:]+") do
    add(directory:gsub("/$", "") .. "/applications")
  end
  add("/usr/local/share/applications")
  add("/usr/share/applications")
  add("/var/lib/flatpak/exports/share/applications")
  add(config.home .. "/.local/share/flatpak/exports/share/applications")
  return paths
end

local function load_entries()
  entries = {}
  local ok, list = pcall(function() return core.desktop_entries(entry_paths()):applications() end)
  if not ok or type(list) ~= "table" then return end
  local seen = {}
  for _, entry in ipairs(list) do
    if not entry.no_display and entry.name ~= "" and not seen[entry.name] then
      seen[entry.name] = true
      entries[#entries + 1] = {
        id = entry.id, name = entry.name, comment = entry.comment or "", icon = entry.icon or "",
        command = entry.command or {}, terminal = entry.run_in_terminal == true,
        keywords = table.concat(entry.keywords or {}, " "):lower(), lower = entry.name:lower(),
      }
    end
  end
  table.sort(entries, function(a, b) return a.lower < b.lower end)
end
load_entries()

-- ------------------------------------------------------------ calculator --

--- A sum, or nil: digits, a dot, the four operators, powers, percent and
--- brackets, worked out here rather than handed to the Lua loader.
local function calculate(text)
  local clean = text:gsub("%s+", ""):gsub(",", "."):gsub("×", "*"):gsub("÷", "/")
  if clean == "" or not clean:find("^[%d%.%(%)%+%-%*/%^%%]+$") or not clean:find("%d") then return nil end
  if not clean:find("[%+%-%*/%^%%]") then return nil end
  local pos = 1
  local function peek() return clean:sub(pos, pos) end
  local function number()
    local digits = clean:match("^%d*%.?%d*", pos)
    if not digits or digits == "" or digits == "." then return nil end
    pos = pos + #digits
    return tonumber(digits)
  end
  local expression
  local function atom()
    local char = peek()
    if char == "(" then
      pos = pos + 1
      local value = expression()
      if peek() ~= ")" then return nil end
      pos = pos + 1
      return value
    elseif char == "-" then
      pos = pos + 1
      local value = atom()
      return value and -value or nil
    end
    return number()
  end
  local function power()
    local base = atom()
    if not base then return nil end
    if peek() == "^" then
      pos = pos + 1
      local exponent = power()
      if not exponent then return nil end
      return base ^ exponent
    end
    return base
  end
  local function term()
    local value = power()
    if not value then return nil end
    while peek() == "*" or peek() == "/" or peek() == "%" do
      local op = peek()
      pos = pos + 1
      local right = power()
      if not right then return nil end
      if op == "*" then value = value * right
      elseif op == "/" then if right == 0 then return nil end value = value / right
      else if right == 0 then return nil end value = value % right end
    end
    return value
  end
  expression = function()
    local value = term()
    if not value then return nil end
    while peek() == "+" or peek() == "-" do
      local op = peek()
      pos = pos + 1
      local right = term()
      if not right then return nil end
      value = op == "+" and value + right or value - right
    end
    return value
  end
  local value = expression()
  if not value or pos <= #clean or value ~= value then return nil end
  if value == math.floor(value) and math.abs(value) < 1e15 then return string.format("%d", value) end
  return (string.format("%.6f", value):gsub("0+$", ""):gsub("%.$", ""))
end

-- ------------------------------------------------------------------ state --

page.state = morf.state { selected = 1, first = 1, count = 0, result = "" }
local state = page.state
local shown = morf.list_model({})
local matches = {}

local function ranked(query)
  query = query:lower()
  local recent_rank = {}
  for index, id in ipairs(recents) do recent_rank[id] = index end
  local out = {}
  for _, entry in ipairs(entries) do
    local rank
    if query == "" then
      rank = recent_rank[entry.id] and (0 + recent_rank[entry.id] / 100) or 3
    elseif entry.lower:sub(1, #query) == query then rank = 0
    elseif entry.lower:find(query, 1, true) then rank = 1
    elseif entry.comment:lower():find(query, 1, true) or entry.keywords:find(query, 1, true) then rank = 2
    end
    if rank then out[#out + 1] = { rank = rank, entry = entry } end
  end
  table.sort(out, function(a, b)
    if a.rank ~= b.rank then return a.rank < b.rank end
    return a.entry.lower < b.entry.lower
  end)
  local list = {}
  for index, item in ipairs(out) do list[index] = item.entry end
  return list
end

local function refill()
  local rows = {}
  local last = math.min(#matches, state.first + ROWS - 1)
  for index = state.first, last do
    local entry = matches[index]
    rows[#rows + 1] = { id = entry.id, name = entry.name, comment = entry.comment, icon = entry.icon, index = index }
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
  state.result = calculate(text) or ""
  matches = ranked(text)
  state.first = 1
  select(1)
end

local function launch(entry, island)
  if not entry then return end
  local command = entry.command
  if type(command) ~= "table" or #command == 0 then return end
  if entry.terminal then
    local wrapped = { config.terminal, "-e" }
    for _, word in ipairs(command) do wrapped[#wrapped + 1] = word end
    command = wrapped
  end
  remember(entry.id)
  pcall(core.exec_detached, command)
  island.close()
end

local query
query = field.new {
  placeholder = "search apps, or type a sum",
  on_change = search,
  on_submit = function(text)
    local island = require("island")
    if state.result ~= "" then
      proc.exec({ "wl-copy", state.result })
      island.close()
      return
    end
    launch(matches[state.selected], island)
  end,
  on_escape = function() require("island").close() end,
  on_key = function(keysym)
    if keysym == K.DOWN or keysym == K.TAB then select(state.selected + 1) return true end
    if keysym == K.UP then select(state.selected - 1) return true end
    if keysym == K.PAGE_DOWN then select(state.selected + ROWS) return true end
    if keysym == K.PAGE_UP then select(state.selected - ROWS) return true end
    return false
  end,
}

function page.on_key(keysym, text)
  return query.handle(keysym, text)
end

function page.on_open()
  load_entries()
  query.clear()
  search("")
end

function page.build(island)
  local W = theme.page_w()
  local function row(item)
    return ui.Item {
      width = W, height = ROW_H,
      enter = { opacity = 0, translate_x = S(16) },
      opacity = 1, translate_x = 0,
      behavior = { opacity = theme.motion.fade, translate_x = theme.motion.move },
      ui.Row {
        gap = S(12), align = "center", height = ROW_H,
        anchors = { left = true, left_margin = S(10) },
        ui.Item {
          width = S(28), height = S(28),
          item.icon ~= "" and ui.Icon { name = item.icon, width = S(28), height = S(28) }
            or theme.icon { text = "󰣆", size = config.iconSize, color = C.muted, anchors = { center_in = true } },
        },
        ui.Column {
          gap = S(1),
          theme.text { text = item.name, size = config.fontSize - 1, font_weight = 700, width = W - S(80), elide = "right" },
          theme.text { text = item.comment ~= "" and item.comment or " ", size = config.fontSize - 4, color = C.muted,
            width = W - S(80), elide = "right" },
        },
      },
      ui.MouseArea {
        anchors = { fill = true }, cursor = "pointer",
        on_entered = function() state.selected = item.index end,
        on_clicked = function() launch(matches[item.index], island) end,
      },
    }
  end
  return ui.Column {
    gap = S(8),
    field.node(query, { width = W, height = S(44), radius = 12, color = C.card }),
    theme.card {
      width = W, height = S(44),
      color = C.on_tint,
      visible = function() return state.result ~= "" end,
      ui.Row {
        gap = S(10), align = "center", height = S(44),
        anchors = { left = true, left_margin = S(12) },
        theme.icon { text = "󰃬", size = config.iconSize - 2, color = C.on },
        theme.text { text = function() return "= " .. state.result end, font_weight = 700 },
        theme.text { text = "Enter copies it", size = config.fontSize - 4, color = C.muted },
      },
    },
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
        text = "Nothing matches", size = config.fontSize - 2, color = C.faint, anchors = { center_in = true },
        visible = function() return state.count == 0 and state.result == "" end,
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
