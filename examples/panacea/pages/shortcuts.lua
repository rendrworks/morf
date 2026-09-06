-- Every binding in one place, from the `bind_*` keys of the settings.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")
local kit = require("kit")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Shortcuts"
page.icon = "󰌌"
page.subtitle = "From ~/.config/panacea/settings.json; Hyprland reads the same keys."

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


function page.build(island)
  local W = theme.page_w()
  local rows = {}
  for _, pair in ipairs(NAMES) do
    local keys = config[pair[1]]
    if type(keys) == "string" and keys ~= "" then
      rows[#rows + 1] = kit.row {
        width = W, height = S(44),
        icon = "󰌌", title = pair[2], subtitle = nil,
        right = kit.figure(keys:gsub("%s*%+%s*", " + "), S(220)), right_w = S(220),
      }
    end
  end
  return kit.page(W, rows)
end

return page
