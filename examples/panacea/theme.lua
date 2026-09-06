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

-- Phone mode: a screen too narrow for the island to float in the middle
-- of. The strip becomes a status bar across the top, the clock at its
-- left and the icons at its right, and a page a sheet the width of the
-- screen under it.
theme.phone = theme.WIDTH < S(config.panelW) + S(160)

--- A page's width: the panel's, times `factor`, but never wider than the
--- screen; the whole screen on a phone.
function theme.panel_w(factor)
  if theme.phone then return theme.WIDTH end
  return math.min(S(config.panelW * (factor or 1)), theme.WIDTH - S(16))
end

--- The room inside a page, with its padding taken off.
function theme.page_w(factor)
  return theme.panel_w(factor) - S(32)
end

-- ----------------------------------------------------------------- colours --

-- lule's palette, when it has written one: the wallpaper's colours, kept
-- in ~/.cache/lule/colors.json as `special.background`, `.foreground`,
-- `.cursor` and sixteen `colors`. The cursor is the accent. Panacea's own
-- settings apply when there is no cache, or when `themeId` is "panacea".
local io = require("morf.io")
local lule = nil
do
  local cache = core.env("XDG_CACHE_HOME") or (config.home .. "/.cache")
  for _, path in ipairs { cache .. "/lule/colors.json", config.home .. "/.lule/wal/colors.json" } do
    local ok, handle = pcall(io.file, path)
    if ok and handle then
      local read, text = pcall(handle.read, handle)
      if read and type(text) == "string" and text ~= "" then
        local decoded, values = pcall(io.json.decode, text)
        if decoded and type(values) == "table" and type(values.special) == "table" then
          lule = values
          lule.path = path
          break
        end
      end
    end
  end
end
if config.themeId == "panacea" then lule = nil end
theme.lule = lule

local function colour_or(value, fallback)
  if type(value) == "string" and value:match("^#%x%x%x%x%x%x$") then return morf.color(value) end
  return morf.color(fallback)
end

local bg = lule and colour_or(lule.special.background, "#000000") or morf.color "#000000"
local fg = lule and colour_or(lule.special.foreground, config.colFg) or morf.color(config.colFg)
local on = lule and colour_or(lule.special.cursor or (lule.colors or {})[2], config.colOn) or morf.color(config.colOn)
if config.themeId == "nothing" then on = fg end
-- The island stays dark even on a light palette: a black capsule is the
-- shape of the thing.
if bg:luminance() > 0.5 then bg = bg:darken(0.6) end

theme.color = {
  bg = bg,
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
  pill = bg,
  button = fg:alpha(0.08),
  hover = fg:alpha(0.13),
  blue = on,
  green = morf.color "#22c55e",
  red = morf.color "#ef4444",
}
local C = theme.color

-- ------------------------------------------------------------------- fonts --

local installed = {}
for _, name in ipairs(morf.font_families()) do installed[name] = true end

--- The first of `wanted` that is installed, else `fallback`.
local function first_installed(wanted, fallback)
  for _, name in ipairs(wanted) do
    if installed[name] then return name end
  end
  for name in pairs(installed) do
    if name:find("Nerd Font") then return name end
  end
  return fallback
end

-- Words in the configured face if it is installed, else a Nerd Font. Icons
-- always from a Nerd Font, since a pixel face has no icons in it.
theme.font = installed[config.fontFam] and config.fontFam
  or first_installed({ "JetBrainsMono Nerd Font", "JetBrainsMono Nerd Font Propo", "Iosevka Nerd Font", "Iosevka NF" }, "monospace")
theme.icon_font = first_installed({
  "JetBrainsMono Nerd Font Propo", "JetBrainsMono Nerd Font", "Iosevka Nerd Font Propo", "Iosevka Nerd Font",
  "Symbols Nerd Font", "Iosevka NF",
}, theme.font)

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
  -- Under the pointer a button grows a little and lifts; pressed it sinks.
  values.scale = function() return down:get() and 0.97 or (hovered:get() and 1.025 or 1) end
  values.translate_y = values.translate_y or function() return hovered:get() and -S(1.5) or 0 end
  values.behavior = values.behavior or { color = motion.hover, scale = motion.snappy }
  values.behavior.scale = values.behavior.scale or motion.snappy
  values.behavior.translate_y = values.behavior.translate_y or motion.snappy
  local on_hover = values.on_hover
  values.on_hover = nil
  values.radius = S(values.radius or config.cornerR)
  local on_wheel = values.on_wheel
  values.on_wheel = nil
  values[#values + 1] = ui.MouseArea {
    anchors = { fill = true },
    cursor = "pointer",
    on_entered = function() hovered:set(true) if on_hover then on_hover(true) end end,
    on_exited = function() hovered:set(false) down:set(false) if on_hover then on_hover(false) end end,
    on_pressed = function() down:set(true) theme.drag_begin() end,
    on_released = function() down:set(false) theme.drag_end() end,
    on_clicked = function() if on_click then on_click() end end,
    -- A wheel over a button is meant for the page under it, and so is a
    -- finger dragged across it.
    on_wheel = on_wheel or function(...) theme.wheel(...) end,
    on_dragged = function(_, _, dx, dy) theme.drag(dx or 0, dy or 0) end,
  }
  return ui.Rect(values)
end

--- A row with a switch at its right: an icon, a name, a line of state,
--- the switch. What a page puts first for the thing it is about -- the
--- Wi-Fi radio, the adapter, do not disturb -- so every page starts alike.
function theme.switch_row(values)
  local W, H = values.width, S(56)
  return theme.card {
    width = W, height = H,
    border_width = 1, border_color = C.edge,
    ui.Row {
      gap = S(12), align = "center", height = H,
      anchors = { left = true, left_margin = S(14) },
      theme.icon { text = values.icon, size = config.iconSize, color = function() return values.on() and C.on or C.muted end },
      ui.Column {
        gap = S(2),
        theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1 },
        values.subtitle and theme.text { text = values.subtitle, size = config.fontSize - 4, color = C.muted,
          width = W - S(140), elide = "right" } or nil,
      },
    },
    ui.Item {
      anchors = { right = true, top = true, right_margin = S(12), top_margin = (H - S(24)) / 2 },
      theme.toggle(values.on, values.set),
    },
  }
end

--- A card row with a title and a hint at its left and `control` at its
--- right, `width` wide: the shape every setting takes.
function theme.setting_row(values)
  local W = values.width
  local H = values.hint and S(58) or S(50)
  return theme.card {
    width = W, height = H,
    border_width = 1, border_color = C.edge,
    ui.Column {
      gap = S(2),
      anchors = { left = true, top = true, left_margin = S(14), top_margin = values.hint and S(11) or S(15) },
      theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1 },
      values.hint and theme.text { text = values.hint, size = config.fontSize - 5, color = C.muted,
        width = W - (values.control_w or S(80)) - S(40), elide = "right" } or nil,
    },
    ui.Item {
      anchors = { right = true, top = true, right_margin = S(12), top_margin = (H - (values.control_h or S(24))) / 2 },
      values.control,
    },
  }
end

--- A card with a title, a figure at the right and a slider under them.
function theme.slider_row(values)
  local W = values.width
  return theme.card {
    width = W, height = S(64),
    border_width = 1, border_color = C.edge,
    theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1,
      anchors = { left = true, top = true, left_margin = S(14), top_margin = S(10) } },
    theme.text { text = values.value, size = config.fontSize - 4, color = C.muted,
      anchors = { right = true, top = true, right_margin = S(14), top_margin = S(12) } },
    ui.Item {
      anchors = { left = true, bottom = true, left_margin = S(14), bottom_margin = S(8) },
      theme.slider { width = W - S(28), height = S(20), track = S(8), knob = S(14), color = C.on,
        fraction = values.fraction, set = values.set },
    },
  }
end

--- Where a wheel goes when the thing under the pointer has no use for
--- it: the open page's scroller. The island fills this in. A finger
--- dragged across a row goes the same way: `drag_begin` at the press,
--- `drag` with each move's delta, `drag_end` at the release.
theme.wheel = function() end
theme.drag_begin = function() end
theme.drag = function() end
theme.drag_end = function() end

--- The switch: a track that fills with the accent, and a knob that slides
--- across and grows as it turns on, on a spring with a little overshoot.
function theme.toggle(on, set)
  local W, H = S(46), S(26)
  local OFF, ON = S(14), S(20)
  return ui.Rect {
    width = W, height = H, radius = H / 2,
    color = function() return on() and C.on or C.faint end,
    border_width = 1,
    border_color = function() return on() and C.on or C.muted end,
    behavior = { color = motion.fade, border_color = motion.fade },
    ui.Rect {
      radius = S(10),
      width = function() return on() and ON or OFF end,
      height = function() return on() and ON or OFF end,
      color = function() return on() and C.bg or C.fg end,
      y = function() return (H - (on() and ON or OFF)) / 2 end,
      x = function() return on() and (W - ON - S(3)) or S(6) end,
      behavior = { x = motion.move, width = motion.move, height = motion.move, y = motion.move, color = motion.fade },
      -- A check that turns in as the switch turns on, with an overshoot.
      theme.icon {
        text = "󰄬", size = config.iconSize - 6, anchors = { center_in = true },
        color = function() return on() and C.on or C.bg end,
        scale = function() return on() and 1 or 0 end,
        rotation = function() return on() and 0 or -120 end,
        behavior = { scale = motion.snappy, rotation = { duration = config.reduceMotion and 1 or 420, easing = "out_back" } },
      },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_clicked = function() set(not on()) end,
    },
  }
end

--- A slider: track, fill and a round knob. `fraction()` reads, `set(f)`
--- writes on press, drag and wheel.
--- A slider, the way a phone draws one now: a thick track that fills
--- with the accent up to a thin bar for a handle, which grows while it
--- is held. `fraction()` reads, `set(f)` writes on press, drag and wheel.
local slider_count = 0
function theme.slider(values)
  local width = values.width
  local height = values.height or S(24)
  local track_h = values.track or S(12)
  local BAR_W = S(4)
  local fraction, set = values.fraction, values.set
  slider_count = slider_count + 1
  local held = morf.signal("panacea.slider.held." .. slider_count, false)
  local hover = morf.signal("panacea.slider.hover." .. slider_count, false)
  local function at(local_x) return math.max(0, math.min(1, (local_x - BAR_W) / (width - BAR_W * 2))) end
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
      width = BAR_W, radius = BAR_W / 2, color = C.fg,
      height = function() return held:get() and (track_h + S(14)) or (hover:get() and track_h + S(10) or track_h + S(6)) end,
      y = function()
        local h = held:get() and (track_h + S(14)) or (hover:get() and track_h + S(10) or track_h + S(6))
        return (height - h) / 2
      end,
      x = function() return BAR_W + fraction() * (width - BAR_W * 2) - BAR_W / 2 end,
      behavior = { x = motion.hover, height = motion.snappy, y = motion.snappy },
    },
    ui.MouseArea {
      anchors = { fill = true }, cursor = "pointer",
      on_entered = function() hover:set(true) end,
      on_exited = function() hover:set(false) end,
      on_pressed = function(_, _, local_x) held:set(true) set(at(local_x)) end,
      on_dragged = function(_, _, _, _, local_x) if held:get() then set(at(local_x)) end end,
      on_released = function() held:set(false) end,
      on_wheel = function(_, _, _, _, _, steps_y)
        if steps_y ~= 0 then set(math.max(0, math.min(1, fraction() - steps_y * 0.05))) end
      end,
    },
  }
end

--- A row of choices, one lit: 30 / 60 / 120, ~/Videos / ~/Pictures. The
--- light is one pill that slides on a spring to whichever is chosen.
function theme.chips(options, current, choose)
  local nodes = {}
  local values = {}
  for index, option in ipairs(options) do
    local value, label = option, tostring(option)
    if type(option) == "table" then
      value, label = option.value, option.label or tostring(option.value)
    end
    values[index] = value
    nodes[index] = theme.button {
      height = S(30), radius = 10,
      color = "transparent",
      hover_color = C.card_hover,
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
  local row
  local function chosen()
    local now = current()
    for index, value in ipairs(values) do
      if value == now then return nodes[index] end
    end
    return nil
  end
  local light = ui.Rect {
    height = S(30), radius = 10, z = -1,
    color = C.on_tint, border_width = 1, border_color = C.on_edge,
    x = function()
      local node = chosen()
      return node and row and ((node.layout_x or 0) - (row.layout_x or 0)) or 0
    end,
    width = function()
      local node = chosen()
      return node and (node.layout_width or 0) or 0
    end,
    opacity = function() return chosen() and 1 or 0 end,
    behavior = { x = motion.move, width = motion.move, opacity = motion.fade },
  }
  row = ui.Row { gap = S(8), table.unpack(nodes) }
  return ui.Item {
    width = function() return row.layout_width or 0 end,
    height = S(30),
    light,
    row,
  }
end

-- A signal that follows `shown` up at once and down after `delay` ms.
local mounted_count = 0
-- One ticker for everything that polls a little: a timer is a thread of
-- its own that wakes every output's loop each time it fires, and fifty of
-- them at 24ms were a quarter of a core doing nothing.
local ticking = {}
morf.timer(24, function()
  for _, tick in ipairs(ticking) do tick() end
end, true)

--- Runs `tick` every 24ms, on the shared ticker.
function theme.tick(tick)
  ticking[#ticking + 1] = tick
end

function theme.mounted(shown, delay)
  mounted_count = mounted_count + 1
  local mounted = morf.signal("panacea.mounted." .. mounted_count, shown:get())
  local clock = morf.elapsed_timer()
  local going = false
  theme.tick(function()
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
  end)
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
