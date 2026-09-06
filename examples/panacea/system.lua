-- What the machine is doing: battery, sound, backlight, network, bluetooth,
-- uptime, traffic. One state table, filled by polls and pushed by the
-- streams that can push; everything drawn reads from here.
--
-- Nothing in this file is engine surface. The engine gives files, processes,
-- D-Bus and timers, and this decides what a "network" is.

local morf = require("morf")
local io = require("morf.io")
local core = require("morf.core")
local proc = require("proc")
local config = require("config")

local system = {}

system.state = morf.state {
  battery = { present = false, percent = 100, charging = false, plugged = false },
  volume = { level = 0.5, muted = false, available = false },
  brightness = { level = 0.7, present = false },
  network = {
    kind = "none",       -- "wifi", "wired", "none"
    name = "",           -- SSID or connection name
    strength = 0,        -- 0-100 for wifi
    iface = "",
    ip = "",
    vpn = false,
    vpn_name = "",
    vpn_iface = "",
    vpn_ip = "",
  },
  bluetooth = { present = false, powered = false, connected = "", connected_count = 0, adapter_name = "" },
  uptime = "",
  uptime_seconds = 0,
  traffic = { rx = 0, tx = 0 },
  user = { name = core.env("USER") or "", full_name = "", host = "" },
}
local state = system.state

--- Reads a small file whole, or nil.
local function slurp(path)
  local ok, handle = pcall(io.file, path)
  if not ok or not handle then return nil end
  local read, text = pcall(handle.read, handle)
  if not read or type(text) ~= "string" then return nil end
  return text
end
system.slurp = slurp

local function trim(text) return proc.trim(text) end

--- Writes a small file whole; false when it cannot.
local function spit(path, text)
  local ok, handle = pcall(io.file, path)
  if not ok or not handle then return false end
  return pcall(handle.write, handle, text)
end

-- ----------------------------------------------------------------- battery --

local battery_dir = nil
for _, name in ipairs { "BAT0", "BAT1", "BAT2", "BATT", "battery" } do
  if slurp("/sys/class/power_supply/" .. name .. "/capacity") then
    battery_dir = "/sys/class/power_supply/" .. name
    break
  end
end

local function poll_battery()
  if not battery_dir then return end
  local capacity = tonumber(trim(slurp(battery_dir .. "/capacity") or ""))
  local status = trim(slurp(battery_dir .. "/status") or "")
  local ac = trim(slurp("/sys/class/power_supply/AC/online") or slurp("/sys/class/power_supply/ACAD/online") or "")
  if capacity then
    state.battery.present = true
    state.battery.percent = capacity
    state.battery.charging = status == "Charging"
    state.battery.plugged = status == "Charging" or status == "Full" or ac == "1"
  end
end

-- ------------------------------------------------------------------- sound --

local function parse_volume(output, success)
  if not success then
    state.volume.available = false
    return
  end
  -- `Volume: 0.56 [MUTED]`
  local level = output:match("Volume:%s*([%d%.]+)")
  if not level then return end
  state.volume.available = true
  state.volume.level = math.max(0, math.min(1, tonumber(level) or 0))
  state.volume.muted = output:find("MUTED", 1, true) ~= nil
end

local function poll_volume()
  proc.exec({ "wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@" }, parse_volume)
end

--- Sets the sink volume, 0..1.
function system.set_volume(level)
  level = math.max(0, math.min(1, level))
  state.volume.level = level
  proc.exec({ "wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@",
    string.format("%d%%", math.floor(level * 100 + 0.5)) }, function() poll_volume() end)
end

function system.toggle_mute()
  state.volume.muted = not state.volume.muted
  proc.exec({ "wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle" }, function() poll_volume() end)
end

-- `pactl subscribe` says the moment a sink changes, which is what makes a
-- volume key show up on the pill without waiting for the next poll.
proc.stream({ "pactl", "subscribe" }, function(line)
  if line:find("on sink", 1, true) or line:find("on server", 1, true) then poll_volume() end
end)

-- --------------------------------------------------------------- backlight --

local backlight_dir = nil
local backlight_max = 1
for _, name in ipairs { "intel_backlight", "amdgpu_bl0", "amdgpu_bl1", "acpi_video0", "nvidia_0", "backlight" } do
  local max = tonumber(trim(slurp("/sys/class/backlight/" .. name .. "/max_brightness") or ""))
  if max and max > 0 then
    backlight_dir = "/sys/class/backlight/" .. name
    backlight_max = max
    break
  end
end
state.brightness.present = backlight_dir ~= nil

local function poll_brightness()
  if not backlight_dir then return end
  local raw = tonumber(trim(slurp(backlight_dir .. "/brightness") or ""))
  if raw then state.brightness.level = raw / backlight_max end
end

--- Sets the backlight, 0..1. Writes the sysfs file when the user may
--- (the `video` group, usually), else asks brightnessctl.
function system.set_brightness(level)
  level = math.max(0.01, math.min(1, level))
  state.brightness.level = level
  if not backlight_dir then return end
  local raw = math.floor(level * backlight_max + 0.5)
  if not spit(backlight_dir .. "/brightness", tostring(raw)) then
    proc.exec({ "brightnessctl", "-q", "s", string.format("%d%%", math.floor(level * 100 + 0.5)) })
  end
end

-- ----------------------------------------------------------------- network --

local function poll_network()
  proc.sh(
    "nmcli -t -f NAME,TYPE,DEVICE connection show --active 2>/dev/null; echo '--'; " ..
    "nmcli -t -f IN-USE,SIGNAL,SSID device wifi list 2>/dev/null | grep '^\\*' | head -1; echo '--'; " ..
    "ip -o -4 addr show scope global 2>/dev/null",
    function(output, success)
      if not success then return end
      local sections = proc.sections(output)
      local connections, wifi, addresses = sections[1] or "", sections[2] or "", sections[3] or ""

      local ips = {}
      for iface, ip in addresses:gmatch("%d+:%s+(%S+)%s+inet%s+([%d%.]+)") do ips[iface] = ip end

      local kind, name, iface = "none", "", ""
      local vpn, vpn_name, vpn_iface = false, "", ""
      for line in connections:gmatch("[^\n]+") do
        local n, t, d = line:match("^(.-):([^:]*):([^:]*)$")
        if n then
          if t == "vpn" or t == "wireguard" or t == "tun" or d:match("^tun") or d:match("^wg") then
            vpn, vpn_name, vpn_iface = true, n, d
          elseif t:find("wireless") and kind ~= "wifi" then
            kind, name, iface = "wifi", n, d
          elseif t:find("ethernet") and kind == "none" then
            kind, name, iface = "wired", n, d
          end
        end
      end
      local signal, ssid = wifi:match("^%*:(%d+):(.*)$")
      if kind == "wifi" then
        state.network.strength = tonumber(signal) or 0
        if ssid and ssid ~= "" then name = ssid end
      else
        state.network.strength = 0
      end
      state.network.kind = kind
      state.network.name = name
      state.network.iface = iface
      state.network.ip = ips[iface] or ""
      state.network.vpn = vpn
      state.network.vpn_name = vpn_name
      state.network.vpn_iface = vpn_iface
      state.network.vpn_ip = ips[vpn_iface] or ""
    end)
end

-- `nmcli monitor` prints a line whenever anything changes.
proc.stream({ "nmcli", "monitor" }, function() poll_network() end)

-- --------------------------------------------------------------- bluetooth --

local bluez = nil
do
  local ok, proxy = pcall(morf.dbus.proxy, "system", "org.bluez", "/",
    "org.freedesktop.DBus.ObjectManager", 4000)
  if ok then bluez = proxy end
end

--- Every adapter and device bluez knows, as plain tables.
function system.bluetooth_objects()
  if not bluez then return nil end
  -- The reply is the method's returns as a list; the one dictionary
  -- `GetManagedObjects` gives back is `reply[1]`.
  local ok, reply = pcall(bluez.call, bluez, "GetManagedObjects")
  if not ok or type(reply) ~= "table" then return nil end
  local objects = reply[1]
  if type(objects) ~= "table" then return nil end
  local adapters, devices = {}, {}
  for path, interfaces in pairs(objects) do
    if type(interfaces) == "table" then
      local adapter = interfaces["org.bluez.Adapter1"]
      if type(adapter) == "table" then
        adapters[#adapters + 1] = {
          path = path,
          powered = adapter.Powered == true,
          discovering = adapter.Discovering == true,
          name = adapter.Alias or adapter.Name or "",
        }
      end
      local device = interfaces["org.bluez.Device1"]
      if type(device) == "table" then
        devices[#devices + 1] = {
          path = path,
          name = device.Alias or device.Name or device.Address or "",
          address = device.Address or "",
          connected = device.Connected == true,
          paired = device.Paired == true,
          trusted = device.Trusted == true,
          icon = device.Icon or "",
          battery = (interfaces["org.bluez.Battery1"] or {}).Percentage,
        }
      end
    end
  end
  table.sort(adapters, function(a, b) return a.path < b.path end)
  table.sort(devices, function(a, b)
    if a.connected ~= b.connected then return a.connected end
    if a.paired ~= b.paired then return a.paired end
    return a.name < b.name
  end)
  return adapters, devices
end

local function poll_bluetooth()
  local adapters, devices = system.bluetooth_objects()
  if not adapters then
    state.bluetooth.present = false
    return
  end
  local adapter = adapters[1]
  state.bluetooth.present = adapter ~= nil
  state.bluetooth.powered = adapter ~= nil and adapter.powered
  state.bluetooth.adapter_name = adapter and adapter.name or ""
  local connected, count = "", 0
  for _, device in ipairs(devices) do
    if device.connected then
      count = count + 1
      if connected == "" then connected = device.name end
    end
  end
  state.bluetooth.connected = connected
  state.bluetooth.connected_count = count
end
system.poll_bluetooth = poll_bluetooth

--- A proxy on one bluez object, for a verb on it.
local function bluez_object(path, interface)
  local ok, proxy = pcall(morf.dbus.proxy, "system", "org.bluez", path, interface, 1500)
  return ok and proxy or nil
end

function system.set_bluetooth_power(on)
  local adapters = system.bluetooth_objects()
  local adapter = adapters and adapters[1]
  if not adapter then return end
  local proxy = bluez_object(adapter.path, "org.bluez.Adapter1")
  if proxy then pcall(proxy.set, proxy, "Powered", on == true) end
  state.bluetooth.powered = on == true
end

--- Connects or disconnects one device. bluez keeps going after the call
--- returns, so a short timeout here only means the answer arrives later.
function system.bluetooth_connect(path, connect)
  local proxy = bluez_object(path, "org.bluez.Device1")
  if not proxy then return end
  pcall(proxy.call, proxy, connect and "Connect" or "Disconnect")
  morf.timer(1500, poll_bluetooth, false)
end

function system.bluetooth_forget(path)
  local adapters = system.bluetooth_objects()
  local adapter = adapters and adapters[1]
  if not adapter then return end
  local proxy = bluez_object(adapter.path, "org.bluez.Adapter1")
  if proxy then pcall(proxy.call_with, proxy, "RemoveDevice", { signature = "o", value = path }) end
  morf.timer(500, poll_bluetooth, false)
end

function system.bluetooth_scan(on)
  local adapters = system.bluetooth_objects()
  local adapter = adapters and adapters[1]
  if not adapter then return end
  local proxy = bluez_object(adapter.path, "org.bluez.Adapter1")
  if proxy then pcall(proxy.call, proxy, on and "StartDiscovery" or "StopDiscovery") end
end

-- ------------------------------------------------------- uptime and traffic --

local function poll_uptime()
  local seconds = tonumber((slurp("/proc/uptime") or ""):match("^([%d%.]+)"))
  if not seconds then return end
  state.uptime_seconds = seconds
  local days = math.floor(seconds / 86400)
  local hours = math.floor(seconds % 86400 / 3600)
  local minutes = math.floor(seconds % 3600 / 60)
  local parts = {}
  if days > 0 then parts[#parts + 1] = days .. (days == 1 and " day" or " days") end
  if hours > 0 then parts[#parts + 1] = hours .. (hours == 1 and " hour" or " hours") end
  parts[#parts + 1] = minutes .. (minutes == 1 and " minute" or " minutes")
  state.uptime = "up " .. table.concat(parts, ", ")
end

--- Bytes received and sent on every interface but loopback, since boot.
local function poll_traffic()
  local text = slurp("/proc/net/dev")
  if not text then return end
  local rx, tx = 0, 0
  for line in text:gmatch("[^\n]+") do
    local iface, numbers = line:match("^%s*([^:]+):%s*(.*)$")
    if iface and iface ~= "lo" then
      local fields = {}
      for n in numbers:gmatch("%d+") do fields[#fields + 1] = tonumber(n) end
      rx = rx + (fields[1] or 0)
      tx = tx + (fields[9] or 0)
    end
  end
  state.traffic.rx = rx
  state.traffic.tx = tx
end

--- `512.5 MB`, `1.2 GB`.
function system.bytes(value)
  value = tonumber(value) or 0
  if value >= 1024 * 1024 * 1024 then return string.format("%.1f GB", value / 1024 / 1024 / 1024) end
  if value >= 1024 * 1024 then return string.format("%.1f MB", value / 1024 / 1024) end
  return string.format("%.0f KB", value / 1024)
end

-- -------------------------------------------------------------------- user --

do
  state.user.host = trim(slurp("/etc/hostname") or "") ~= "" and trim(slurp("/etc/hostname")) or "localhost"
  local passwd = slurp("/etc/passwd") or ""
  local me = state.user.name
  for line in passwd:gmatch("[^\n]+") do
    local name, gecos = line:match("^([^:]+):[^:]*:[^:]*:[^:]*:([^:]*):")
    if name == me then
      state.user.full_name = (gecos or ""):match("^([^,]*)") or ""
      break
    end
  end
end

-- ------------------------------------------------------------------- polls --

poll_battery()
poll_volume()
poll_brightness()
poll_network()
poll_bluetooth()
poll_uptime()
poll_traffic()

morf.timer(10000, poll_battery, true)
morf.timer(5000, poll_volume, true)
morf.timer(1000, poll_brightness, true)
morf.timer(15000, poll_network, true)
morf.timer(10000, poll_bluetooth, true)
morf.timer(30000, poll_uptime, true)
morf.timer(5000, poll_traffic, true)

system.poll_volume = poll_volume
system.poll_network = poll_network
system.poll_brightness = poll_brightness

--- Runs a program the shell does not wait for.
function system.launch(command)
  if type(command) == "string" then command = { "sh", "-c", command } end
  pcall(core.exec_detached, command)
end

return system
