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

function page.build(island)
  local W = S(config.panelW) - S(32)
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
    ui.Item {
      width = W, height = S(28),
      theme.text { text = "Battery", font_weight = 700, size = config.fontSize + 1, anchors = { left = true, left_margin = S(4), top = true, top_margin = S(4) } },
      ui.Item {
        width = S(28), height = S(28), anchors = { right = true },
        theme.icon { text = "󰑐", size = config.iconSize - 3, color = C.muted, anchors = { center_in = true } },
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = poll },
      },
    },
    theme.card {
      width = W, height = S(64),
      ui.Row {
        gap = S(14), align = "center", height = S(64),
        anchors = { left = true, left_margin = S(14) },
        theme.icon { text = function() return require("bar_glyphs").battery_glyph() end, size = config.iconSize + 2,
          color = function() return state.battery.charging and C.ok or C.fg end },
        ui.Column {
          gap = S(2),
          theme.text { text = function() return state.battery.present and (state.battery.percent .. "%") or "No battery" end,
            size = config.fontSize + 4, font_weight = 700 },
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
    theme.text {
      text = "power-profiles-daemon is not running", size = config.fontSize - 4, color = C.muted,
      visible = function() return not page.info.profiles end,
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
