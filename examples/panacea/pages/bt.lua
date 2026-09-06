-- Bluetooth: the adapter's switch, scanning, and the devices to connect,
-- disconnect or forget, over bluez.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
local system = require("system")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

page.title = "Bluetooth"
page.icon = "󰂯"
page.subtitle = function()
  if not state.bluetooth.present and state.bluetooth.adapter_name == "" and not state.bluetooth.powered then return "No adapter" end
  if not state.bluetooth.powered then return "Off" end
  if page.scanning:get() then return "Looking for devices…" end
  return "Visible as " .. state.bluetooth.adapter_name
end

page.devices = morf.list_model({})
page.scanning = morf.signal("panacea.bt.scanning", false)

function page.refresh()
  local _, devices = system.bluetooth_objects()
  -- A call that failed -- bluez is slow to answer while it discovers --
  -- says nothing about the devices; the list keeps what it had.
  if not devices then return end
  local rows = {}
  for _, device in ipairs(devices or {}) do
    if #rows >= 10 then break end
    if device.paired or device.connected or device.trusted or device.name ~= device.address then
      rows[#rows + 1] = {
        id = device.path, path = device.path, name = device.name, address = device.address,
        connected = device.connected, paired = device.paired, icon = device.icon or "",
        battery = device.battery,
      }
    end
  end
  page.devices:replace(rows, "id")
  system.poll_bluetooth()
end

function page.scan(on)
  page.scanning:set(on)
  system.bluetooth_scan(on)
end

function page.on_open()
  page.refresh()
  page.scan(true)
end

function page.on_close()
  page.scan(false)
end

morf.timer(3000, function()
  if page.scanning:get() then page.refresh() end
end, true)

local function glyph_for(icon)
  if icon:find("audio") or icon:find("headset") or icon:find("headphone") then return "󰋋" end
  if icon:find("phone") then return "󰏲" end
  if icon:find("computer") or icon:find("laptop") then return "󰌢" end
  if icon:find("input") or icon:find("keyboard") then return "󰌌" end
  if icon:find("mouse") then return "󰍽" end
  if icon:find("gamepad") or icon:find("joystick") then return "󰊗" end
  return "󰂯"
end

function page.build(island)
  local W = theme.page_w()
  local function device_row(row)
    return kit.row {
      width = W,
      icon = glyph_for(row.icon),
      title = row.name,
      subtitle = (row.connected and "Connected" or (row.paired and "Paired" or "Not paired"))
        .. (row.battery and ("  ·  " .. row.battery .. "%") or ""),
      active = function() return row.connected end,
      on_click = function()
        system.bluetooth_connect(row.path, not row.connected)
        morf.timer(2000, page.refresh, false)
      end,
      right = row.paired and kit.cross(function()
        system.bluetooth_forget(row.path)
        morf.timer(600, page.refresh, false)
      end) or nil,
    }
  end
  return kit.page(W, {
    kit.switch_row {
      width = W, icon = "󰂯", title = "Bluetooth",
      subtitle = function()
        if not state.bluetooth.powered then return "Off" end
        if state.bluetooth.connected ~= "" then return "Connected to " .. state.bluetooth.connected end
        return state.bluetooth.adapter_name ~= "" and state.bluetooth.adapter_name or "On"
      end,
      on = function() return state.bluetooth.powered end,
      set = function(on)
        system.set_bluetooth_power(on)
        morf.timer(800, page.refresh, false)
      end,
    },
    kit.section("Devices", function() return page.devices:len() > 0 end),
    ui.Repeater { as = "column", gap = kit.GAP, model = page.devices, delegate = device_row },
    kit.empty(function() return state.bluetooth.powered and "No devices" or "Bluetooth is off" end,
      function() return page.devices:len() == 0 end),
  })
end

return page
