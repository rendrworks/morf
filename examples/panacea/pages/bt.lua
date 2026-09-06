-- Bluetooth: the adapter's switch, scanning, and the devices to connect,
-- disconnect or forget, over bluez.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}

page.title = "Bluetooth"
page.icon = "󰂯"
page.subtitle = function()
  if not state.bluetooth.present then return "No adapter" end
  if not state.bluetooth.powered then return "Off" end
  if page.scanning:get() then return "Looking for devices…" end
  return "Visible as " .. state.bluetooth.adapter_name
end

page.devices = morf.list_model({})
page.scanning = morf.signal("panacea.bt.scanning", false)

function page.refresh()
  local _, devices = system.bluetooth_objects()
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
  local ROW = S(52)
  local function device_row(row)
    return theme.button {
      width = W, height = ROW,
      color = function() return row.connected and C.on_tint or C.card end,
      hover_color = function() return row.connected and C.on_tint or C.card_hover end,
      border_width = 1,
      border_color = function() return row.connected and C.on_edge or C.edge end,
      on_click = function()
        system.bluetooth_connect(row.path, not row.connected)
        morf.timer(2000, page.refresh, false)
      end,
      ui.Row {
        gap = S(12), align = "center", height = ROW,
        anchors = { left = true, left_margin = S(12) },
        theme.icon { text = glyph_for(row.icon), size = config.iconSize, color = row.connected and C.on or C.muted },
        ui.Column {
          gap = S(2),
          theme.text { text = row.name, font_weight = 700, size = config.fontSize - 1, width = W - S(160), elide = "right" },
          theme.text {
            size = config.fontSize - 4, color = C.muted,
            text = (row.connected and "Connected" or (row.paired and "Paired" or "Not paired"))
              .. (row.battery and ("  ·  " .. row.battery .. "%") or ""),
          },
        },
      },
      ui.Item {
        width = S(32), height = S(32),
        anchors = { right = true, top = true, right_margin = S(8), top_margin = (ROW - S(32)) / 2 },
        visible = row.paired,
        theme.icon { text = "󰅖", size = config.iconSize - 3, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea {
          anchors = { fill = true }, cursor = "pointer",
          on_clicked = function()
            system.bluetooth_forget(row.path)
            morf.timer(600, page.refresh, false)
          end,
        },
      },
    }
  end

  return ui.Column {
    gap = S(10),
    ui.Item {
      width = W, height = S(26),
      theme.text { text = "Adapter", size = config.fontSize - 3, color = C.muted, anchors = { left = true, top = true, top_margin = S(4) } },
      ui.Item {
        anchors = { right = true },
        theme.toggle(function() return state.bluetooth.powered end, function(on)
          system.set_bluetooth_power(on)
          morf.timer(800, page.refresh, false)
        end),
      },
    },
    ui.Repeater { as = "column", gap = S(6), model = page.devices, delegate = device_row },
    theme.text {
      text = "No devices", size = config.fontSize - 2, color = C.faint,
      visible = function() return page.devices:len() == 0 end,
    },
  }
end

return page
