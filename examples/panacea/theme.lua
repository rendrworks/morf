-- The look: black, one text colour dimmed for the quiet parts, one accent,
-- a mono face, and a scale.
--
-- Panacea's numbers are for a 1080p screen. `S` scales every one of them;
-- `dpiScale = "auto"` derives the scale from the output's height.

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
  local logical = screen_height / screen_scale
  scale = math.max(1, math.min(2, logical / 1700))
  scale = math.floor(scale * 4 + 0.5) / 4
end
theme.scale = scale

function theme.S(value)
  return math.floor(value * scale + 0.5)
end
local S = theme.S

theme.WIDTH = tonumber(screen.width) or 1920
theme.HEIGHT = screen_height

-- ----------------------------------------------------------------- colours --

local fg = morf.color(config.colFg)
local on = morf.color(config.colOn)
if config.themeId == "nothing" then on = fg end

theme.color = {
  bg = morf.color "#000000",
  fg = fg,
  muted = fg:alpha(config.mutedAlpha),
  faint = fg:alpha(0.25),
  edge = fg:alpha(0.1),
  card = fg:alpha(0.08),
  card_hover = fg:alpha(0.13),
  card_down = fg:alpha(0.18),
  on = on,
  on_tint = on:alpha(0.22),
  on_edge = on:alpha(0.55),
  ok = morf.color "#22c55e",
  ok_tint = morf.color("#22c55e"):alpha(0.22),
  warn = morf.color "#fb923c",
  crit = morf.color(config.themeId == "nothing" and "#d71921" or "#ef4444"),
  crit_tint = morf.color("#ef4444"):alpha(0.22),
  coffee = morf.color "#f59e0b",
  coffee_tint = morf.color("#f59e0b"):alpha(0.2),
  -- The same names the shared modules use.
  text = fg,
  dim = fg:alpha(config.mutedAlpha),
  pill = morf.color "#000000",
  button = fg:alpha(0.08),
  hover = fg:alpha(0.13),
  blue = on,
  green = morf.color "#22c55e",
  red = morf.color "#ef4444",
}
local C = theme.color

-- ------------------------------------------------------------------- fonts --

--- The configured face if installed, else the first Nerd Font, else
--- whatever monospace there is.
local function pick_font()
  local installed = {}
  for _, name in ipairs(morf.font_families()) do installed[name] = true end
  if installed[config.fontFam] then return config.fontFam end
  for _, wanted in ipairs {
    "JetBrainsMono Nerd Font", "JetBrainsMono Nerd Font Propo", "JetBrainsMono NF",
    "Iosevka Nerd Font", "Iosevka Nerd Font Propo", "Iosevka NF",
    "FiraCode Nerd Font", "Hack Nerd Font", "Symbols Nerd Font",
  } do
    if installed[wanted] then return wanted end
  end
  for name in pairs(installed) do
    if name:find("Nerd Font") then return name end
  end
  return "monospace"
end
theme.font = pick_font()
theme.icon_font = theme.font

theme.clock = core.system_clock { precision = "seconds" }

-- ------------------------------------------------------------------ motion --

-- Panacea moves in 230 ms with a 79 percent bounce: a spring that overshoots
-- a little and settles. `reduceMotion` makes every move land at once.
local bounce = math.max(0, math.min(100, config.animBounce or 79)) / 100
local move_ms = config.reduceMotion and 1 or (config.animMove or config.animMs or 230)
local fade_ms = config.reduceMotion and 1 or (config.animFade or 200)
local hover_ms = config.reduceMotion and 1 or (config.animHover or 150)

-- A spring whose settle time is about the configured move and whose
-- damping follows the bounce: 79 percent is a light overshoot.
local stiffness = 180 * (230 / move_ms)
local damping = 30 - 14 * bounce
theme.motion = {
  move = { kind = "spring", stiffness = stiffness, damping = damping, mass = 1 },
  soft = { kind = "spring", stiffness = stiffness * 0.6, damping = damping * 0.9, mass = 1 },
  fade = { duration = fade_ms, easing = "out_cubic" },
  hover = { duration = hover_ms, easing = "out_cubic" },
  -- The shared modules' names.
  spring = { kind = "spring", stiffness = stiffness, damping = damping, mass = 1 },
  snappy = { kind = "spring", stiffness = stiffness * 1.6, damping = damping, mass = 1 },
  quick = { duration = hover_ms, easing = "out_cubic" },
}
local motion = theme.motion
theme.move_ms = move_ms

-- ------------------------------------------------------------------- nodes --

--- Words. `size` is a design size; the default is the configured text size.
function theme.text(values)
  values.font_family = theme.font
  values.font_size = S(values.size or config.fontSize)
  values.size = nil
  values.color = values.color or C.fg
  values.font_weight = values.font_weight or 400
  return ui.Text(values)
end

--- A glyph, at the icon size unless told otherwise.
function theme.icon(values)
  values.font_family = theme.icon_font
  values.font_size = S(values.size or config.iconSize)
  values.size = nil
  values.color = values.color or C.fg
  return ui.Text(values)
end

--- A small heading in caps: SIZES, FONT, TRAY.
function theme.label(values)
  values.text = type(values.text) == "string" and values.text:upper() or values.text
  values.size = values.size or (config.fontSize - 4)
  values.color = values.color or C.muted
  values.letter_spacing = S(1.5)
  values.font_weight = 700
  return theme.text(values)
end

--- A translucent rounded box: every tile, card and button.
function theme.card(values)
  values.color = values.color or C.card
  values.radius = S(values.radius or config.cornerR)
  return ui.Rect(values)
end

local counter = 0

--- A card that answers the pointer: lighter under it, darker pressed, and
--- runs `on_click`. `tint` and `tint_edge` colour it when it is "on".
function theme.button(values)
  counter = counter + 1
  local hovered = morf.signal("panacea.button.hover." .. counter, false)
  local down = morf.signal("panacea.button.down." .. counter, false)
  local on_click = values.on_click
  local base = values.color or C.card
  local hover = values.hover_color or C.card_hover
  values.on_click, values.hover_color = nil, nil
  local function paint(value)
    if type(value) == "function" then value = value() end
    if type(value) == "string" then value = morf.color(value) end
    return value
  end
  values.color = function()
    local rest, lit = paint(base), paint(hover)
    if down:get() then return rest:mix(lit, 0.6) end
    return hovered:get() and lit or rest
  end
  values.scale = function() return down:get() and 0.97 or 1 end
  values.behavior = values.behavior or { color = motion.hover, scale = motion.snappy }
  values.radius = S(values.radius or config.cornerR)
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

--- The pill switch: a track with a knob that slides to the right when on.
function theme.toggle(on, set)
  local W, H = S(44), S(24)
  local KNOB = S(18)
  return ui.Rect {
    width = W, height = H, radius = H / 2,
    color = function() return on() and C.on or C.faint end,
    behavior = { color = motion.hover },
    ui.Rect {
      width = KNOB, height = KNOB, radius = KNOB / 2, color = C.fg,
      y = (H - KNOB) / 2,
      x = function() return on() and (W - KNOB - S(3)) or S(3) end,
      behavior = { x = motion.move },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_clicked = function() set(not on()) end,
    },
  }
end

--- A slider: track, fill and a round knob. `fraction()` reads, `set(f)`
--- writes on press, drag and wheel.
function theme.slider(values)
  local width = values.width
  local height = values.height or S(22)
  local track_h = values.track or S(6)
  local knob = values.knob or S(14)
  local fraction, set = values.fraction, values.set
  local dragging = false
  local function at(local_x) return math.max(0, math.min(1, (local_x - knob / 2) / (width - knob))) end
  return ui.Item {
    width = width, height = height,
    ui.Rect {
      width = width, height = track_h, radius = track_h / 2, color = C.faint,
      y = (height - track_h) / 2,
      ui.Rect {
        height = track_h, radius = track_h / 2, color = values.color or C.on,
        width = function() return math.max(track_h, fraction() * width) end,
        behavior = { width = motion.hover },
      },
    },
    ui.Rect {
      width = knob, height = knob, radius = knob / 2, color = C.fg,
      y = (height - knob) / 2,
      x = function() return fraction() * (width - knob) end,
      behavior = { x = motion.hover },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_pressed = function(_, _, local_x) dragging = true set(at(local_x)) end,
      on_dragged = function(_, _, local_x) if dragging then set(at(local_x)) end end,
      on_released = function() dragging = false end,
      on_wheel = function(_, _, _, _, _, steps_y)
        if steps_y ~= 0 then set(math.max(0, math.min(1, fraction() - steps_y * 0.05))) end
      end,
    },
  }
end

--- A row of choices, one lit: 30 / 60 / 120, ~/Videos / ~/Pictures.
function theme.chips(options, current, choose)
  local nodes = {}
  for index, option in ipairs(options) do
    local value, label = option, tostring(option)
    if type(option) == "table" then
      value, label = option.value, option.label or tostring(option.value)
    end
    nodes[index] = theme.button {
      height = S(30), radius = 10,
      color = function() return current() == value and C.on_tint or C.card end,
      hover_color = function() return current() == value and C.on_tint or C.card_hover end,
      on_click = function() choose(value) end,
      ui.Row {
        height = S(30), align = "center",
        ui.Item { width = S(14), height = 1 },
        theme.text {
          text = label, size = config.fontSize - 2,
          color = function() return current() == value and C.fg or C.muted end,
          font_weight = function() return current() == value and 700 or 400 end,
        },
        ui.Item { width = S(14), height = 1 },
      },
    }
  end
  return ui.Row { gap = S(8), table.unpack(nodes) }
end

-- A signal that follows `shown` up at once and down after `delay` ms.
local mounted_count = 0
function theme.mounted(shown, delay)
  mounted_count = mounted_count + 1
  local mounted = morf.signal("panacea.mounted." .. mounted_count, shown:get())
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
      elseif clock:elapsed_ms() >= (delay or move_ms * 2) then
        mounted:set(false)
      end
    end
  end, true)
  return mounted
end

--- Fills `values` with the motion of a node that comes and goes with
--- `shown`: a fade and a small rise, kept in the tree until it has gone.
function theme.reveal(shown, values)
  local from_y = values.from_y or S(10)
  local mounted = theme.mounted(shown, values.delay)
  values.from_y, values.delay = nil, nil
  -- Visible the moment it is shown, and until the way out has played.
  values.visible = function() return shown:get() or mounted:get() end
  values.opacity = function() return shown:get() and 1 or 0 end
  values.translate_y = function() return shown:get() and 0 or from_y end
  values.behavior = { opacity = motion.fade, translate_y = motion.move }
  return values
end

return theme
