-- Shared pywal palette and geometry tokens.
--
-- Ported from the `Theme.qml` singletons in `~/.config/quickshell/settings` and
-- `~/.config/quickshell/osd`, which are the same file twice. Every size is
-- expressed against a 2160px reference short side, so the shell keeps its
-- proportions on any output rather than being pinned to one panel.

local morf = require("morf")
local core = require("morf.core")

local theme = {}

-- The pywal palette, with the same greys to fall back on. The file's leaf
-- keys are the tokens, read now and again whenever pywal rewrites it, so a
-- binding that reads a colour follows the palette with nothing to reload.
theme.palette = morf.theme({
  color0 = "#000000",
  color1 = "#ffffff",
  color236 = "#1e1e1e",
  color238 = "#2a2a2a",
  color240 = "#303030",
  color244 = "#555555",
}, { source = (core.env("HOME") or "") .. "/.cache/wal/colors.json" })

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

return theme
