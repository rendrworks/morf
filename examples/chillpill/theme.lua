-- The look: near-black pills on whatever the wallpaper is, a pixel face for
-- words and a Nerd Font for icons, one scale for everything.
--
-- Every size in the shell goes through `S`, so a 4K output and a laptop
-- panel get the same picture at different pixel counts. `dpiScale = "auto"`
-- derives it from the output's height; a number in the config is used as is.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")

local theme = {}

-- ------------------------------------------------------------------ scale --

local screen = (morf.screens or {})[1] or {}
local screen_height = tonumber(screen.height) or 1080
local screen_scale = tonumber(screen.scale) or 1

local scale = config.dpiScale
if type(scale) ~= "number" then
  -- The original is drawn for a 1080p-ish output; above that the pill would
  -- be a sliver. Logical height, so a scaled output is not scaled twice.
  local logical = screen_height / screen_scale
  scale = math.max(1, math.min(2, logical / 1700))
  scale = math.floor(scale * 4 + 0.5) / 4
end
theme.scale = scale

--- A design pixel at this output's scale, rounded to a whole pixel.
function theme.S(value)
  return math.floor(value * scale + 0.5)
end
local S = theme.S

theme.WIDTH = tonumber(screen.width) or 1920
theme.HEIGHT = screen_height

-- ----------------------------------------------------------------- colours --

theme.color = {
  pill = morf.color "#161616",       -- the bar and every panel
  card = morf.color "#1c1c1c",       -- a box inside a panel
  button = morf.color "#232323",     -- a pressable box inside a card
  hover = morf.color "#2c2c2c",
  disc = morf.color "#3a3a3a",       -- an occupied workspace
  disc_active = morf.color "#555555",
  track = morf.color "#3a3a3a",      -- the empty part of a slider
  fill = morf.color "#dcdcdc",       -- the full part of a slider
  text = morf.color "#ececec",
  dim = morf.color "#8c8c8c",
  faint = morf.color "#5a5a5a",
  green = morf.color "#4ade80",
  blue = morf.color "#4f7be8",
  red = morf.color "#e5484d",
  orange = morf.color "#f5a524",
  yellow = morf.color "#fbbf24",
  edge = morf.color "#2a2a2a",       -- the one-pixel line round a card
}

-- ------------------------------------------------------------------- fonts --

theme.font = config.fontFamily
theme.font_source = core.shell_path("assets/fonts")

--- The first installed Nerd Font, unless the config names one.
local function find_icon_font()
  if config.iconFont ~= "auto" and config.iconFont ~= "" then return config.iconFont end
  local installed = {}
  for _, name in ipairs(morf.font_families()) do installed[name] = true end
  for _, wanted in ipairs {
    -- The Propo and plain faces first: their glyphs are as wide as they
    -- look, where a Mono face squeezes each icon into one cell and lets
    -- the ink spill over the letter after it.
    "JetBrainsMono Nerd Font Propo", "JetBrainsMono Nerd Font",
    "Iosevka Nerd Font Propo", "Iosevka Nerd Font",
    "Symbols Nerd Font", "JetBrainsMono NF", "Iosevka NF",
    "JetBrainsMono Nerd Font Mono", "Iosevka Nerd Font Mono",
  } do
    if installed[wanted] then return wanted end
  end
  for name in pairs(installed) do
    if name:find("Nerd Font") or name:find(" NF") then return name end
  end
  return theme.font
end
theme.icon_font = find_icon_font()

-- ------------------------------------------------------------------- nodes --

--- Words, in the pixel face. `size` is a design size.
function theme.text(values)
  values.font_family = theme.font
  values.font_source = theme.font_source
  values.font_size = S(values.size or 13)
  values.size = nil
  values.color = values.color or theme.color.text
  values.font_weight = values.font_weight or 400
  return ui.Text(values)
end

--- One Nerd Font glyph.
function theme.icon(values)
  values.font_family = theme.icon_font
  values.font_size = S(values.size or 14)
  values.size = nil
  values.color = values.color or theme.color.text
  return ui.Text(values)
end

--- A rounded dark box: the pill, a panel, a card.
function theme.box(values)
  values.color = values.color or theme.color.pill
  if values.radius then values.radius = S(values.radius) end
  return ui.Rect(values)
end

--- A box with `pad` pixels round its one child, sized by that child.
function theme.padded(values)
  local pad = values.pad or S(24)
  values.pad = nil
  local child = values[1]
  values[1] = ui.Inset { margin = pad, child }
  return theme.box(values)
end

--- A box that lightens under the pointer and darkens under a press.
---
--- `values.on_click` is the action; the rest is the Rect.
function theme.button(values)
  local on_click = values.on_click
  local base = values.color or theme.color.button
  local hover = values.hover_color or theme.color.hover
  local hovered = morf.signal("chillpill.hover." .. tostring(values.key or {}), false)
  local down = morf.signal("chillpill.down." .. tostring(values.key or {}), false)
  values.on_click, values.hover_color, values.key = nil, nil, nil
  local function paint(value)
    if type(value) == "function" then value = value() end
    if type(value) == "string" then value = morf.color(value) end
    return value
  end
  values.color = function()
    local rest, lit = paint(base), paint(hover)
    if down:get() then return rest:mix(lit, 0.5) end
    return hovered:get() and lit or rest
  end
  values.scale = function() return down:get() and 0.96 or 1 end
  values.behavior = values.behavior or { color = { duration = 90 }, scale = theme.motion.snappy }
  if values.radius then values.radius = S(values.radius) end
  values[#values + 1] = ui.MouseArea {
    anchors = { fill = true },
    cursor = "pointer",
    on_entered = function() hovered:set(true) end,
    on_exited = function() hovered:set(false) down:set(false) end,
    on_pressed = function() down:set(true) end,
    on_released = function() down:set(false) end,
    on_clicked = function() if on_click then on_click() end end,
  }
  return ui.Rect(values)
end

--- A horizontal bar with a lighter part up to `fraction`.
function theme.meter(values)
  local fraction = values.fraction
  local width = values.width
  local height = values.height or S(6)
  local fill_color = values.fill or theme.color.fill
  values.fraction, values.fill = nil, nil
  values.height = height
  values.radius = height / 2
  values.color = values.color or theme.color.track
  values[#values + 1] = ui.Rect {
    height = height,
    radius = height / 2,
    color = fill_color,
    anchors = { left = true, top = true },
    width = function()
      local value = type(fraction) == "function" and fraction() or fraction
      local total = type(width) == "function" and width() or width
      return math.max(height, math.floor(math.max(0, math.min(1, value or 0)) * total))
    end,
    behavior = { width = { duration = 160, easing = "out_cubic" } },
  }
  return ui.Rect(values)
end

-- ------------------------------------------------------------------ motion --

-- One vocabulary of movement, so every panel, button and pill moves the
-- same way: a spring for position and size, a short ease for opacity.
theme.motion = {
  spring = { kind = "spring", stiffness = 280, damping = 26, mass = 1 },
  soft = { kind = "spring", stiffness = 200, damping = 22, mass = 1 },
  snappy = { kind = "spring", stiffness = 420, damping = 30, mass = 1 },
  fade = { duration = 180, easing = "out_quad" },
  quick = { duration = 110, easing = "out_quad" },
}
local motion = theme.motion

local mounted_count = 0

--- A signal that goes up the moment `shown` does and down `delay`
--- milliseconds after it, so a node can play its way out before it is
--- hidden.
function theme.mounted(shown, delay)
  mounted_count = mounted_count + 1
  local mounted = morf.signal("chillpill.mounted." .. mounted_count, shown:get())
  local clock = morf.elapsed_timer()
  local going = false
  morf.timer(24, function()
    local want = shown:get()
    if want then
      going = false
      if not mounted:get() then mounted:set(true) end
    elseif mounted:get() then
      if not going then
        going = true
        clock:restart()
      elseif clock:elapsed_ms() >= (delay or 260) then
        mounted:set(false)
      end
    end
  end, true)
  return mounted
end

--- Fills `values` with the motion of a panel that comes and goes with
--- `shown`: it fades, slides in from `from_y` / `from_x` and grows from
--- `from_scale`, and stays in the tree until the way out has played.
function theme.reveal(shown, values)
  local from_y = values.from_y or 0
  local from_x = values.from_x or 0
  local from_scale = values.from_scale or 0.96
  values.from_y, values.from_x, values.from_scale = nil, nil, nil
  local mounted = theme.mounted(shown, values.delay or 260)
  values.delay = nil
  values.visible = function() return mounted:get() end
  values.opacity = function() return shown:get() and 1 or 0 end
  values.translate_y = function() return shown:get() and 0 or from_y end
  values.translate_x = function() return shown:get() and 0 or from_x end
  values.scale = function() return shown:get() and 1 or from_scale end
  values.behavior = {
    opacity = motion.fade,
    translate_y = motion.spring,
    translate_x = motion.spring,
    scale = motion.spring,
  }
  return values
end

--- Opening and closing a surface of its own with motion: the root fades
--- and grows in, shrinks and fades out, and the surface closes once that
--- has played. The root starts at `opacity = 0, scale = 0.96` with
--- behaviors on both.
function theme.surface_motion(window, root)
  local closing = false
  local motionless = {}
  function motionless.open()
    closing = false
    window:open()
    -- A tick later, so the write is its own flush and the behavior runs
    -- from the hidden state rather than landing there.
    morf.timer(16, function()
      if not closing then
        root.opacity = 1
        root.scale = 1
      end
    end, false)
  end
  function motionless.close()
    if closing then return end
    closing = true
    root.opacity = 0
    root.scale = 0.96
    morf.timer(170, function()
      if closing then window:close() end
    end, false)
  end
  return motionless
end

--- The pointer's fraction along a meter, from a surface x and the meter's
--- left edge and width.
function theme.fraction_at(x, left, width)
  return math.max(0, math.min(1, (x - left) / width))
end

return theme
