-- The island: one black capsule at the top edge that is the whole shell.
--
-- Collapsed it is a strip with the day, the clock, the workspace, the
-- keyboard layout and the battery. Asked for a page it grows downwards
-- into that page, on a spring, and flows back when the page closes. It
-- hugs the screen edge with two concave corners, so there is no gap and no
-- border, only the content changing.
--
-- Two surfaces draw it. The collapsed pill lives on the shell's own
-- surface, which takes no keyboard. A page lives on a fullscreen overlay
-- surface with exclusive keyboard focus, opened when a page opens and
-- closed once the island has flowed back: every page closes with Escape or
-- a click outside, and that surface is what hears both.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local system = require("system")
local hypr = require("hypr")

local S = theme.S
local C = theme.color
local motion = theme.motion
local state = system.state

local island = {}

local PILL_H = S(config.pillH)
local FLARE = S(config.notchFlare)
local RADIUS = S(config.cornerR)
local PAD = S(16)

-- Which page is open on the expanded surface; "" is collapsed.
island.page = morf.signal("panacea.page", "")
island.expanded = morf.signal("panacea.expanded", false)
-- What the recorder says, for the collapsed strip.
island.recording = morf.signal("panacea.recording", "")

-- Pages register here: name -> { build = fn(host) -> node, width = fn?,
-- on_key = fn(keysym, text)?, on_open = fn?, on_close = fn? }.
island.pages = {}

function island.register(name, page)
  island.pages[name] = page
end

--- The width a page asks for, in output pixels.
local function page_width(name)
  local page = island.pages[name]
  if page and page.width then return page.width() end
  return S(config.panelW)
end

-- --------------------------------------------------------------- collapsed --

--- What the strip shows: day, clock, workspace, layout, battery.
local function collapsed_row()
  local clock_format = config.clock12 and "%I:%M" or "%H:%M"
  if config.clockSeconds then clock_format = clock_format .. ":%S" end
  return ui.Row {
    gap = S(12),
    align = "center",
    height = PILL_H,
    -- The recorder's dot and time, while it runs.
    ui.Row {
      gap = S(6), align = "center",
      visible = function() return island.recording:get() ~= "" end,
      ui.Rect { width = S(8), height = S(8), radius = S(4), color = C.crit },
      theme.text { text = function() return island.recording:get() end, color = C.crit, font_weight = 700 },
    },
    theme.text {
      text = function() return theme.clock:format("%a") end,
      color = C.muted,
      visible = config.clockWeekday,
    },
    theme.text {
      text = function() return theme.clock:format(clock_format) end,
      font_weight = 700,
      size = config.fontSize + 1,
    },
    theme.text {
      text = function() return hypr.state.active_label end,
      font_weight = 700,
    },
    theme.text {
      text = function() return hypr.state.keymap ~= "" and hypr.state.keymap or "US" end,
      color = function() return hypr.state.keymap == "RU" and C.on or C.muted end,
    },
    ui.Row {
      gap = S(4), align = "center",
      visible = state.battery.present,
      theme.icon {
        text = function()
          if state.battery.charging then return "󱐋" end
          if state.battery.percent <= 15 then return "󰁺" end
          return "󰁹"
        end,
        size = config.iconSize - 3,
        color = function()
          if state.battery.charging then return C.ok end
          if state.battery.percent <= 15 then return C.crit end
          return C.fg
        end,
      },
      theme.text { text = function() return state.battery.percent .. "%" end, color = C.muted },
    },
  }
end

-- ------------------------------------------------------------------- body --

--- The capsule and its flares, sized by `width` and `height` functions,
--- around `content`.
local function capsule(width, height, content)
  local body = ui.Rect {
    color = C.bg,
    width = width,
    height = height,
    top_left_radius = config.notchMode and 0 or RADIUS,
    top_right_radius = config.notchMode and 0 or RADIUS,
    bottom_left_radius = RADIUS,
    bottom_right_radius = RADIUS,
    behavior = config.bench and {} or { width = motion.move, height = motion.move },
    content,
  }
  local flares = {}
  if config.notchMode then
    flares[1] = ui.Image {
      source = core.shell_path("assets/flare-left.svg"),
      width = FLARE, height = FLARE,
      anchors = { left = true, top = true, left_margin = -FLARE },
    }
    flares[2] = ui.Image {
      source = core.shell_path("assets/flare-right.svg"),
      width = FLARE, height = FLARE,
      anchors = { right = true, top = true, right_margin = -FLARE },
    }
  end
  return ui.Item {
    width = width,
    height = height,
    behavior = config.bench and {} or { width = motion.move, height = motion.move },
    body,
    flares[1],
    flares[2],
  }
end

--- The collapsed strip, centred under the top edge. `shown` fades it out
--- while the expanded surface has the island.
function island.build_collapsed(shown)
  local row = collapsed_row()
  local content = ui.Item {
    anchors = { fill = true },
    ui.Item {
      anchors = { center_in = true },
      row,
    },
    ui.MouseArea {
      anchors = { fill = true },
      cursor = "pointer",
      on_clicked = function() island.open("main") end,
    },
  }
  local width = function()
    return math.max(S(config.collapsedW), (row.layout_width or 0) + PAD * 3)
  end
  local pill = capsule(width, function() return PILL_H end, content)
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = config.notchMode and 0 or S(config.islandGap) },
    ui.Item {
      width = width,
      height = PILL_H,
      opacity = function() return shown:get() and 1 or 0 end,
      behavior = { opacity = { duration = 80 } },
      pill,
    },
  }
end

-- ---------------------------------------------------------------- expanded --

local page_nodes = {}

--- Every registered page, built once, each in a reveal of its own.
local function build_pages()
  local nodes = {}
  for name, page in pairs(island.pages) do
    local shown = morf.signal("panacea.page.shown." .. name, false)
    local content = page.build(island)
    local node = ui.Item(theme.reveal(shown, {
      anchors = { left = true, top = true, right = true, margins = PAD },
      from_y = -S(12),
      content,
    }))
    page_nodes[name] = { node = node, shown = shown, content = content }
    nodes[#nodes + 1] = node
  end
  return nodes
end

--- The island with its pages, for the expanded surface. Its size follows
--- the page: the strip's when none is open, the page's otherwise.
function island.build_expanded()
  local strip = collapsed_row()
  local strip_node = ui.Item {
    anchors = { fill = true },
    opacity = function() return island.page:get() == "" and 1 or 0 end,
    behavior = { opacity = { duration = 120 } },
    ui.Item { anchors = { center_in = true }, strip },
  }
  local width = function()
    local name = island.page:get()
    if name == "" then return math.max(S(config.collapsedW), (strip.layout_width or 0) + PAD * 3) end
    return page_width(name)
  end
  local height = function()
    local name = island.page:get()
    if name == "" then return PILL_H end
    local entry = page_nodes[name]
    local wanted = entry and (entry.content.layout_height or 0) or 0
    if wanted <= 0 then return PILL_H end
    return math.min(S(config.expandedH), wanted + PAD * 2)
  end
  local pages = build_pages()
  local content = ui.Item {
    anchors = { fill = true },
    strip_node,
    ui.Item { anchors = { fill = true }, table.unpack(pages) },
  }
  local body = capsule(width, height, content)
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = config.notchMode and 0 or S(config.islandGap) },
    body,
  }
end

-- ------------------------------------------------------------------ verbs --

-- Filled by init: opens and closes the expanded surface.
island.surface = { open = function() end, close = function() end }

function island.open(name)
  if not island.pages[name] then return false end
  local current = island.page:get()
  if current == name then
    island.close()
    return false
  end
  if current ~= "" then
    local leaving = page_nodes[current]
    if leaving then leaving.shown:set(false) end
    local page = island.pages[current]
    if page and page.on_close then page.on_close() end
  end
  local entry = page_nodes[name]
  if not entry then return false end
  island.expanded:set(true)
  island.surface.open()
  island.page:set(name)
  entry.shown:set(true)
  local page = island.pages[name]
  if page.on_open then page.on_open() end
  return true
end

function island.close()
  local current = island.page:get()
  if current == "" then return end
  local entry = page_nodes[current]
  if entry then entry.shown:set(false) end
  local page = island.pages[current]
  if page and page.on_close then page.on_close() end
  island.page:set("")
  -- The island flows back first; the surface goes once it has.
  morf.timer(theme.move_ms * 2, function()
    if island.page:get() == "" then
      island.expanded:set(false)
      island.surface.close()
    end
  end, false)
end

function island.toggle(name)
  if island.page:get() == name then island.close() else island.open(name) end
  return island.page:get()
end

--- A key on the expanded surface: Escape closes, the rest goes to the page.
function island.handle_key(keysym, text)
  local name = island.page:get()
  local page = island.pages[name]
  if page and page.on_key and page.on_key(keysym, text) then return end
  if keysym == 0xff1b then island.close() end
end

return island
