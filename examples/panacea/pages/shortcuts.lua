-- Every binding in one place, from the `bind_*` keys of the settings.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

local S = theme.S
local C = theme.color

local page = {}

local NAMES = {
  { "bind_pillControls", "Quick settings" },
  { "bind_pillLauncher", "Launcher" },
  { "bind_pillClip", "Clipboard" },
  { "bind_pillWifi", "Networks" },
  { "bind_pillBt", "Bluetooth" },
  { "bind_pillSettings", "Settings" },
  { "bind_pillPower", "Power menu" },
  { "bind_terminal", "Terminal" },
  { "bind_browser", "Browser" },
  { "bind_fileManager", "File manager" },
  { "bind_closeWindow", "Close window" },
  { "bind_fullscreen", "Fullscreen" },
  { "bind_screenshot", "Screenshot" },
  { "bind_emptyWorkspace", "Empty workspace" },
  { "bind_specialWorkspace", "Special workspace" },
  { "bind_toggleSplit", "Toggle split" },
  { "bind_themeSwitch", "Switch theme" },
  { "bind_exitHypr", "Exit Hyprland" },
}

function page.width() return S(config.panelW * 1.3) end

function page.build(island)
  local W = page.width() - S(32)
  local half = math.floor((W - S(12)) / 2)
  local left, right = {}, {}
  for index, pair in ipairs(NAMES) do
    local keys = config[pair[1]]
    if type(keys) == "string" and keys ~= "" then
      local chips = {}
      for part in keys:gmatch("[^+]+") do
        local word = part:gsub("^%s+", ""):gsub("%s+$", "")
        chips[#chips + 1] = ui.Rect {
          height = S(24), radius = S(6), color = C.card_hover,
          ui.Row {
            height = S(24), align = "center",
            ui.Item { width = S(8), height = 1 },
            theme.text { text = word, size = config.fontSize - 4, font_weight = 700 },
            ui.Item { width = S(8), height = 1 },
          },
        }
      end
      local row = ui.Item {
        width = half, height = S(34),
        ui.Row { gap = S(4), align = "center", height = S(34), anchors = { left = true }, table.unpack(chips) },
        theme.text { text = pair[2], size = config.fontSize - 3, color = C.muted, anchors = { right = true, top = true, top_margin = S(9) } },
      }
      if index % 2 == 1 then left[#left + 1] = row else right[#right + 1] = row end
    end
  end
  return ui.Column {
    gap = S(10),
    theme.text { text = "Shortcuts", font_weight = 700, size = config.fontSize + 1 },
    theme.text { text = "From ~/.config/panacea/settings.json; Hyprland reads the same keys.", size = config.fontSize - 4, color = C.muted },
    ui.Row {
      gap = S(12),
      ui.Column { gap = S(2), table.unpack(left) },
      ui.Column { gap = S(2), table.unpack(right) },
    },
  }
end

return page
