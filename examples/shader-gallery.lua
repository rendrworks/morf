-- Everything the shader language gained, in one screen.
--
-- Each panel is one capability that did not exist before, labelled with what it
-- needed. Run it with:
--
--     oslo make run --example examples/shader-gallery.lua
--
-- Nothing here is Lua that runs per pixel. Each `fragment` string is parsed,
-- type checked and printed as WGSL once, while this file loads; what executes
-- is compiled shader code on the GPU.

local morf = require("morf")
local ui = require("morf.ui")

-- A floating panel rather than a full screen, so it sits over the corner of
-- the desktop and leaves the terminal you launched it from visible. Set
-- MORF_GALLERY_FULL=1 to have it take the whole output instead.
local core = require("morf.core")
local FULL = core.env("MORF_GALLERY_FULL") == "1"

local screen = morf.screens[1]
local SW = (screen and screen.width) or 1280
local SH = (screen and screen.height) or 720

local W = FULL and SW or 900
local H = FULL and SH or 520

morf.surface.width = W
morf.surface.height = H
morf.surface.layer = "overlay"
morf.surface.anchors = FULL and { top = true, left = true, right = true, bottom = true }
  or { top = true, right = true }
morf.surface.margin_top = FULL and 0 or 40
morf.surface.margin_right = FULL and 0 or 40

local INK = "#0d0f14"
local PANEL = "#161922"
local LABEL = "#7d8496"
local TITLE = "#cdd3e0"

--------------------------------------------------------------------------------
-- Matrices. Rotation could only be written by expanding the arithmetic by hand.
--------------------------------------------------------------------------------
morf.shader("rotor", {
  params = { blades = 6.0 },
  fragment = [[
    function fragment(uv, time, resolution, coverage, blades)
      local p = uv - vec2(0.5, 0.5)
      local a = time * 0.6
      local turn = mat2(vec2(cos(a), sin(a)), vec2(0.0 - sin(a), cos(a)))
      local q = turn * p
      local angle = atan2(q.y, q.x)
      local radius = length(q)
      local petal = cos(angle * blades) * 0.16 + 0.30
      local edge = smoothstep(petal + 0.008, petal - 0.008, radius)
      return vec4(edge * 0.95, edge * 0.45 + radius, 0.25, 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- Integers and bitwise. A real hash, and therefore real noise: the constants
-- below need thirty-two bits and an f32 has twenty-four of mantissa.
--------------------------------------------------------------------------------
morf.shader("static", {
  fragment = [[
    function hash(seed)
      local h = seed * u32(747796405) + u32(2891336453)
      local word = ((h >> ((h >> u32(28)) + u32(4))) ~ h) * u32(277803737)
      return (word >> u32(22)) ~ word
    end

    function fragment(uv, time, resolution, coverage)
      local cell = floor(uv * 26.0)
      local step = floor(time * 8.0)
      local seed = u32(cell.x) * u32(374761393)
        + u32(cell.y) * u32(668265263)
        + u32(step) * u32(2246822519)
      local v = f32(hash(seed) & u32(65535)) / 65535.0
      return vec4(v * 0.55, v * 0.75, v, 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- Arrays and a switch. A palette is a Lua list; the band choice is an if chain
-- the emitter recognises as a jump table.
--------------------------------------------------------------------------------
morf.shader("bands", {
  fragment = [[
    function fragment(uv, time, resolution, coverage)
      local ramp = {
        vec3(0.96, 0.35, 0.30),
        vec3(0.98, 0.72, 0.25),
        vec3(0.35, 0.82, 0.55),
        vec3(0.30, 0.60, 0.98),
        vec3(0.66, 0.45, 0.95)
      }
      local drift = fract(uv.x + time * 0.12)
      local slot = clamp(i32(drift * 5.0), 0, 4)
      local base = ramp[slot]
      -- Recognised as a switch: one whole number, distinct constants, an else.
      local lift = 0.0
      if slot == 0 then
        lift = 0.00
      elseif slot == 1 then
        lift = 0.06
      elseif slot == 2 then
        lift = 0.12
      elseif slot == 3 then
        lift = 0.18
      else
        lift = 0.24
      end
      return vec4(base + vec3(lift, lift, lift) * (1.0 - uv.y), 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- Derivatives. The engine's own shader antialiases with fwidth; before this a
-- configuration's could not, so a shader drawing its own shape had no way to
-- soften it at the resolution it was actually drawn at.
--------------------------------------------------------------------------------
morf.shader("crisp", {
  kind = "surface",
  fragment = [[
    function fragment(uv, time, resolution)
      local p = (uv - vec2(0.5, 0.5)) * 2.0
      local a = atan2(p.y, p.x) + time * 0.35
      local r = length(p)
      local star = cos(a * 5.0) * 0.18 + 0.52
      local d = r - star
      -- One pixel of softness whatever the node's size, taken from the
      -- derivative rather than guessed at.
      local edge = fwidth(d)
      local inside = 1.0 - smoothstep(0.0 - edge, edge, d)
      return vec4(0.35 + r * 0.4, 0.85 - r * 0.3, 0.95, inside)
    end
  ]],
})

--------------------------------------------------------------------------------
-- A data block and a vertex displacement. The bars are numbers the config
-- pushes each tick; the whole panel sways because its corners are moved.
--------------------------------------------------------------------------------
morf.shader("levels", {
  data = { bars = 24 },
  vertex = [[
    function vertex(corner, size, time)
      -- Moves the quad, not the shape inside it.
      return corner + vec2(sin(time * 1.1 + corner.y * 0.03) * 5.0, 0.0)
    end
  ]],
  fragment = [[
    function fragment(uv, time, resolution, coverage)
      local slot = clamp(i32(uv.x * 24.0), 0, 23)
      local level = bars[slot]
      local lit = step(1.0 - level, uv.y)
      local gap = smoothstep(0.06, 0.12, fract(uv.x * 24.0))
      local shade = lit * gap
      return vec4(shade * 0.35, shade * 0.85, shade * (0.5 + level * 0.5), 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- Records and helpers. Two lights, each a named record, shaded by one function.
--------------------------------------------------------------------------------
morf.shader("lights", {
  fragment = [[
    function shade(light, at)
      local fall = 1.0 - clamp(length(at) * light.reach, 0.0, 1.0)
      return light.colour * light.power * fall * fall
    end

    function fragment(uv, time, resolution, coverage)
      local warm = { colour = vec3(1.0, 0.62, 0.28), power = 0.95, reach = 1.7 }
      local cool = { reach = 1.7, power = 0.85, colour = vec3(0.30, 0.55, 1.0) }
      local sweep = sin(time * 0.7) * 0.22
      local lit = shade(warm, uv - vec2(0.32 + sweep, 0.5))
        + shade(cool, uv - vec2(0.68 - sweep, 0.5))
      return vec4(saturate(lit), 1.0)
    end
  ]],
})

--------------------------------------------------------------------------------
-- Layout
--------------------------------------------------------------------------------
local PANELS = {
  { "rotor", "matrices — a rotation, written as one" },
  { "static", "integers — a real 32-bit hash" },
  { "bands", "arrays + switch — a palette, a jump table" },
  { "crisp", "derivatives — its own antialiased edge" },
  { "levels", "data block + vertex — pushed numbers, swaying quad" },
  { "lights", "records + helpers — two lights, one function" },
}

local COLS = 3
local PANEL_W = 260
local PANEL_H = 160
local GAP_X = 28
local GAP_Y = 46

local rows = math.ceil(#PANELS / COLS)
local total_w = COLS * PANEL_W + (COLS - 1) * GAP_X
local total_h = rows * PANEL_H + (rows - 1) * GAP_Y
local left = (W - total_w) / 2
local top = (H - total_h) / 2 + 10

local children = { width = W, height = H }
children[#children + 1] = ui.Rect { width = W, height = H, color = INK }

children[#children + 1] = ui.Text {
  x = left,
  y = top - 54,
  width = total_w,
  text = "shaders written in Lua, compiled to WGSL",
  font_size = 17,
  color = TITLE,
}

local level_nodes = {}

for index, panel in ipairs(PANELS) do
  local column = (index - 1) % COLS
  local row = math.floor((index - 1) / COLS)
  local x = left + column * (PANEL_W + GAP_X)
  local y = top + row * (PANEL_H + GAP_Y)

  children[#children + 1] = ui.Rect {
    x = x - 2, y = y - 2, width = PANEL_W + 4, height = PANEL_H + 4,
    radius = 16, color = PANEL,
  }
  local node = ui.Rect {
    x = x, y = y, width = PANEL_W, height = PANEL_H,
    radius = 14,
    color = PANEL,
    shader = panel[1],
  }
  if panel[1] == "levels" then
    level_nodes[#level_nodes + 1] = node
  end
  children[#children + 1] = node
  children[#children + 1] = ui.Text {
    x = x, y = y + PANEL_H + 10, width = PANEL_W,
    text = panel[2], font_size = 11, color = LABEL,
  }
end

-- The data block is the one thing the configuration itself drives: everything
-- else is the clock, which the shader reads on its own.
local phase = 0
children[#children + 1] = ui.Timer {
  interval = 60,
  ["repeat"] = true,
  running = true,
  on_triggered = function()
    phase = phase + 1
    local values = {}
    for slot = 1, 24 do
      local a = phase * 0.09 + slot * 0.5
      values[slot] = 0.18 + (math.sin(a) * 0.5 + 0.5) * 0.72
    end
    for _, node in ipairs(level_nodes) do
      morf.shader_data(node, "bars", values)
    end
  end,
}

ui.Item(children)
