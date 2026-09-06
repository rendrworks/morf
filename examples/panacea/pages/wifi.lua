-- Networks: scan, connect, forget. Over nmcli when NetworkManager runs
-- the machine, else iwctl, which is what Panacea itself assumes.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local proc = require("proc")
local field = require("field")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

page.title = "Wi-Fi"
page.icon = "󰤨"
page.subtitle = function()
  if page.status:get() ~= "" then return page.status:get() end
  if page.scanning:get() then return "Scanning…" end
  return state.network.kind == "wifi" and ("Connected to " .. state.network.name) or "Not connected"
end

page.networks = morf.list_model({})
page.status = morf.signal("panacea.wifi.status", "")
page.scanning = morf.signal("panacea.wifi.scanning", false)
page.enabled = morf.signal("panacea.wifi.enabled", true)

local backend = nil
local function detect(callback)
  if backend then return callback(backend) end
  proc.sh("command -v nmcli >/dev/null && echo nm || (command -v iwctl >/dev/null && echo iwd)", function(output)
    backend = proc.trim(output)
    if backend == "" then backend = "none" end
    callback(backend)
  end)
end

local function parse_nm(output)
  local rows, seen = {}, {}
  for line in output:gmatch("[^\n]+") do
    local in_use, signal, security, ssid = line:match("^(.-):(%d+):(.-):(.*)$")
    if ssid and ssid ~= "" and not seen[ssid] then
      seen[ssid] = true
      rows[#rows + 1] = {
        id = ssid, ssid = ssid, signal = tonumber(signal) or 0,
        secured = security ~= "" and security ~= "--", connected = in_use == "*",
      }
    end
  end
  return rows
end

local function parse_iwd(output)
  local rows = {}
  for line in output:gmatch("[^\n]+") do
    -- `  > MyNet   psk   ****` after the header; the `>` marks the one in use.
    local clean = line:gsub("\27%[[%d;]*m", "")
    local mark, ssid, security, bars = clean:match("^%s*(>?)%s*(.-)%s+(%S+)%s+(%*+)%s*$")
    if ssid and ssid ~= "" and not ssid:find("Network name") and not ssid:find("^%-") then
      rows[#rows + 1] = {
        id = ssid, ssid = ssid, signal = #bars * 25,
        secured = security ~= "open", connected = mark == ">",
      }
    end
  end
  return rows
end

function page.scan()
  if page.scanning:get() then return end
  page.scanning:set(true)
  detect(function(kind)
    if kind == "nm" then
      proc.exec({ "nmcli", "-t", "-f", "IN-USE,SIGNAL,SECURITY,SSID", "device", "wifi", "list", "--rescan", "auto" },
        function(output, success)
          page.scanning:set(false)
          if not success then page.status:set("nmcli did not answer") return end
          local rows = parse_nm(output)
          table.sort(rows, function(a, b)
            if a.connected ~= b.connected then return a.connected end
            return a.signal > b.signal
          end)
          page.networks:replace(rows, "id")
          page.status:set("")
        end)
    elseif kind == "iwd" then
      proc.sh("iwctl station wlan0 scan; sleep 2; iwctl station wlan0 get-networks", function(output, success)
        page.scanning:set(false)
        if not success then page.status:set("iwctl did not answer") return end
        page.networks:replace(parse_iwd(output), "id")
        page.status:set("")
      end)
    else
      page.scanning:set(false)
      page.status:set("Neither nmcli nor iwctl is installed")
    end
  end)
end

local function connect(ssid, password)
  page.status:set("Connecting to " .. ssid .. "…")
  detect(function(kind)
    local command
    if kind == "nm" then
      command = { "nmcli", "device", "wifi", "connect", ssid }
      if password and password ~= "" then
        command[#command + 1] = "password"
        command[#command + 1] = password
      end
    else
      command = { "iwctl" }
      if password and password ~= "" then
        command[#command + 1] = "--passphrase"
        command[#command + 1] = password
      end
      for _, word in ipairs { "station", "wlan0", "connect", ssid } do command[#command + 1] = word end
    end
    proc.exec(command, function(output, success)
      if success then
        page.status:set("")
        system.poll_network()
        page.scan()
      elseif output:find("ecret") or output:find("assword") or output:find("assphrase") then
        page.ask(ssid)
      else
        page.status:set((output:match("Error:%s*(.-)%.?\n") or "Could not connect"):sub(1, 70))
      end
    end)
  end)
end

local function disconnect()
  detect(function(kind)
    if kind == "nm" then
      proc.exec({ "nmcli", "device", "disconnect", state.network.iface ~= "" and state.network.iface or "wlan0" },
        function() system.poll_network() page.scan() end)
    else
      proc.exec({ "iwctl", "station", "wlan0", "disconnect" }, function() system.poll_network() page.scan() end)
    end
  end)
end

local function forget(ssid)
  detect(function(kind)
    if kind == "nm" then
      proc.exec({ "nmcli", "connection", "delete", ssid }, function() page.scan() end)
    else
      proc.exec({ "iwctl", "known-networks", ssid, "forget" }, function() page.scan() end)
    end
  end)
end

-- The password, typed into the page itself: the island has the keyboard.
page.asking = morf.signal("panacea.wifi.asking", "")
local password = field.new {
  placeholder = "password",
  secret = true,
  on_submit = function(text)
    local ssid = page.asking:get()
    page.asking:set("")
    if ssid ~= "" then connect(ssid, text) end
  end,
  on_escape = function() page.asking:set("") end,
}

function page.ask(ssid)
  page.asking:set(ssid)
  password.clear()
  page.status:set("")
end

function page.on_key(keysym, text)
  if page.asking:get() ~= "" then
    password.handle(keysym, text)
    return true
  end
  return false
end

function page.on_open()
  page.scan()
end

-- A fresh scan every so often while the page is open.
morf.timer(12000, function()
  if require("island").page:get() == "wifi" then page.scan() end
end, true)

function page.on_close()
  page.asking:set("")
end

function page.build(island)
  local W = S(config.panelW) - S(32)
  local ROW = S(52)
  local function network_row(row)
    return theme.button {
      width = W, height = ROW,
      color = function() return row.connected and C.on_tint or C.card end,
      hover_color = function() return row.connected and C.on_tint or C.card_hover end,
      border_width = 1,
      border_color = function() return row.connected and C.on_edge or C.edge end,
      on_click = function()
        if row.connected then disconnect() else connect(row.ssid) end
      end,
      ui.Row {
        gap = S(12), align = "center", height = ROW,
        anchors = { left = true, left_margin = S(12) },
        theme.icon {
          text = row.signal >= 75 and "󰤨" or row.signal >= 50 and "󰤥" or row.signal >= 25 and "󰤢" or "󰤟",
          size = config.iconSize, color = row.connected and C.on or C.muted,
        },
        ui.Column {
          gap = S(2),
          theme.text { text = row.ssid, font_weight = 700, size = config.fontSize - 1, width = W - S(160), elide = "right" },
          theme.text {
            text = row.connected and "Connected" or (row.signal .. "%" .. (row.secured and "  ·  󰌾" or "")),
            size = config.fontSize - 4, color = C.muted,
          },
        },
      },
      ui.Item {
        width = S(32), height = S(32),
        anchors = { right = true, top = true, right_margin = S(8), top_margin = (ROW - S(32)) / 2 },
        visible = row.connected,
        theme.icon { text = "󰅖", size = config.iconSize - 3, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() forget(row.ssid) end },
      },
    }
  end

  return ui.Column {
    gap = S(10),
    ui.Column {
      gap = S(6),
      visible = function() return page.asking:get() ~= "" end,
      theme.text { text = function() return "Password for " .. page.asking:get() end, size = config.fontSize - 2, color = C.muted },
      field.node(password, { width = W, height = S(44), radius = 12, color = C.card }),
    },
    ui.Repeater { as = "column", gap = S(6), model = page.networks, delegate = network_row },
    theme.text {
      text = "No networks yet", size = config.fontSize - 2, color = C.faint,
      visible = function() return page.networks:len() == 0 and not page.scanning:get() end,
    },
  }
end

return page
