-- The battery: charge and state, the power profile, capacity, health and
-- the power being drawn. Profiles go through power-profiles-daemon over
-- D-Bus; the numbers come from upower.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local proc = require("proc")

local S = theme.S
local C = theme.color
local state = system.state

local page = {}
page.slots = {}

page.title = "Battery"
page.icon = "󰁹"

page.profile = morf.signal("panacea.battery.profile", "")
page.info = morf.state { capacity = "—", health = "—", rate = "—", available = false, profiles = false }

local PROFILES = {
  { id = "power-saver", label = "Power saver", glyph = "󰌪" },
  { id = "balanced", label = "Balanced", glyph = "󰗑" },
  { id = "performance", label = "Performance", glyph = "󰓅" },
}

local daemon = nil
for _, pair in ipairs {
  { "org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles" },
  { "net.hadess.PowerProfiles", "/net/hadess/PowerProfiles" },
} do
  local ok, proxy = pcall(morf.dbus.proxy, "system", pair[1], pair[2], pair[1], 1000)
  if ok then
    local read, active = pcall(proxy.get, proxy, "ActiveProfile")
    if read and type(active) == "string" then
      daemon = proxy
      page.profile:set(active)
      page.info.profiles = true
      break
    end
  end
end

local function pretty(id)
  for _, profile in ipairs(PROFILES) do
    if profile.id == id then return profile.label end
  end
  return id
end

function page.set_profile(id)
  if not daemon then return end
  pcall(daemon.set, daemon, "ActiveProfile", id)
  page.profile:set(id)
end

local function poll()
  if daemon then
    local ok, active = pcall(daemon.get, daemon, "ActiveProfile")
    if ok and type(active) == "string" then page.profile:set(active) end
  end
  proc.sh("upower -i $(upower -e 2>/dev/null | grep -i BAT | head -1) 2>/dev/null", function(output, success)
    if not success or output == "" then
      page.info.available = false
      return
    end
    page.info.available = true
    local full = tonumber(output:match("energy%-full:%s*([%d%.]+)"))
    local design = tonumber(output:match("energy%-full%-design:%s*([%d%.]+)"))
    local rate = tonumber(output:match("energy%-rate:%s*([%d%.]+)"))
    page.info.capacity = full and string.format("%.1f Wh", full) or "—"
    if full and design and design > 0 then
      page.info.health = string.format("%d%%", math.floor(full / design * 100 + 0.5))
    end
    page.info.rate = rate and string.format("%.1f W", rate) or "—"
  end)
end
poll()
morf.timer(30000, poll, true)

page.subtitle = function()
  if not state.battery.present then return "No battery" end
  if state.battery.charging then return "Charging" end
  local profile = page.profile:get()
  if profile ~= "" then return pretty(profile) end
  return state.battery.plugged and "Plugged in" or "On battery"
end

function page.build(island)
  local W = theme.page_w()
  local third = math.floor((W - S(8) * 2) / 3)
  local function profile_button(profile)
    return theme.button {
      width = third, height = S(56),
      color = function() return page.profile:get() == profile.id and C.ok_tint or C.card end,
      hover_color = function() return page.profile:get() == profile.id and C.ok_tint or C.card_hover end,
      border_width = 1,
      border_color = function() return page.profile:get() == profile.id and C.ok:alpha(0.5) or C.edge end,
      on_click = function() page.set_profile(profile.id) end,
      ui.Column {
        gap = S(4), anchors = { center_in = true },
        ui.Item { width = third - S(20), height = S(18),
          theme.icon { text = profile.glyph, size = config.iconSize - 2, anchors = { center_in = true },
            color = function() return page.profile:get() == profile.id and C.ok or C.muted end } },
        ui.Item { width = third - S(20), height = S(16),
          theme.text { text = profile.label, size = config.fontSize - 3, anchors = { center_in = true },
            color = function() return page.profile:get() == profile.id and C.fg or C.muted end } },
      },
    }
  end
  local function stat(label, value)
    return theme.card {
      width = third, height = S(46),
      ui.Column {
        gap = S(2), anchors = { center_in = true },
        ui.Item { width = third - S(20), height = S(14),
          theme.text { text = label, size = config.fontSize - 5, color = C.muted, anchors = { center_in = true } } },
        ui.Item { width = third - S(20), height = S(18),
          theme.text { text = value, size = config.fontSize - 1, font_weight = 700, anchors = { center_in = true } } },
      },
    }
  end
  return ui.Column {
    gap = S(10),
    theme.card {
      width = W, height = S(64),
      ui.Row {
        gap = S(14), align = "center", height = S(64),
        anchors = { left = true, left_margin = S(14) },
        (function()
          local icon_slot = ui.Item { width = S(config.iconSize + 2), height = S(config.iconSize + 2) }
          page.slots.battery_glyph = { node = icon_slot, size = config.iconSize + 2 }
          return icon_slot
        end)(),
        ui.Column {
          gap = S(2),
          (function()
            local text_slot = ui.Item { width = S(80), height = S((config.fontSize + 4) * 1.3) }
            page.slots.battery_text = { node = text_slot, size = config.fontSize + 4, weight = 700 }
            return text_slot
          end)(),
          theme.text {
            size = config.fontSize - 3, color = C.muted,
            text = function()
              if state.battery.charging then return "Charging" end
              local profile = page.profile:get()
              if profile ~= "" then return pretty(profile) end
              return state.battery.plugged and "Plugged in" or "On battery"
            end,
          },
        },
      },
    },
    ui.Row {
      gap = S(8),
      profile_button(PROFILES[1]), profile_button(PROFILES[2]), profile_button(PROFILES[3]),
    },
    theme.card {
      width = W, height = S(40), border_width = 1, border_color = C.edge,
      visible = function() return not page.info.profiles end,
      theme.text { text = "Profiles need power-profiles-daemon, which is not running", size = config.fontSize - 4,
        color = C.muted, anchors = { left = true, left_margin = S(14), top = true, top_margin = S(12) } },
    },
    ui.Row {
      gap = S(8),
      stat("Capacity", function() return page.info.capacity end),
      stat("Health", function() return page.info.health end),
      stat("Power", function() return page.info.rate end),
    },
  }
end

return page
