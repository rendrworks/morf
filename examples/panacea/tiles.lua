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
      local parts = proc.sections(output)
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

--- One pill, `width` wide: the kit's row, pill-shaped. The body flips
--- the switch; a tile with a page behind it has a chevron that opens it,
--- split off by a hairline when the body does something else. With
--- `slot`, the icon and the name are left empty for a piece of the strip
--- to land on, registered in `slots`. In a grid the pills cascade in.
function tiles.pill(entry, width, island, slots, stagger)
  local kit = require("kit")
  local on = entry.on or function() return false end
  local function open_page() if entry.page then island.open(entry.page) end end
  local function open_now() return island.page:get() ~= "" end
  local values = {
    width = width, pill = true,
    visible = entry.available,
    icon = entry.icon, title = entry.title, subtitle = entry.subtitle,
    active = on, accent = entry.icon_color, tint = entry.tint, edge = entry.edge,
    on_click = entry.toggle or open_page,
    on_right_click = entry.page and open_page or nil,
    translate_y = stagger and function() return open_now() and 0 or S(18 + 7 * stagger) end or nil,
    opacity = stagger and function() return open_now() and 1 or 0 end or nil,
    on_wheel = function(sx, sy, px, py, steps_x, steps_y)
      if steps_x ~= 0 and tiles.turn then tiles.turn(steps_x > 0 and 1 or -1) else theme.wheel(sx, sy, px, py, steps_x, steps_y) end
    end,
  }
  if entry.page then values.right = kit.chevron(open_page, entry.toggle ~= nil) end
  if entry.slot then
    local icon_slot = ui.Item { width = S(config.iconSize), height = S(config.iconSize), anchors = { center_in = true } }
    local circle = ui.Rect {
      width = kit.CIRCLE, height = kit.CIRCLE, radius = kit.CIRCLE / 2,
      color = function() return on() and (entry.icon_color or C.on) or C.card_hover end,
      behavior = { color = theme.motion.hover },
      icon_slot,
    }
    slots[entry.slot .. "_glyph"] = { node = icon_slot, size = config.iconSize }
    local room = width - kit.PAD - kit.CIRCLE - S(12) - S(56)
    local title_slot = ui.Item { width = room, height = S((config.fontSize - 1) * 1.3) }
    slots[entry.slot .. "_text"] = { node = title_slot, size = config.fontSize - 1, weight = 700 }
    values.icon_node, values.title_node = circle, title_slot
  end
  return kit.row(values)
end

--- The grid, `width` wide, in the settings' order: `tileColumns` to a
--- row, `tileRows` rows to a page, and as many pages as it takes, side by
--- side, one in view. The wheel, or a dot under the grid, turns the page,
--- and the pages slide. A tile that hides -- no modem, no night light
--- installed -- leaves no hole on its page, the rest flow up.
function tiles.build(island, width, slots)
  local columns = math.max(1, math.floor(tonumber(config.tileColumns) or 2))
  local rows = math.max(1, math.floor(tonumber(config.tileRows) or 3))
  local GAP = S(8)
  local H = require("kit").ROW
  local tile_w = math.floor((width - GAP * (columns - 1)) / columns)
  local per_page = columns * rows
  local pages = {}
  for _, name in ipairs(config.tiles or {}) do
    local entry = tiles.all[name]
    if entry then
      local last = pages[#pages]
      if not last or #last == per_page then
        last = {}
        pages[#pages + 1] = last
      end
      last[#last + 1] = tiles.pill(entry, tile_w, island, slots, #last)
    end
  end
  if #pages <= 1 then
    return ui.Flex { direction = "row", wrap = true, gap = GAP, width = width, table.unpack(pages[1] or {}) }
  end
  local page = morf.signal("panacea.tiles.page", 0)
  local function turn(by) page:set(math.max(0, math.min(#pages - 1, page:get() + by))) end
  tiles.turn = turn
  local strip = {}
  for index, nodes in ipairs(pages) do
    strip[index] = ui.Flex { direction = "row", wrap = true, gap = GAP, width = width, table.unpack(nodes) }
  end
  local dots = {}
  for index = 1, #pages do
    dots[index] = ui.Item {
      width = S(16), height = S(16),
      ui.Rect {
        width = S(6), height = S(6), radius = S(3), anchors = { center_in = true },
        color = function() return page:get() == index - 1 and C.on or C.faint end,
        behavior = { color = theme.motion.fade },
      },
      ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() page:set(index - 1) end },
    }
  end
  return ui.Column {
    gap = S(4),
    ui.ClipRect {
      width = width, height = rows * H + (rows - 1) * GAP, color = "transparent",
      ui.Row {
        gap = GAP,
        x = function() return -page:get() * (width + GAP) end,
        behavior = { x = theme.motion.move },
        table.unpack(strip),
      },
      ui.MouseArea {
        anchors = { fill = true }, z = -1,
        on_wheel = function(sx, sy, px, py, steps_x, steps_y)
          if steps_x ~= 0 then turn(steps_x > 0 and 1 or -1) else theme.wheel(sx, sy, px, py, steps_x, steps_y) end
        end,
      },
    },
    ui.Item {
      width = width, height = S(16),
      ui.Row { gap = 0, anchors = { horizontal_center = true }, table.unpack(dots) },
    },
  }
end

return tiles
