-- Quick settings tiles, the way a phone lays them out: a grid of switches,
-- each an icon, a name and a line of state, lit while it is on. A tap
-- flips the switch; a right click opens the page behind it, as a long
-- press does on a phone. `tiles` in the settings picks and orders them
-- from `tiles.all`, `tileColumns` says how many to a row, and a tile whose
-- tool is not installed stays away on its own.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local proc = require("proc")
local notify = require("notify")
local bar = require("bar_glyphs")

local S = theme.S
local C = theme.color
local state = system.state

local tiles = {}

-- What the tiles know that `system` does not: the radios, the microphone,
-- the hotspot, the night light, the idle inhibitor.
tiles.state = morf.state {
  wifi_radio = true, airplane = false, mic_muted = false, mic_present = false,
  hotspot = false, nightlight = false, nightlight_tool = "", caffeine = false,
}
local own = tiles.state

local function poll()
  proc.sh(
    "nmcli -t radio wifi 2>/dev/null; echo '--'; rfkill list 2>/dev/null; echo '--'; "
    .. "wpctl get-volume @DEFAULT_AUDIO_SOURCE@ 2>/dev/null; echo '--'; "
    .. "nmcli -t -f NAME,TYPE connection show --active 2>/dev/null; echo '--'; "
    .. "for t in hyprsunset wlsunset gammastep; do command -v $t >/dev/null && { echo $t; pgrep -x $t >/dev/null && echo running; break; }; done",
    function(output, success)
      if not success then return end
      local parts = {}
      for part in (output .. "\n--\n"):gmatch("(.-)\n%-%-\n") do parts[#parts + 1] = part end
      own.wifi_radio = (parts[1] or ""):find("enabled") ~= nil
      local devices, blocked = 0, 0
      for line in (parts[2] or ""):gmatch("[^\n]+") do
        if line:find("Soft blocked") then
          devices = devices + 1
          if line:find("yes") then blocked = blocked + 1 end
        end
      end
      own.airplane = devices > 0 and blocked == devices
      local mic = parts[3] or ""
      own.mic_present = mic:find("Volume") ~= nil
      own.mic_muted = mic:find("MUTED") ~= nil
      own.hotspot = (parts[4] or ""):find("Hotspot") ~= nil
      local light = parts[5] or ""
      own.nightlight_tool = light:match("^(%S+)") or ""
      own.nightlight = light:find("running") ~= nil
    end)
end
poll()
morf.timer(5000, poll, true)

-- Coffee mode: a `systemd-inhibit` child that lives while the switch is on.
local coffee_process = nil
function tiles.set_caffeine(on)
  own.caffeine = on
  if on and not coffee_process then
    local io = require("morf.io")
    local ok, process = pcall(io.process_view, {
      command = { "systemd-inhibit", "--what=idle", "--who=panacea", "--why=Coffee mode", "sleep", "infinity" },
      environment = { LD_LIBRARY_PATH = "" },
    })
    if ok then
      pcall(process.start, process)
      coffee_process = process
    end
  elseif not on and coffee_process then
    pcall(coffee_process.kill, coffee_process)
    coffee_process = nil
  end
end

local function after(command)
  proc.sh(command, function() poll() end)
end

--- Switches the Wi-Fi radio, from the page's own switch.
function tiles.set_wifi_radio(on)
  own.wifi_radio = on
  after(on and "rfkill unblock wifi; nmcli radio wifi on" or "nmcli radio wifi off")
  morf.timer(2500, function() system.poll_network() end, false)
end

--- Every tile there is. `on` says whether it is lit, `toggle` is the tap,
--- `page` the page a right click (or a tap without a toggle) opens,
--- `available` whether the machine can do it at all. `slot` names a piece
--- of the strip that lands on the tile's icon and title.
tiles.all = {
  wifi = {
    icon = function() return own.wifi_radio and bar.network_glyph() or "󰤮" end,
    title = function()
      if state.network.kind ~= "wifi" or state.network.name == "" then return "Wi-Fi" end
      return state.network.name
    end,
    subtitle = function()
      if not own.wifi_radio then return "Off" end
      if state.network.kind == "wifi" then return state.network.strength .. "%" end
      return "Not connected"
    end,
    on = function() return own.wifi_radio end,
    toggle = function() tiles.set_wifi_radio(not own.wifi_radio) end,
    page = "wifi",
  },
  bluetooth = {
    icon = "󰂯",
    title = "Bluetooth",
    subtitle = function()
      if not state.bluetooth.powered then return "Off" end
      return state.bluetooth.connected ~= "" and state.bluetooth.connected or "On"
    end,
    on = function() return state.bluetooth.powered end,
    toggle = function() system.set_bluetooth_power(not state.bluetooth.powered) end,
    page = "bt",
    available = function() return state.bluetooth.present end,
  },
  sound = {
    icon = bar.volume_glyph,
    title = "Sound",
    subtitle = function()
      if state.volume.muted then return "Muted" end
      return math.floor(state.volume.level * 100 + 0.5) .. "%"
    end,
    on = function() return not state.volume.muted end,
    toggle = system.toggle_mute,
    page = "audio",
  },
  mic = {
    icon = function() return own.mic_muted and "󰍭" or "󰍬" end,
    title = "Microphone",
    subtitle = function() return own.mic_muted and "Muted" or "On" end,
    on = function() return not own.mic_muted end,
    toggle = function() after("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle") end,
    page = "audio",
    available = function() return own.mic_present end,
  },
  airplane = {
    icon = "󰀝",
    title = "Airplane",
    subtitle = function() return own.airplane and "On" or "Off" end,
    on = function() return own.airplane end,
    toggle = function() after(own.airplane and "rfkill unblock all" or "rfkill block all") end,
    tint = C.warn_tint or C.on_tint, icon_color = C.warn,
  },
  hotspot = {
    icon = "󰑩",
    title = "Hotspot",
    subtitle = function() return own.hotspot and "Sharing" or "Off" end,
    on = function() return own.hotspot end,
    toggle = function()
      after(own.hotspot and "nmcli connection down Hotspot" or "nmcli device wifi hotspot")
    end,
    page = "wifi",
  },
  vpn = {
    icon = "󰦝",
    title = function() return state.network.vpn and state.network.vpn_name ~= "" and state.network.vpn_name or "VPN" end,
    subtitle = function() return state.network.vpn and "Connected" or "Off" end,
    on = function() return state.network.vpn end,
    toggle = function()
      if state.network.vpn then
        after("nmcli connection down '" .. state.network.vpn_name .. "'")
      else
        after("nmcli -t -f NAME,TYPE connection show | grep -E ':(vpn|wireguard)$' | head -1 | cut -d: -f1 | xargs -r -I{} nmcli connection up '{}'")
      end
    end,
  },
  dnd = {
    icon = function() return notify.silent:get() and "󰂛" or "󰂚" end,
    title = "Don't disturb",
    subtitle = function() return notify.silent:get() and "On" or "Off" end,
    on = function() return notify.silent:get() end,
    toggle = function() notify.silent:set(not notify.silent:get()) end,
    page = "notif",
  },
  caffeine = {
    icon = "󰅶",
    title = "Coffee mode",
    subtitle = function() return own.caffeine and "Screen stays on" or "Off" end,
    on = function() return own.caffeine end,
    toggle = function() tiles.set_caffeine(not own.caffeine) end,
    tint = C.coffee_tint, icon_color = C.coffee, edge = C.coffee:alpha(0.6),
  },
  nightlight = {
    icon = "󰖔",
    title = "Night light",
    subtitle = function() return own.nightlight and "On" or "Off" end,
    on = function() return own.nightlight end,
    toggle = function()
      local tool = own.nightlight_tool
      if own.nightlight then
        after("pkill -x " .. tool)
      elseif tool == "hyprsunset" then
        after("setsid hyprsunset -t 4000 >/dev/null 2>&1 &")
      elseif tool == "wlsunset" then
        after("setsid wlsunset -t 4000 -T 4001 >/dev/null 2>&1 &")
      elseif tool == "gammastep" then
        after("setsid gammastep -O 4000 >/dev/null 2>&1 &")
      end
    end,
    available = function() return own.nightlight_tool ~= "" end,
    tint = C.warn_tint or C.on_tint, icon_color = C.warn,
  },
  record = {
    icon = function() return require("pages.record").state.running and "󰓛" or "󰑊" end,
    title = function()
      local record = require("pages.record").state
      return record.running and ("Rec · " .. record.elapsed) or "Recorder"
    end,
    subtitle = function()
      local record = require("pages.record").state
      return record.running and record.file or "Ready to record"
    end,
    on = function() return require("pages.record").state.running end,
    toggle = function()
      local record = require("pages.record")
      if record.state.running then record.stop() else record.start() end
    end,
    page = "record",
    tint = C.crit_tint, icon_color = C.crit, edge = C.crit:alpha(0.5),
  },
  screenshot = {
    icon = "󰹑",
    title = "Screenshot",
    subtitle = "Region to clipboard",
    on = function() return false end,
    toggle = function()
      require("island").close()
      morf.timer(300, function()
        if system.slurp("/usr/bin/hyprshot") then
          proc.sh("hyprshot -m region --clipboard-only", function() end)
        else
          proc.sh("grim -g \"$(slurp)\" - | wl-copy", function() end)
        end
      end, false)
    end,
  },
  battery = {
    slot = "battery",
    title = function() return state.battery.present and (state.battery.percent .. "%") or "Power" end,
    subtitle = function()
      if state.battery.charging then return "Charging" end
      local profile = require("pages.battery").profile:get()
      return profile ~= "" and profile or (state.battery.plugged and "Plugged in" or "On battery")
    end,
    on = function() return state.battery.charging end,
    page = "battery",
    tint = C.ok_tint, icon_color = C.ok, edge = C.ok:alpha(0.5),
  },
  powersaver = {
    icon = "󰌪",
    title = "Battery saver",
    subtitle = function()
      local profile = require("pages.battery").profile:get()
      return profile ~= "" and profile or "Off"
    end,
    on = function() return require("pages.battery").profile:get() == "power-saver" end,
    toggle = function()
      local battery = require("pages.battery")
      battery.set_profile(battery.profile:get() == "power-saver" and "balanced" or "power-saver")
    end,
    page = "battery",
    available = function() return require("pages.battery").info.profiles end,
    tint = C.ok_tint, icon_color = C.ok, edge = C.ok:alpha(0.5),
  },
}

--- One pill, `width` wide: an icon and a name, tinted with the accent
--- while on. The body flips the switch; a tile with a page behind it has
--- a chevron at its right end that opens it, split off by a hairline, so
--- a tap never opens what a tap was meant to switch. A tile with a page
--- and no switch opens it from anywhere. With `slot`, the icon and the
--- name are left empty for a piece of the strip to land on, registered
--- in `slots`.
function tiles.pill(entry, width, island, slots)
  local H = S(50)
  local on = entry.on or function() return false end
  local tint = entry.tint or C.on_tint
  local edge = entry.edge or C.on_edge
  local lit = entry.icon_color or C.on
  local ARROW = entry.page and S(44) or 0
  local function open_page() if entry.page then island.open(entry.page) end end
  local function icon_node()
    if entry.slot then
      local node = ui.Item { width = S(config.iconSize), height = S(config.iconSize) }
      slots[entry.slot .. "_glyph"] = { node = node, size = config.iconSize }
      return node
    end
    return theme.icon { text = entry.icon, size = config.iconSize, color = function() return on() and lit or C.muted end }
  end
  local function label_node(room)
    if entry.slot then
      local node = ui.Item { width = room, height = S((config.fontSize - 1) * 1.3) }
      slots[entry.slot .. "_text"] = { node = node, size = config.fontSize - 1, weight = 700 }
      return node
    end
    return theme.text { text = entry.title, font_weight = 700, size = config.fontSize - 1, width = room, elide = "right" }
  end
  local room = width - S(58) - ARROW
  return theme.button {
    width = width, height = H, radius = H / 2,
    visible = entry.available,
    color = function() return on() and tint or C.card end,
    hover_color = function() return on() and tint or C.card_hover end,
    border_width = 1,
    border_color = function() return on() and edge or C.edge end,
    on_click = entry.toggle or open_page,
    ui.Row {
      gap = S(10), align = "center", height = H,
      anchors = { left = true, left_margin = S(18) },
      icon_node(),
      ui.Column {
        gap = S(1),
        label_node(room),
        entry.subtitle and theme.text { text = entry.subtitle, size = config.fontSize - 5, color = C.muted,
          width = room, elide = "right" } or nil,
      },
    },
    entry.page and ui.Item {
      width = ARROW, height = H,
      anchors = { right = true },
      ui.Rect { width = 1, height = H - S(22), y = S(11), color = C.edge, visible = entry.toggle ~= nil },
      theme.icon { text = "󰅂", size = config.iconSize - 2, anchors = { center_in = true }, color = C.muted },
      entry.toggle and ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = open_page } or nil,
    } or nil,
    ui.MouseArea {
      anchors = { fill = true },
      accepted_buttons = "right",
      on_clicked = open_page,
    },
  }
end

--- The grid, `width` wide, in the settings' order and column count. One
--- wrapping row: a tile that hides -- no modem, no night light installed
--- -- leaves no hole, the rest flow up.
function tiles.build(island, width, slots)
  local columns = math.max(1, math.floor(tonumber(config.tileColumns) or 2))
  local GAP = S(8)
  local tile_w = math.floor((width - GAP * (columns - 1)) / columns)
  local nodes = {}
  for _, name in ipairs(config.tiles or {}) do
    local entry = tiles.all[name]
    if entry then nodes[#nodes + 1] = tiles.pill(entry, tile_w, island, slots) end
  end
  return ui.Flex { direction = "row", wrap = true, gap = GAP, width = width, table.unpack(nodes) }
end

return tiles
