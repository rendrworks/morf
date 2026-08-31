-- Shared pywal palette and geometry tokens.
--
-- Ported from the `Theme.qml` singletons in `~/.config/quickshell/settings` and
-- `~/.config/quickshell/osd`, which are the same file twice. Every size is
-- expressed against a 2160px reference short side, so the shell keeps its
-- proportions on any output rather than being pinned to one panel.

local morf = require("morf")
local io = require("morf.io")
local core = require("morf.core")

local theme = {}

local FALLBACK = {
  color0 = "#000000",
  color1 = "#ffffff",
  color236 = "#1e1e1e",
  color238 = "#2a2a2a",
  color240 = "#303030",
  color244 = "#555555",
}

local colors = {}
for key, value in pairs(FALLBACK) do colors[key] = value end

-- Bumped whenever the palette is reloaded, so bindings that read a colour can
-- depend on it without the palette itself having to be a signal.
theme.revision = morf.signal("quickshell.theme.revision", 0)

local wal = io.file_view {
  path = (core.env("HOME") or "") .. "/.cache/wal/colors.json",
  preload = true,
  watch_changes = true,
}

--- Rereads the palette, returning whether anything changed.
function theme.reload()
  if not wal:loaded() then return false end
  local ok, decoded = pcall(io.json.decode, wal:text())
  if not ok or type(decoded) ~= "table" then return false end
  local changed = false
  for key in pairs(FALLBACK) do
    local value = decoded.colors and decoded.colors[key]
    if type(value) == "string" and value ~= colors[key] then
      colors[key] = value
      changed = true
    end
  end
  if changed then
    theme.revision:set(theme.revision:get() + 1)
  end
  return changed
end

--- Reads one palette entry, registering the caller as a palette dependent.
function theme.color(name)
  theme.revision:get()
  return colors[name] or FALLBACK[name] or "#ff00ff"
end

function theme.color0() return theme.color("color0") end
function theme.color1() return theme.color("color1") end
function theme.color236() return theme.color("color236") end
function theme.color238() return theme.color("color238") end
function theme.color240() return theme.color("color240") end
function theme.color244() return theme.color("color244") end

--- Adds an alpha channel to a palette entry, as `#rrggbbaa`.
function theme.alpha(name, amount)
  local value = math.floor(math.max(0, math.min(1, amount)) * 255 + 0.5)
  return string.format("%s%02x", theme.color(name):sub(1, 7), value)
end

--- Picks black or white for legibility against a background, matching the
--- luminance test in `Numbers.qml`.
function theme.readable(hex)
  local raw = tostring(hex):gsub("#", "")
  if #raw < 6 then return "#ffffff" end
  local r = tonumber(raw:sub(1, 2), 16) or 255
  local g = tonumber(raw:sub(3, 4), 16) or 255
  local b = tonumber(raw:sub(5, 6), 16) or 255
  local luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255
  return luminance < 0.45 and "#ffffff" or "#000000"
end

--- The first reported output, which every size is measured against.
function theme.reference()
  local screens = morf.screens or {}
  local screen = screens[1]
  local width = screen and screen.width or 3840
  local height = screen and screen.height or 2160
  return width, height
end

--- Scales a design value given against a 2160px short side.
function theme.scaled(value)
  local width, height = theme.reference()
  return math.min(width, height) * (value / 2160)
end

theme.font = "IosevkaTerm Nerd Font Mono"
theme.font_source = core.shell_path("../board/assets/fonts")

-- Geometry tokens, matching settings/Theme.qml.
function theme.corner_radius() return theme.scaled(10) end
function theme.panel_radius() return theme.scaled(12) end
function theme.pill_radius() return theme.scaled(4) end
function theme.border_width() return theme.scaled(1) end
function theme.heavy_border_width() return theme.scaled(2) end
function theme.morph_border_growth() return theme.scaled(4) end
function theme.tiny_radius() return theme.scaled(1) end

-- OSD tokens, matching osd/Theme.qml.
function theme.spacing() return theme.scaled(12) end
function theme.icon_size() return theme.scaled(36) end
function theme.progress_radius() return theme.scaled(8) end
function theme.font_size_small() return theme.scaled(18) end
function theme.font_size_medium() return theme.scaled(22) end
function theme.font_size_large() return theme.scaled(28) end

theme.short_duration = 200
theme.medium_duration = 300
theme.long_duration = 400

theme.reload()

return theme
