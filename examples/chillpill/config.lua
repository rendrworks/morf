-- Settings, read once from `~/.config/chillpill/config.json`.
--
-- The keys are ChillPill's own, so a config.jsonc written for the original
-- reads here unchanged: comments are stripped before the JSON is parsed, and
-- anything the file leaves out takes the default below.

local core = require("morf.core")
local io = require("morf.io")

local config = {
  -- A picture for the dashboard's profile; empty tries `~/.face`.
  displayPicture = "",
  -- "24h" or "12h".
  clockFormat = "24h",
  -- What the pill shows, left to right. Also: bluetooth, weather, vpn.
  pillModules = { "battery", "volume", "workspaces", "network", "clock" },
  -- The gap above the pill, and the same below it: what the pill reserves
  -- is the pill and one gap each side.
  pillTopMargin = 9,
  pillBottomMargin = 9,
  pillScale = 1.0,
  -- Keep the pill out of the way until the pointer reaches the top edge.
  pillOnHover = false,
  -- A number, or "auto": 1 on a laptop panel, up to 2 on a 4K output.
  dpiScale = "auto",
  -- Minutes, cycled by the timer button in the control center.
  timerPresets = { 1, 5, 10, 15, 30 },
  mediaPopupDuration = 2000,
  maxWorkspaces = 5,
  notificationDisplayTime = 3000,
  maxNotificationsInStack = 20,
  osdDuration = 800,
  weatherLocation = "Delhi",
  weatherUnits = "metric",
  -- Two letters, for the public holidays the calendar marks; "" for none.
  country = "IN",
  defaultTerminal = "kitty",
  wallpapersDir = "~/Pictures/wallpapers",
  screenLockAppCommand = "hyprlock",
  -- IP address and SSID on the dashboard and pill.
  showSensitiveInfo = true,
  -- Fonts. The text face ships with the example; the icon face is any Nerd
  -- Font, found among the installed ones when left at "auto".
  fontFamily = "Monocraft",
  iconFont = "auto",
  -- Notify when the session has downloaded this much, in MB; 0 is never.
  bandwidthAlertMB = 0,
}

local home = core.env("HOME") or ""

--- `~` at the start of a path is the home directory.
function config.expand(path)
  if type(path) ~= "string" then return "" end
  if path:sub(1, 2) == "~/" then return home .. path:sub(2) end
  if path == "~" then return home end
  return path
end

--- JSON with `//` and `/* */` comments, and trailing commas, is jsonc.
local function strip_comments(text)
  text = text:gsub("/%*.-%*/", "")
  local out = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do
    -- A `//` inside a string is left alone: only one outside quotes ends
    -- the line.
    local in_string, cut = false, nil
    local index = 1
    while index <= #line do
      local char = line:sub(index, index)
      if char == "\\" and in_string then
        index = index + 1
      elseif char == '"' then
        in_string = not in_string
      elseif char == "/" and not in_string and line:sub(index + 1, index + 1) == "/" then
        cut = index
        break
      end
      index = index + 1
    end
    out[#out + 1] = cut and line:sub(1, cut - 1) or line
  end
  text = table.concat(out, "\n")
  return (text:gsub(",%s*([}%]])", "%1"))
end

local path = (core.env("XDG_CONFIG_HOME") or (home .. "/.config")) .. "/chillpill/config.json"
local ok, handle = pcall(io.file, path)
if ok and handle then
  local read, text = pcall(handle.read, handle)
  if read and type(text) == "string" and text ~= "" then
    local decoded, values = pcall(io.json.decode, strip_comments(text))
    if decoded and type(values) == "table" then
      for key, value in pairs(values) do
        if config[key] ~= nil and type(value) == type(config[key]) then
          config[key] = value
        end
      end
    end
  end
end

config.path = path
config.home = home
config.wallpapersDir = config.expand(config.wallpapersDir)
config.displayPicture = config.expand(config.displayPicture)

return config
