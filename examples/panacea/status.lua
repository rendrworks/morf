-- The strip's status icons: what a phone's status bar shows at its right
-- end -- the modem's signal and generation, VPN, Wi-Fi, Bluetooth, do not
-- disturb, a muted microphone, the recorder -- as small glyphs that appear
-- only while they have something to say. `statusIcons` in the settings
-- picks and orders them; each is a name from `status.icons`. The battery
-- is not here: it is the strip's own piece, and it morphs.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local system = require("system")
local cellular = require("cellular")
local bar = require("bar_glyphs")

local S = theme.S
local C = theme.color
local state = system.state

local status = {}

--- Every icon there is: `glyph` and `color` are functions of none,
--- `visible` says whether it has anything to show, and `text` is a short
--- label beside the glyph (the modem's generation).
status.icons = {
  cellular = {
    visible = function() return cellular.state.present end,
    glyph = cellular.glyph,
    text = function() return cellular.state.tech end,
    color = function() return cellular.state.connected and C.fg or C.muted end,
  },
  vpn = {
    visible = function() return state.network.vpn end,
    glyph = function() return "󰦝" end,
    color = function() return C.ok end,
  },
  wifi = {
    visible = function() return state.network.kind ~= "none" end,
    glyph = bar.network_glyph,
    color = function() return C.fg end,
  },
  bluetooth = {
    visible = function() return state.bluetooth.powered end,
    glyph = function() return state.bluetooth.connected ~= "" and "󰂱" or "󰂯" end,
    color = function() return state.bluetooth.connected ~= "" and C.on or C.muted end,
  },
  dnd = {
    visible = function() return require("notify").silent:get() end,
    glyph = function() return "󰂛" end,
    color = function() return C.muted end,
  },
  mic = {
    visible = function() return require("tiles").state.mic_muted end,
    glyph = function() return "󰍭" end,
    color = function() return C.crit end,
  },
  volume = {
    visible = function() return state.volume.muted end,
    glyph = function() return "󰝟" end,
    color = function() return C.muted end,
  },
  caffeine = {
    visible = function() return require("tiles").state.caffeine end,
    glyph = function() return "󰅶" end,
    color = function() return C.coffee end,
  },
  airplane = {
    visible = function() return require("tiles").state.airplane end,
    glyph = function() return "󰀝" end,
    color = function() return C.warn end,
  },
  hotspot = {
    visible = function() return require("tiles").state.hotspot end,
    glyph = function() return "󰑩" end,
    color = function() return C.on end,
  },
}

--- The row of icons, `size` tall, `gap` apart, in the settings' order;
--- `values` are the row's own properties (where it sits, how it moves).
--- Every icon sits centred in a slot of the same width, so the row reads
--- as a row whatever the glyphs' own widths are.
function status.build(size, gap, values)
  local SLOT = size + S(6)
  local nodes = {}
  for _, name in ipairs(config.statusIcons or {}) do
    local icon = status.icons[name]
    if icon then
      nodes[#nodes + 1] = ui.Row {
        gap = S(2), align = "center",
        visible = icon.visible,
        ui.Item {
          width = SLOT, height = SLOT,
          theme.icon { text = icon.glyph, size = size, color = icon.color, anchors = { center_in = true } },
        },
        icon.text and theme.text { text = icon.text, size = config.fontSize - 6, font_weight = 700, color = icon.color,
          visible = function() return icon.text() ~= "" end } or nil,
      }
    end
  end
  values = values or {}
  values.direction, values.align, values.gap = "row", "center", gap
  for _, node in ipairs(nodes) do values[#values + 1] = node end
  return ui.Flex(values)
end

return status
