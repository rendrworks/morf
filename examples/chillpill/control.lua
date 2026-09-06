-- The control center, with the Wi-Fi and Bluetooth panels beside it.
--
-- Under the pill: the player, four buttons (Wi-Fi, silent, timer,
-- Bluetooth), the volume and brightness sliders, and the notifications
-- since the shell started. The Wi-Fi button opens a list of networks to the
-- left, the Bluetooth button a list of devices to the right.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local media = require("media")
local notify = require("notify")
local proc = require("proc")
local field = require("field")

local S = theme.S
local C = theme.color
local state = system.state

local control = {}

local WIDTH = S(520)
local PAD = S(24)
local INNER = WIDTH - PAD * 2
local SIDE_WIDTH = S(360)
-- A side panel lists this many at most, so it ends above the screen's foot.
local MOST_ROWS = 8

control.wifi_open = morf.signal("chillpill.control.wifi", false)
control.bluetooth_open = morf.signal("chillpill.control.bluetooth", false)
control.shown = morf.signal("chillpill.control.shown", false)

-- Filled by init: what makes the OSD say something.
control.osd = function() end
control.prompt = nil

-- ------------------------------------------------------------------ player --

local icon_buttons = 0
local function icon_button(glyph, size, on_click, enabled)
  icon_buttons = icon_buttons + 1
  local hovered = morf.signal("chillpill.control.icon." .. icon_buttons, false)
  return ui.Item {
    width = S(40), height = S(40),
    theme.icon {
      text = glyph, size = size,
      anchors = { center_in = true },
      color = function()
        if enabled and not enabled() then return C.faint end
        return hovered:get() and C.text or C.dim
      end,
      behavior = { color = { duration = 100 } },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_entered = function() hovered:set(true) end,
      on_exited = function() hovered:set(false) end,
      on_clicked = on_click,
    },
  }
end

local function player_card()
  local ART = S(64)
  local m = media.state
  local progress_width = INNER - S(40)
  local art = ui.ClipRect {
    width = ART, height = ART, radius = S(12),
    ui.Rect { anchors = { fill = true }, color = C.button },
    theme.icon {
      text = "󰎆", size = 28, color = C.faint, anchors = { center_in = true },
      visible = function() return m.art == "" end,
    },
    ui.Image {
      anchors = { fill = true },
      source = function() return m.art end,
      fill_mode = "preserve_aspect_crop",
      visible = function() return m.art ~= "" end,
    },
  }
  return theme.padded {
    width = INNER,
    pad = S(20),
    color = C.card,
    radius = 24,
    border_color = C.edge,
    border_width = 1,
    visible = function() return m.present end,
    ui.Column {
      gap = S(14),
      ui.Item {
        width = INNER - S(40), height = ART,
        ui.Row {
          gap = S(18), align = "center", height = ART,
          art,
          ui.Column {
            gap = S(8),
            theme.text {
              text = function() return m.title ~= "" and m.title or "Nothing playing" end,
              size = 20, font_weight = 700,
              width = INNER - S(40) - ART - S(18) - S(140), elide = "right",
            },
            theme.text {
              text = function() return m.artist end,
              size = 14, color = C.dim,
              width = INNER - S(40) - ART - S(18) - S(140), elide = "right",
            },
          },
        },
        ui.Row {
          gap = S(4), align = "center", height = ART,
          anchors = { right = true },
          icon_button("󰒮", 22, media.previous, function() return m.can_go_previous end),
          icon_button(function() return m.status == "Playing" and "󰏤" or "󰐊" end, 26, media.play_pause),
          icon_button("󰒭", 22, media.next, function() return m.can_go_next end),
        },
      },
      ui.Item {
        width = progress_width, height = S(16),
        theme.meter {
          width = progress_width, height = S(5),
          anchors = { left = true, top = true, top_margin = S(5) },
          fraction = function()
            -- Re-read every second while playing.
            local _ = morf.clock and morf.clock:get()
            return media.progress()
          end,
        },
        ui.MouseArea {
          anchors = { fill = true }, cursor = "pointer",
          on_clicked = function(_, _, local_x) media.seek(local_x / progress_width) end,
        },
      },
      ui.Item {
        width = progress_width, height = S(16),
        theme.text {
          size = 13, color = C.dim, anchors = { left = true },
          text = function()
            local _ = morf.clock and morf.clock:get()
            return media.clock(media.position())
          end,
        },
        theme.text {
          size = 13, color = C.dim, anchors = { right = true },
          text = function() return media.clock(m.length) end,
        },
      },
    },
  }
end

-- ----------------------------------------------------------------- buttons --

local BUTTON_HEIGHT = S(60)
local BUTTON_WIDTH = math.floor((INNER - S(12) * 3) / 4)

--- One of the four: an icon, a word, a tint when it is on.
local function big_button(values)
  local active = values.active
  local on_color = morf.color("#1f2634")
  local node = theme.button {
    width = BUTTON_WIDTH, height = BUTTON_HEIGHT, radius = 16,
    color = function() return (active and active()) and on_color or C.card end,
    hover_color = C.hover,
    border_color = C.edge, border_width = 1,
    on_click = values.on_click,
    ui.Row {
      gap = S(10), align = "center", height = BUTTON_HEIGHT,
      anchors = { center_in = true },
      theme.icon { text = values.glyph, size = 18, color = values.color or C.text },
      values.label and theme.text {
        text = values.label, size = 14, width = BUTTON_WIDTH - S(56), elide = "right",
        visible = function()
          local label = type(values.label) == "function" and values.label() or values.label
          return label ~= ""
        end,
      } or nil,
    },
  }
  return node
end

-- The countdown. `preset` is the index into `timerPresets`; `ends_at` is
-- when it fires, in elapsed-timer milliseconds, or nil when idle.
local timer_clock = morf.elapsed_timer()
control.timer = morf.state { preset = 1, ends_at = 0, running = false, remaining = "" }

function control.timer_tick()
  local t = control.timer
  if not t.running then return end
  local left = t.ends_at - timer_clock:elapsed_ms()
  if left <= 0 then
    t.running = false
    t.remaining = ""
    control.osd("timer", "Time's up")
    notify.local_notice("Timer", "Time's up", config.timerPresets[t.preset] .. " minutes have passed", "󱎫")
    return
  end
  local seconds = math.ceil(left / 1000)
  t.remaining = string.format("%d:%02d", math.floor(seconds / 60), seconds % 60)
end

local function timer_label()
  local t = control.timer
  if t.running then return t.remaining end
  return tostring(config.timerPresets[t.preset] or 1) .. "m"
end

local function timer_toggle()
  local t = control.timer
  if t.running then
    t.running = false
    t.remaining = ""
  else
    local minutes = config.timerPresets[t.preset] or 1
    t.ends_at = timer_clock:elapsed_ms() + minutes * 60000
    t.running = true
    control.timer_tick()
  end
end

local function timer_cycle(by)
  local t = control.timer
  if t.running then return end
  local count = #config.timerPresets
  t.preset = ((t.preset - 1 + by) % count) + 1
end

local function buttons()
  local timer_button = big_button {
    glyph = "󱎫",
    label = timer_label,
    color = function() return control.timer.running and C.green or C.text end,
    active = function() return control.timer.running end,
    on_click = timer_toggle,
  }
  return ui.Row {
    gap = S(12),
    big_button {
      glyph = function() return require("bar").network_glyph() end,
      color = function() return state.network.kind == "none" and C.dim or C.blue end,
      label = function()
        if state.network.kind == "none" then return "Off" end
        return require("bar").network_label()
      end,
      active = function() return control.wifi_open:get() end,
      on_click = function()
        control.wifi_open:set(not control.wifi_open:get())
        if control.wifi_open:get() then control.scan_wifi() end
      end,
    },
    big_button {
      glyph = function() return notify.silent:get() and "󰂛" or "󰂚" end,
      color = function() return notify.silent:get() and C.dim or C.text end,
      active = function() return notify.silent:get() end,
      on_click = function() notify.silent:set(not notify.silent:get()) end,
    },
    ui.Item {
      width = BUTTON_WIDTH, height = BUTTON_HEIGHT,
      timer_button,
      -- The wheel cycles the presets; it sits over the button and passes
      -- clicks through by taking only the wheel.
      ui.MouseArea {
        anchors = { fill = true },
        z = -1,
        on_wheel = function(_, _, _, _, _, steps_y)
          if steps_y ~= 0 then timer_cycle(steps_y > 0 and 1 or -1) end
        end,
      },
    },
    big_button {
      glyph = function() return require("bar").bluetooth_glyph() end,
      color = function() return state.bluetooth.powered and C.blue or C.dim end,
      label = function()
        if not state.bluetooth.present then return "None" end
        if not state.bluetooth.powered then return "Off" end
        if state.bluetooth.connected ~= "" then return state.bluetooth.connected end
        return "On"
      end,
      active = function() return control.bluetooth_open:get() end,
      on_click = function()
        control.bluetooth_open:set(not control.bluetooth_open:get())
        if control.bluetooth_open:get() then system.poll_bluetooth() end
      end,
    },
  }
end

-- ----------------------------------------------------------------- sliders --

local function slider(values)
  local BAR = INNER - S(120)
  local dragging = false
  local function at(local_x)
    return theme.fraction_at(local_x, 0, BAR)
  end
  return ui.Item {
    width = INNER, height = S(32),
    ui.Item {
      width = S(28), height = S(32),
      theme.icon { text = values.glyph, size = 18, anchors = { center_in = true } },
    },
    ui.Item {
      width = BAR, height = S(32),
      anchors = { left = true, left_margin = S(44) },
      theme.meter {
        width = BAR, height = S(6),
        anchors = { left = true, top = true, top_margin = S(13) },
        fraction = values.fraction,
      },
      ui.MouseArea {
        anchors = { fill = true }, cursor = "pointer",
        on_pressed = function(_, _, local_x)
          dragging = true
          values.set(at(local_x))
        end,
        on_dragged = function(_, _, local_x)
          if dragging then values.set(at(local_x)) end
        end,
        on_released = function() dragging = false end,
        on_wheel = function(_, _, _, _, _, steps_y)
          if steps_y ~= 0 then values.set(values.fraction() - steps_y * 0.05) end
        end,
      },
    },
    theme.text {
      size = 14, anchors = { right = true, top = true, top_margin = S(5) },
      text = function() return string.format("%d%%", math.floor(values.fraction() * 100 + 0.5)) end,
    },
  }
end

local function sliders()
  return ui.Column {
    gap = S(8),
    slider {
      glyph = function() return require("bar").volume_glyph() end,
      fraction = function() return state.volume.level end,
      set = function(level) system.set_volume(level) end,
    },
    slider {
      glyph = "󰃟",
      fraction = function() return state.brightness.level end,
      set = function(level) system.set_brightness(level) end,
    },
  }
end

-- ----------------------------------------------------------- notifications --

local LIST_HEIGHT = S(300)
local ROW_WIDTH = INNER - S(2)

local function notification_row(row)
  local summary = theme.text { text = row.summary ~= "" and row.summary or row.app, size = 15, font_weight = 700,
    width = ROW_WIDTH - S(220), elide = "right" }
  local body = theme.text {
    text = row.body, size = 13, color = C.dim, wrap = true, max_lines = 3,
    width = ROW_WIDTH - S(120),
    visible = row.body ~= "",
  }
  local node = ui.Item {
    width = ROW_WIDTH,
    enter = { opacity = 0, translate_x = S(24) },
    opacity = 1, translate_x = 0,
    behavior = { opacity = theme.motion.fade, translate_x = theme.motion.spring },
    ui.Column {
      anchors = { left = true, top = true, left_margin = S(18), top_margin = S(14) },
      gap = S(6),
      ui.Row {
        gap = S(18), align = "start",
        ui.Item { width = S(36), height = S(36), notify.badge(row, S(36)) },
        ui.Column {
          gap = S(6),
          summary,
          body,
        },
      },
      ui.Item { width = 1, height = S(8) },
    },
    theme.text {
      text = row.time, size = 12, color = C.dim,
      anchors = { right = true, top = true, right_margin = S(56), top_margin = S(16) },
    },
    ui.Item {
      width = S(28), height = S(28),
      anchors = { right = true, top = true, right_margin = S(16), top_margin = S(10) },
      theme.icon { text = "󰅖", size = 14, color = C.dim, anchors = { center_in = true } },
      ui.MouseArea {
        anchors = { fill = true }, cursor = "pointer",
        on_clicked = function() notify.dismiss(row.id) end,
      },
    },
    ui.Rect { height = 1, color = C.edge, anchors = { left = true, right = true, bottom = true, margins = S(12) } },
  }
  return node, function(next)
    summary.text = next.summary ~= "" and next.summary or next.app
    body.text = next.body
    body.visible = next.body ~= ""
  end
end

local function notifications_card()
  local offset = morf.signal("chillpill.control.notify_offset", 0)
  local list = ui.Repeater {
    as = "column",
    model = notify.history,
    delegate = notification_row,
  }
  local clip = ui.ClipRect {
    width = ROW_WIDTH, height = LIST_HEIGHT,
    color = "transparent",
    ui.Item {
      width = ROW_WIDTH,
      translate_y = function() return -offset:get() end,
      behavior = { translate_y = theme.motion.spring },
      list,
    },
    ui.MouseArea {
      anchors = { fill = true },
      z = -1,
      on_wheel = function(_, _, _, _, _, steps_y)
        local total = list.layout_height or 0
        local most = math.max(0, total - LIST_HEIGHT)
        offset:set(math.max(0, math.min(most, offset:get() + steps_y * S(60))))
      end,
    },
  }
  return theme.box {
    width = INNER,
    color = C.card,
    radius = 20,
    border_color = C.edge, border_width = 1,
    ui.Column {
      ui.Item {
        width = INNER, height = S(48),
        ui.Rect { anchors = { fill = true }, color = C.button, top_left_radius = S(20), top_right_radius = S(20) },
        theme.text {
          size = 15, anchors = { left = true, left_margin = S(18), top = true, top_margin = S(14) },
          text = function() return "Notifications (" .. notify.count:get() .. ")" end,
        },
        theme.button {
          width = S(96), height = S(30), radius = 15,
          color = C.hover, hover_color = C.disc,
          anchors = { right = true, right_margin = S(12), top = true, top_margin = S(9) },
          on_click = notify.clear,
          theme.text { text = "Clear all", size = 13, anchors = { center_in = true } },
        },
      },
      ui.Item {
        width = INNER, height = LIST_HEIGHT,
        clip,
        theme.text {
          text = "Nothing yet", size = 14, color = C.faint,
          anchors = { center_in = true },
          visible = function() return notify.count:get() == 0 end,
        },
      },
    },
  }
end

-- ------------------------------------------------------------------- wifi --

control.networks = morf.list_model({})
local scanning = morf.signal("chillpill.control.scanning", false)
local wifi_error = morf.signal("chillpill.control.wifi_error", "")

function control.scan_wifi()
  if scanning:get() then return end
  scanning:set(true)
  proc.exec({ "nmcli", "-t", "-f", "IN-USE,SIGNAL,SECURITY,SSID", "device", "wifi", "list" },
    function(output, success)
      scanning:set(false)
      if not success then
        wifi_error:set("nmcli is not answering")
        return
      end
      wifi_error:set("")
      local rows, seen = {}, {}
      for line in output:gmatch("[^\n]+") do
        local in_use, signal, security, ssid = line:match("^(.-):(%d+):(.-):(.*)$")
        if ssid and ssid ~= "" and not seen[ssid] and #rows < MOST_ROWS then
          seen[ssid] = true
          rows[#rows + 1] = {
            id = ssid,
            ssid = ssid,
            signal = tonumber(signal) or 0,
            secured = security ~= "" and security ~= "--",
            connected = in_use == "*",
          }
        end
      end
      table.sort(rows, function(a, b)
        if a.connected ~= b.connected then return a.connected end
        return a.signal > b.signal
      end)
      control.networks:replace(rows, "id")
    end)
end

local function connect_wifi(ssid, password)
  local command = { "nmcli", "device", "wifi", "connect", ssid }
  if password and password ~= "" then
    command[#command + 1] = "password"
    command[#command + 1] = password
  end
  wifi_error:set("Connecting to " .. ssid .. "…")
  proc.exec(command, function(output, success)
    if success then
      wifi_error:set("")
      system.poll_network()
      control.scan_wifi()
    elseif output:find("ecrets") or output:find("password") then
      if control.prompt then
        control.prompt("Password for " .. ssid, function(secret) connect_wifi(ssid, secret) end)
      else
        wifi_error:set("A password is needed")
      end
    else
      wifi_error:set((output:match("Error:%s*(.-)%.?\n") or "Could not connect"):sub(1, 60))
    end
  end)
end

local function network_row(row)
  local ROW_H = S(64)
  local blue = C.blue
  local node = theme.button {
    width = SIDE_WIDTH - PAD * 2, height = ROW_H, radius = 16,
    color = row.connected and blue or C.card,
    hover_color = row.connected and blue:lighten(0.05) or C.hover,
    on_click = function()
      if row.connected then
        proc.exec({ "nmcli", "device", "disconnect", state.network.iface }, function() system.poll_network() control.scan_wifi() end)
      else
        connect_wifi(row.ssid)
      end
    end,
    ui.Row {
      gap = S(16), align = "center", height = ROW_H,
      anchors = { left = true, left_margin = S(18) },
      theme.icon { text = row.secured and "󰌾" or "󰌿", size = 16, color = row.connected and C.text or C.dim },
      ui.Column {
        gap = S(4),
        theme.text { text = row.ssid, size = 15, font_weight = 700, width = SIDE_WIDTH - PAD * 2 - S(80), elide = "right" },
        theme.text {
          text = row.connected and "Connected" or (row.signal .. "%"),
          size = 12, color = row.connected and C.text:alpha(0.8) or C.dim,
        },
      },
    },
  }
  return node
end

local function wifi_panel()
  return theme.padded(theme.reveal(control.wifi_open, {
    width = SIDE_WIDTH,
    pad = PAD,
    radius = 36,
    from_x = S(28),
    ui.Flex {
      direction = "column", align = "start",
      gap = S(12),
      ui.Item {
        width = SIDE_WIDTH - PAD * 2, height = S(36),
        theme.text { text = "Wi-Fi", size = 20, font_weight = 700, anchors = { left = true, top = true, top_margin = S(4) } },
        ui.Item {
          width = S(32), height = S(32), anchors = { right = true },
          theme.icon {
            text = "󰑐", size = 16, anchors = { center_in = true },
            color = function() return scanning:get() and C.faint or C.dim end,
          },
          ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = control.scan_wifi },
        },
      },
      theme.text {
        size = 13, color = C.dim, width = SIDE_WIDTH - PAD * 2, wrap = true, max_lines = 2,
        text = function() return wifi_error:get() end,
        visible = function() return wifi_error:get() ~= "" end,
      },
      ui.Repeater {
        as = "column", gap = S(10),
        model = control.networks,
        delegate = network_row,
      },
      theme.text {
        text = "No networks found", size = 14, color = C.faint,
        visible = function() return control.networks:len() == 0 and not scanning:get() end,
      },
    },
  }))
end

-- -------------------------------------------------------------- bluetooth --

control.devices = morf.list_model({})

local function refresh_devices()
  local _, devices = system.bluetooth_objects()
  local rows = {}
  for _, device in ipairs(devices or {}) do
    if #rows >= MOST_ROWS then break end
    if device.paired or device.connected or device.trusted or device.name ~= device.address then
      rows[#rows + 1] = {
        id = device.path, path = device.path, name = device.name,
        connected = device.connected, paired = device.paired, icon = device.icon,
      }
    end
  end
  control.devices:replace(rows, "id")
end
control.refresh_devices = refresh_devices

local function device_glyph(icon)
  if icon:find("audio") or icon:find("headset") or icon:find("headphone") then return "󰋋" end
  if icon:find("phone") then return "󰏲" end
  if icon:find("computer") or icon:find("laptop") then return "󰌢" end
  if icon:find("input") or icon:find("keyboard") then return "󰌌" end
  if icon:find("mouse") then return "󰍽" end
  return "󰂯"
end

local function device_row(row)
  local ROW_H = S(72)
  local blue = C.blue
  local subtitle = row.connected and "Connected"
    or (row.paired and "Paired • right-click to forget" or "Not paired")
  local node = theme.button {
    width = SIDE_WIDTH - PAD * 2, height = ROW_H, radius = 16,
    color = row.connected and blue or C.card,
    hover_color = row.connected and blue:lighten(0.05) or C.hover,
    on_click = function()
      system.bluetooth_connect(row.path, not row.connected)
      morf.timer(2000, refresh_devices, false)
    end,
    ui.Row {
      gap = S(16), align = "center", height = ROW_H,
      anchors = { left = true, left_margin = S(18) },
      theme.icon { text = row.connected and "󰂱" or device_glyph(row.icon or ""), size = 16, color = row.connected and C.text or C.dim },
      ui.Column {
        gap = S(4),
        theme.text { text = row.name, size = 15, font_weight = 700, width = SIDE_WIDTH - PAD * 2 - S(80), elide = "right" },
        theme.text {
          text = subtitle, size = 12,
          color = row.connected and C.text:alpha(0.8) or C.dim,
          width = SIDE_WIDTH - PAD * 2 - S(80), wrap = true, max_lines = 2,
        },
      },
    },
    ui.MouseArea {
      anchors = { fill = true },
      accepted_buttons = "right",
      on_clicked = function()
        system.bluetooth_forget(row.path)
        morf.timer(600, refresh_devices, false)
      end,
    },
  }
  return node
end

local function bluetooth_panel()
  return theme.padded(theme.reveal(control.bluetooth_open, {
    width = SIDE_WIDTH,
    pad = PAD,
    radius = 36,
    from_x = -S(28),
    ui.Flex {
      direction = "column", align = "start",
      gap = S(12),
      ui.Item {
        width = SIDE_WIDTH - PAD * 2, height = S(36),
        theme.text { text = "Bluetooth", size = 20, font_weight = 700, anchors = { left = true, top = true, top_margin = S(4) } },
        -- The switch.
        theme.button {
          width = S(52), height = S(28), radius = 14,
          anchors = { right = true, top = true, top_margin = S(4) },
          color = function() return state.bluetooth.powered and C.blue or C.button end,
          hover_color = function() return state.bluetooth.powered and C.blue:lighten(0.05) or C.hover end,
          on_click = function()
            system.set_bluetooth_power(not state.bluetooth.powered)
            morf.timer(800, function() system.poll_bluetooth() refresh_devices() end, false)
          end,
          ui.Rect {
            width = S(22), height = S(22), radius = S(11), color = C.text,
            y = S(3),
            x = function() return state.bluetooth.powered and S(27) or S(3) end,
            behavior = { x = { duration = 140, easing = "out_quad" } },
          },
        },
      },
      theme.text {
        size = 13, color = C.dim,
        text = function()
          if not state.bluetooth.present then return "No adapter" end
          if not state.bluetooth.powered then return "Turned off" end
          return "Visible as \"" .. state.bluetooth.adapter_name .. "\""
        end,
      },
      ui.Repeater {
        as = "column", gap = S(10),
        model = control.devices,
        delegate = device_row,
      },
    },
  }))
end

-- ------------------------------------------------------------------ panel --

local function centre_panel()
  return theme.padded {
    width = WIDTH,
    pad = PAD,
    radius = 36,
    -- A Flex rather than a Column: a hidden child takes no room in one,
    -- which is how the player card leaves when nothing plays.
    ui.Flex {
      direction = "column", align = "start",
      gap = S(16),
      player_card(),
      buttons(),
      ui.Item { width = INNER, height = S(4) },
      sliders(),
      ui.Item { width = INNER, height = S(4) },
      notifications_card(),
    },
  }
end

--- The three panels in a row under the pill; `top` is the pill's zone.
function control.build(top)
  local row = ui.Flex {
    direction = "row",
    gap = S(20),
    align = "start",
    wifi_panel(),
    centre_panel(),
    bluetooth_panel(),
  }
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = top },
    ui.Item(theme.reveal(control.shown, {
      from_y = -S(28),
      from_scale = 0.96,
      transform_origin_y = 0,
      row,
    })),
  }
end

-- The side panels close with the centre one.
morf.timer(250, function()
  if not control.shown:get() then
    control.wifi_open:set(false)
    control.bluetooth_open:set(false)
  end
end, true)

return control
