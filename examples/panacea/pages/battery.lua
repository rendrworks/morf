-- The battery: charge and state, the power profile, capacity, health and
-- the power being drawn. Profiles go through power-profiles-daemon over
-- D-Bus; the numbers come from upower.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")
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
  local third = kit.cell(W, 3)
  -- The strip's glyph and figure land on this row.
  local icon_slot = ui.Item { width = S(config.iconSize + 2), height = S(config.iconSize + 2), anchors = { center_in = true } }
  local circle = ui.Rect {
    width = kit.CIRCLE, height = kit.CIRCLE, radius = kit.CIRCLE / 2,
    color = function() return state.battery.charging and C.ok or C.card_hover end,
    behavior = { color = theme.motion.hover },
    icon_slot,
  }
  page.slots.battery_glyph = { node = icon_slot, size = config.iconSize + 2 }
  local text_slot = ui.Item { width = S(120), height = S((config.fontSize + 4) * 1.3) }
  page.slots.battery_text = { node = text_slot, size = config.fontSize + 4, weight = 700 }
  local profiles = {}
  for _, profile in ipairs(PROFILES) do profiles[#profiles + 1] = { label = profile.label, value = profile.id } end
  return kit.page(W, {
    kit.row {
      width = W, height = S(64),
      icon_node = circle, title_node = text_slot,
      subtitle = function()
        if state.battery.charging then return "Charging" end
        local profile = page.profile:get()
        if profile ~= "" then return pretty(profile) end
        return state.battery.plugged and "Plugged in" or "On battery"
      end,
      active = function() return state.battery.charging end, accent = C.ok, tint = C.ok_tint, edge = C.ok:alpha(0.5),
    },
    kit.choice_row { width = W, title = "Profile", options = profiles,
      current = function() return page.profile:get() end, choose = page.set_profile },
    kit.empty("Profiles need power-profiles-daemon, which is not running", function() return not page.info.profiles end),
    kit.grid(W, 3, {
      kit.stat(third, "Capacity", function() return page.info.capacity end),
      kit.stat(third, "Health", function() return page.info.health end),
      kit.stat(third, "Power", function() return page.info.rate end),
    }),
  })
end

return page
