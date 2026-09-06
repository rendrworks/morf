-- The pill.
--
-- One rounded bar at the top of the output with the modules the config
-- lists, left to right: battery, volume, workspaces, network, clock, and
-- optionally bluetooth, weather and vpn. Each module is a small row of an
-- icon and a word; clicking one opens the panel it belongs to, and the
-- wheel over volume or workspaces changes them.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local system = require("system")
local hypr = require("hypr")

local S = theme.S
local C = theme.color
local state = system.state

local bar = {}

-- Design sizes, scaled once. `pillScale` grows the pill alone.
local P = function(value) return S(value * config.pillScale) end
local HEIGHT = P(38)
local PAD = P(20)
local GAP = P(18)
local DISC = P(27)
local TEXT = 14 * config.pillScale
local ICON = 15 * config.pillScale

--- What the pill takes from the top of the output: the gap above it, the
--- pill, and the gap below less what the compositor already keeps between
--- the reserved zone and the windows, so the pill sits as far from the
--- windows as from the edge.
function bar.zone()
  local below = math.max(0, S(config.pillBottomMargin) - hypr.gaps_out())
  return S(config.pillTopMargin) + HEIGHT + below
end
bar.HEIGHT = HEIGHT

-- What a click on a module opens; `init.lua` fills these in.
bar.open = {
  control = function() end,
  dashboard = function() end,
  wifi = function() end,
  bluetooth = function() end,
  weather = function() end,
}

-- ------------------------------------------------------------------ glyphs --

local BATTERY = { "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹" }
local WIFI = { "󰤯", "󰤟", "󰤢", "󰤥", "󰤨" }

function bar.battery_glyph()
  if state.battery.charging then return "󰂄" end
  local percent = state.battery.percent
  local index = math.max(1, math.min(10, math.floor(percent / 10 + 0.5)))
  if percent <= 5 then index = 1 end
  return BATTERY[index]
end

function bar.battery_color()
  if state.battery.charging or state.battery.plugged then return C.green end
  if state.battery.percent <= 15 then return C.red end
  if state.battery.percent <= 30 then return C.yellow end
  return C.text
end

function bar.volume_glyph()
  if state.volume.muted or state.volume.level <= 0 then return "󰝟" end
  if state.volume.level < 0.34 then return "󰕿" end
  if state.volume.level < 0.67 then return "󰖀" end
  return "󰕾"
end

function bar.network_glyph()
  local network = state.network
  if network.kind == "wifi" then
    local index = math.max(1, math.min(5, math.ceil(network.strength / 20)))
    return WIFI[index]
  elseif network.kind == "wired" then
    return "󰈀"
  end
  return "󰤮"
end

function bar.network_label()
  local network = state.network
  if network.kind == "none" then return "Offline" end
  if not config.showSensitiveInfo then return network.kind == "wifi" and "Wi-Fi" or "Wired" end
  return network.name ~= "" and network.name or (network.kind == "wifi" and "Wi-Fi" or "Wired")
end

function bar.bluetooth_glyph()
  if not state.bluetooth.powered then return "󰂲" end
  if state.bluetooth.connected ~= "" then return "󰂱" end
  return "󰂯"
end

-- ----------------------------------------------------------------- modules --

--- An icon and a word, with an action.
local function module(values)
  local parts = {}
  if values.glyph then
    parts[#parts + 1] = theme.icon { text = values.glyph, size = ICON, color = values.color or C.text }
  end
  if values.label then
    parts[#parts + 1] = theme.text { text = values.label, size = TEXT, color = values.label_color }
  end
  local row = ui.Row {
    gap = P(10),
    align = "center",
    height = HEIGHT,
    table.unpack(parts),
  }
  local hovered = morf.signal("chillpill.bar.module." .. tostring(row), false)
  -- No width of its own: the item is as wide as its row.
  return ui.Item {
    height = HEIGHT,
    scale = function() return hovered:get() and 1.12 or 1 end,
    behavior = { scale = theme.motion.soft },
    row,
    ui.MouseArea {
      anchors = { fill = true },
      cursor = "pointer",
      on_entered = function() hovered:set(true) end,
      on_exited = function() hovered:set(false) end,
      on_clicked = values.on_click,
      on_wheel = values.on_wheel,
    },
  }
end

local function battery_module()
  return module {
    glyph = bar.battery_glyph,
    color = bar.battery_color,
    label = function() return string.format("%d%%", state.battery.percent) end,
    on_click = function() bar.open.control() end,
  }
end

local function volume_module()
  return module {
    glyph = bar.volume_glyph,
    color = function() return state.volume.muted and C.dim or C.text end,
    label = function() return string.format("%d%%", math.floor(state.volume.level * 100 + 0.5)) end,
    label_color = function() return state.volume.muted and C.dim or C.text end,
    on_click = function() bar.open.control() end,
    on_wheel = function(_, _, _, _, _, steps_y)
      if steps_y == 0 then return end
      system.set_volume(state.volume.level - steps_y * 0.05)
    end,
  }
end

local function network_module()
  return module {
    glyph = bar.network_glyph,
    color = function() return state.network.kind == "none" and C.dim or C.blue end,
    label = bar.network_label,
    on_click = function() bar.open.wifi() end,
  }
end

local function bluetooth_module()
  return module {
    glyph = bar.bluetooth_glyph,
    color = function() return state.bluetooth.powered and C.blue or C.dim end,
    label = function()
      if not state.bluetooth.powered then return "Off" end
      if state.bluetooth.connected ~= "" then return state.bluetooth.connected end
      return "On"
    end,
    on_click = function() bar.open.bluetooth() end,
  }
end

local function vpn_module()
  return module {
    glyph = function() return state.network.vpn and "󰦝" or "󰦞" end,
    color = function() return state.network.vpn and C.green or C.dim end,
    label = function() return state.network.vpn and (config.showSensitiveInfo and state.network.vpn_name or "VPN") or "No VPN" end,
    on_click = function() bar.open.control() end,
  }
end

local clock = core.system_clock { precision = "seconds" }
bar.clock = clock

local function clock_module()
  local format = config.clockFormat == "12h" and "%I:%M" or "%H:%M"
  return module {
    label = function() return clock:format(format) end,
    on_click = function() bar.open.dashboard() end,
  }
end

local DISC_GAP = P(12)

--- One workspace: a number, on a disc when it has windows. The active
--- workspace's disc is not here: it is one disc under the row that slides
--- to whichever number is active.
local function workspace_disc(row)
  local disc = ui.Rect {
    width = DISC,
    height = DISC,
    radius = DISC / 2,
    color = row.exists and C.disc or morf.color("transparent"),
    scale = row.exists and 1 or 0.7,
    behavior = { color = theme.motion.fade, scale = theme.motion.spring },
  }
  local label = theme.text {
    text = row.label,
    size = TEXT,
    color = row.active and C.text or (row.exists and C.text or C.text),
    anchors = { center_in = true },
  }
  local node = ui.Item {
    width = DISC,
    height = DISC,
    disc,
    label,
    ui.MouseArea {
      anchors = { fill = true },
      cursor = "pointer",
      on_clicked = function() hypr.go_to(row.id) end,
    },
  }
  return node, function(next)
    disc.color = next.exists and C.disc or morf.color("transparent")
    disc.scale = next.exists and 1 or 0.7
    label.text = next.label
    label.color = next.urgent and C.red or C.text
  end
end

local function workspaces_module()
  local repeater = ui.Repeater {
    as = "row",
    gap = DISC_GAP,
    align = "center",
    height = HEIGHT,
    model = hypr.state.rows,
    delegate = workspace_disc,
  }
  -- The active disc slides along the row on a spring.
  local function active_index()
    local rows = hypr.state.rows
    local count = rows:len()
    local active = hypr.state.active
    for index = 1, count do
      local row = rows:get(index)
      if row and row.id == active then return index end
    end
    return 1
  end
  local slider = ui.Rect {
    width = DISC,
    height = DISC,
    radius = DISC / 2,
    color = C.disc_active,
    y = (HEIGHT - DISC) / 2,
    translate_x = function() return (active_index() - 1) * (DISC + DISC_GAP) end,
    behavior = { translate_x = theme.motion.spring },
  }
  return ui.Item {
    height = HEIGHT,
    slider,
    repeater,
    ui.MouseArea {
      anchors = { fill = true },
      -- Under the discs, so a click reaches them and a wheel reaches this.
      z = -1,
      on_wheel = function(_, _, _, _, _, steps_y)
        if steps_y ~= 0 then hypr.step(steps_y > 0 and 1 or -1) end
      end,
    },
  }
end

local weather_state = nil
function bar.set_weather(state_table) weather_state = state_table end

local function weather_module()
  return module {
    glyph = function() return weather_state and weather_state.glyph or "󰖐" end,
    color = C.text,
    label = function()
      if not weather_state or weather_state.temperature == "" then return "--°" end
      return weather_state.temperature
    end,
    on_click = function() bar.open.weather() end,
  }
end

local MODULES = {
  battery = battery_module,
  volume = volume_module,
  workspaces = workspaces_module,
  network = network_module,
  clock = clock_module,
  bluetooth = bluetooth_module,
  weather = weather_module,
  vpn = vpn_module,
}

-- ----------------------------------------------------------------- the bar --

bar.hovered = morf.signal("chillpill.bar.hovered", false)

--- The pill, centred under the top edge. Returns the node.
function bar.build()
  local children = {}
  for _, name in ipairs(config.pillModules) do
    local build = MODULES[name]
    if build then
      if name == "battery" and not state.battery.present then
        -- A desktop has no battery to show.
        build = nil
      end
    end
    if build then children[#children + 1] = build() end
  end

  -- The padding is two spacers in the row, so the pill's implicit width is
  -- the row's and nothing waits on a laid-out size.
  table.insert(children, 1, ui.Item { width = PAD, height = 1 })
  children[#children + 1] = ui.Item { width = PAD, height = 1 }
  local row = ui.Row {
    gap = GAP,
    align = "center",
    height = HEIGHT,
    table.unpack(children),
  }

  local pill = theme.box {
    height = HEIGHT,
    radius = 38 / 2 * config.pillScale,
    color = C.pill,
    -- Slides up out of sight when the config asks for a pill on hover only.
    translate_y = function()
      return (config.pillOnHover and not bar.hovered:get()) and -(HEIGHT + S(config.pillTopMargin) - S(4)) or 0
    end,
    scale = function() return bar.hovered:get() and 1.04 or 1 end,
    enter = { translate_y = -S(80), opacity = 0 },
    opacity = 1,
    behavior = { translate_y = theme.motion.soft, scale = theme.motion.soft, opacity = theme.motion.fade },
    row,
    ui.MouseArea {
      anchors = { fill = true },
      z = -2,
      on_entered = function() bar.hovered:set(true) end,
      on_exited = function() bar.hovered:set(false) end,
    },
  }

  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = S(config.pillTopMargin) },
    pill,
  }
end

return bar
