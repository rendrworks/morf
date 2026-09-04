-- Shaders written in Lua, compiled to WGSL, run on the GPU.
--
-- The `fragment` string is not Lua that runs. It is parsed, type checked and
-- printed as WGSL once, while this file loads; what executes per pixel is
-- compiled shader code on the GPU. That is why it is a string rather than a
-- function — `print` inside it would have nowhere to print to, and pretending
-- otherwise would only invite the confusion.
--
-- The shape still comes from the node. A shader decides the colour inside it,
-- so clipping, hit testing and the input region all keep working exactly as
-- they do on a node without one.

local morf = require("morf")
local ui = require("morf.ui")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1280
local H = (screen and screen.height) or 720

morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }

local theme = morf.theme {
  ink = "#0f1116",
  muted = "#8b90a0",
}

-- A shader that reads `time` repaints every frame. One that does not costs
-- nothing after the first — which is what makes a shader affordable on a bar
-- that never changes. Nothing here declares which it is: the compiler noticed
-- while it was lowering the body, where it cannot be forgotten.
morf.shader("plasma", {
  params = { speed = 1.0, warp = 6.0 },
  fragment = [[
    function fragment(uv, time, resolution, coverage, speed, warp)
      local centred = uv - vec2(0.5, 0.5)
      local d = length(centred)
      local wave = sin(d * warp - time * speed) * 0.5 + 0.5
      local ring = smoothstep(0.0, 0.4, wave)
      return vec4(ring * 0.25, ring * 0.6, 0.9, 1.0)
    end
  ]],
})

morf.shader("sheen", {
  params = { angle = 0.6 },
  fragment = [[
    function fragment(uv, time, resolution, coverage, angle)
      local band = fract(uv.x * 3.0 + uv.y * angle)
      local edge = smoothstep(0.45, 0.5, band) - smoothstep(0.5, 0.55, band)
      return vec4(0.32 + edge * 0.45, 0.34 + edge * 0.45, 0.44 + edge * 0.45, 1.0)
    end
  ]],
})

-- Real control flow, which is what a compiler buys over the cheaper designs:
-- this loop's exit depends on the data it is computing. It cannot run away —
-- every loop the compiler emits carries an iteration counter the shader cannot
-- reach around, so a shader can be wrong but never take the session with it.
morf.shader("orbit", {
  params = { steps = 10.0 },
  fragment = [[
    function fragment(uv, time, resolution, coverage, steps)
      local p = uv - vec2(0.5, 0.5)
      local total = 0.0
      local i = 0.0
      while i < steps do
        local a = i * 0.7 + time * 0.4
        local o = vec2(cos(a), sin(a)) * 0.28
        local d = length(p - o)
        if d < 0.09 then
          total = total + (0.09 - d) * 8.0
        end
        i = i + 1.0
      end
      total = clamp(total, 0.0, 1.0)
      return vec4(total * 0.95, total * 0.35, 0.62 - total * 0.2, 1.0)
    end
  ]],
})

-- A surface shader decides its own coverage: the node's rounded rectangle is
-- not consulted at all, so the shape here is whatever the shader returns alpha
-- for. Geometry and shader stop composing in this mode, which is inherent to
-- it rather than a gap.
morf.shader("blades", {
  kind = "surface",
  params = { count = 6.0 },
  fragment = [[
    function fragment(uv, time, resolution, count)
      local p = uv - vec2(0.5, 0.5)
      local angle = atan2(p.y, p.x) + time * 0.5
      local radius = length(p)
      -- A rosette: the petal count comes from a parameter, so animating it
      -- animates the shape itself rather than only its colour.
      local petal = cos(angle * count) * 0.14 + 0.28
      local inside = 1.0 - smoothstep(petal - 0.01, petal + 0.01, radius)
      return vec4(0.95, 0.55 + radius, 0.2, inside)
    end
  ]],
})

-- An effect shader reads what is already rendered underneath it. That needs
-- somewhere to read *from*, so a node carrying one becomes a compositing layer
-- whether or not anything else about it would have made it into one — its
-- subtree renders to a target first, and the shader reworks that.
morf.shader("ripple", {
  kind = "effect",
  params = { amount = 0.012, rate = 2.0 },
  fragment = [[
    function fragment(uv, time, resolution, amount, rate)
      -- Sampling somewhere other than this pixel is the whole point: a
      -- distortion has to be able to reach sideways.
      local wave = sin(uv.y * 24.0 + time * rate) * amount
      local shifted = vec2(uv.x + wave, uv.y)
      return texture(shifted)
    end
  ]],
})

local PANEL_W = 240
local PANEL_H = 150
local GAP = 32

local panels = {
  { "plasma", { speed = 1.4, warp = 9.0 }, "plasma — reads the clock" },
  { "sheen", { angle = 1.2 }, "sheen — static, never repaints" },
  { "orbit", { steps = 12.0 }, "orbit — a loop with a real exit" },
  { "blades", { count = 7.0 }, "blades — surface: its own shape" },
}

local total = #panels * PANEL_W + (#panels - 1) * GAP
local left = (W - total) / 2
local top = (H - PANEL_H) / 2 - 20

local children = { width = W, height = H }
children[#children + 1] = ui.Rect { width = W, height = H, color = theme.ink }

-- The ripple has to *wrap* the row, not lie over it. An effect shader samples
-- the layer its own node became, and a layer holds that node's subtree — so a
-- transparent rectangle laid over its siblings samples nothing but itself, and
-- returns exactly that. Applying one to a leaf would likewise give it nothing
-- but that leaf.
--
-- It spans the whole surface so the panels below can keep the coordinates they
-- were laid out with.
local rippled = {
  width = W,
  height = H,
  shader = "ripple",
  shader_params = { amount = 0.008, rate = 2.5 },
}

for index, panel in ipairs(panels) do
  local x = left + (index - 1) * (PANEL_W + GAP)
  rippled[#rippled + 1] = ui.Rect {
    x = x,
    y = top,
    width = PANEL_W,
    height = PANEL_H,
    radius = 18,
    color = "#1b1d23",
    shader = panel[1],
    shader_params = panel[2],
  }
  -- The captions stay outside the ripple: they are there to be read.
  children[#children + 1] = ui.Text {
    x = x,
    y = top + PANEL_H + 12,
    width = PANEL_W,
    text = panel[3],
    font_size = 12,
    color = theme.muted,
  }
end

table.insert(children, 2, ui.Item(rippled))

ui.Item(children)
