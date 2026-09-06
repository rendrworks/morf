-- The app launcher.
--
-- A surface of its own with the keyboard: type to filter every desktop
-- entry on the session, up and down to pick, Enter to start it, Escape to
-- put it away. Entries come from `core.desktop_entries`; what to show and
-- in what order is decided here.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local field = require("field")

local S = theme.S
local C = theme.color
local K = field.keys

local launcher = {}

local WIDTH = S(720)
local PAD = S(24)
local INNER = WIDTH - PAD * 2
local ROW_HEIGHT = S(82)
local ROWS = 7
local ICON = S(48)

-- --------------------------------------------------------------- entries --

local entries = {}

--- Where desktop entries live: every `applications` directory under the
--- XDG data directories, plus the usual places in case the session was
--- started with `XDG_DATA_DIRS` pointed elsewhere (a nixGL-style wrapper
--- does that).
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
  local ok, table_ = pcall(function()
    return core.desktop_entries(entry_paths()):applications()
  end)
  if not ok or type(table_) ~= "table" then return end
  local seen = {}
  for _, entry in ipairs(table_) do
    if not entry.no_display and entry.name ~= "" and not seen[entry.name] then
      seen[entry.name] = true
      entries[#entries + 1] = {
        id = entry.id,
        name = entry.name,
        comment = entry.comment or "",
        icon = entry.icon or "",
        command = entry.command or {},
        terminal = entry.run_in_terminal == true,
        keywords = table.concat(entry.keywords or {}, " "):lower(),
        lower = entry.name:lower(),
      }
    end
  end
  table.sort(entries, function(a, b) return a.lower < b.lower end)
end
load_entries()

--- Entries matching `query`, best first: a name that starts with it, then
--- a name that contains it, then a description or keyword that does.
local function matching(query)
  query = query:lower()
  if query == "" then return entries end
  local ranked = {}
  for _, entry in ipairs(entries) do
    local rank = nil
    if entry.lower:sub(1, #query) == query then rank = 0
    elseif entry.lower:find(query, 1, true) then rank = 1
    elseif entry.comment:lower():find(query, 1, true) or entry.keywords:find(query, 1, true) then rank = 2
    end
    if rank then ranked[#ranked + 1] = { rank = rank, entry = entry } end
  end
  table.sort(ranked, function(a, b)
    if a.rank ~= b.rank then return a.rank < b.rank end
    return a.entry.lower < b.entry.lower
  end)
  local out = {}
  for index, item in ipairs(ranked) do out[index] = item.entry end
  return out
end

-- ------------------------------------------------------------------ state --

launcher.state = morf.state { selected = 1, first = 1, count = 0, total = 0 }
local state = launcher.state
local shown = morf.list_model({})
local matches = entries

local function refill()
  local rows = {}
  local last = math.min(#matches, state.first + ROWS - 1)
  for index = state.first, last do
    local entry = matches[index]
    rows[#rows + 1] = {
      id = entry.id, name = entry.name, comment = entry.comment, icon = entry.icon,
      index = index,
    }
  end
  shown:replace(rows, "id")
  state.count = #matches
  state.total = #entries
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

local function search(query)
  matches = matching(query)
  state.first = 1
  select(1)
end

local function launch(entry)
  if not entry then return end
  local command = entry.command
  if type(command) ~= "table" or #command == 0 then return end
  if entry.terminal then
    local wrapped = { config.defaultTerminal, "-e" }
    for _, word in ipairs(command) do wrapped[#wrapped + 1] = word end
    command = wrapped
  end
  pcall(core.exec_detached, command)
  launcher.close()
end

local query = field.new {
  placeholder = "search apps...",
  on_change = search,
  on_submit = function() launch(matches[state.selected]) end,
  on_escape = function() launcher.close() end,
  on_key = function(keysym)
    if keysym == K.DOWN or keysym == K.TAB then select(state.selected + 1) return true end
    if keysym == K.UP then select(state.selected - 1) return true end
    if keysym == K.PAGE_DOWN then select(state.selected + ROWS) return true end
    if keysym == K.PAGE_UP then select(state.selected - ROWS) return true end
    if keysym == K.HOME then select(1) return true end
    if keysym == K.END then select(#matches) return true end
    return false
  end,
}

-- ------------------------------------------------------------------- rows --

local function row(item)
  local selected = function() return item.index == state.selected end
  local node = ui.Rect {
    width = INNER, height = ROW_HEIGHT, radius = S(16),
    color = function() return selected() and C.card or morf.color("transparent") end,
    ui.Rect {
      width = S(3), height = ROW_HEIGHT - S(24), radius = S(2), color = C.text,
      anchors = { left = true, left_margin = S(14), top = true, top_margin = S(12) },
      visible = selected,
    },
    ui.Row {
      gap = S(20), align = "center", height = ROW_HEIGHT,
      anchors = { left = true, left_margin = S(32) },
      ui.Item {
        width = ICON, height = ICON,
        item.icon ~= "" and ui.Icon { name = item.icon, width = ICON, height = ICON }
          or theme.icon { text = "󰣆", size = 26, color = C.dim, anchors = { center_in = true } },
      },
      ui.Column {
        gap = S(6),
        theme.text { text = item.name, size = 18, font_weight = 700, width = INNER - S(120), elide = "right" },
        theme.text {
          text = item.comment ~= "" and item.comment or " ", size = 14, color = C.dim,
          width = INNER - S(120), elide = "right",
        },
      },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_entered = function() state.selected = item.index end,
      on_clicked = function() launch(matches[item.index]) end,
    },
  }
  return node
end

-- ---------------------------------------------------------------- surface --

local HEIGHT = PAD * 2 + S(36) + S(16) + S(56) + S(16) + ROW_HEIGHT * ROWS + S(8)

local root
local window
window = morf.window.layer {
  namespace = "chillpill-launcher",
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
          theme.text { text = "Applications", size = 22, font_weight = 700, anchors = { left = true, top = true, top_margin = S(2) } },
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
          ui.Repeater { as = "column", model = shown, delegate = row },
          theme.text {
            text = "No matches", size = 15, color = C.faint, anchors = { center_in = true },
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
launcher.open_signal = morf.signal("chillpill.launcher.open", false)

function launcher.open()
  load_entries()
  query.clear()
  search("")
  motion.open()
  launcher.open_signal:set(true)
end

function launcher.close()
  motion.close()
  launcher.open_signal:set(false)
end

function launcher.toggle()
  if launcher.open_signal:get() then launcher.close() else launcher.open() end
  return launcher.open_signal:get()
end

return launcher
