-- An old screen: a phosphor terminal behind curved glass.
--
--     oslo make run --example examples/crt-terminal.lua
--
-- The text underneath is ordinary text — a column of `ui.Text` nodes, laid out
-- and rasterised exactly as anywhere else. Everything that makes it look like a
-- tube from 1983 happens afterwards, in one effect shader that samples the
-- rendered result and reworks it: the glass bows the picture, the beam draws
-- one line at a time, the shadow mask splits every pixel into three phosphors,
-- and the corners fall off into the dark.
--
-- That separation is the point of effect mode. Nothing about the text knows it
-- is on a CRT, so the same treatment would work over a clock, a menu, or a
-- video, and the text stays selectable geometry rather than becoming pixels the
-- configuration has to draw itself.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local FULL = core.env("MORF_CRT_FULL") == "1"

local screen = morf.screens[1]
local SW = (screen and screen.width) or 1280
local SH = (screen and screen.height) or 720

local W = FULL and SW or 860
local H = FULL and SH or 540

morf.surface.width = W
morf.surface.height = H
morf.surface.layer = "overlay"
morf.surface.anchors = FULL and { top = true, left = true, right = true, bottom = true }
  or { top = true, right = true }
morf.surface.margin_top = FULL and 0 or 40
morf.surface.margin_right = FULL and 0 or 40
morf.surface.keyboard_focus = "none"

--------------------------------------------------------------------------------
-- The tube.
--------------------------------------------------------------------------------

morf.shader("crt", {
  kind = "effect",
  params = {
    -- How much the glass bows. 0 is a flat panel.
    curve = 0.10,
    -- Depth of the scanline gaps, and how many lines the tube draws.
    scan = 0.38,
    lines = 320.0,
    -- Strength of the red/green/blue phosphor stripes.
    mask = 0.30,
    -- How far light bleeds sideways out of a lit phosphor.
    glow = 0.55,
    -- Speed of the bright band rolling down the screen.
    roll = 0.35,
  },
  fragment = [[
    -- A tube's face is a piece of a sphere. Pushing each point outward by the
    -- square of the other axis is the cheap approximation of that, and it is
    -- the one that matters: straight lines bow, and they bow more at the
    -- corners than at the edges, which is what the eye actually reads as glass.
    function warp(uv, curve)
      local p = uv * 2.0 - vec2(1.0, 1.0)
      local pull = vec2(p.y * p.y, p.x * p.x) * curve
      return (p + p * pull) * 0.5 + vec2(0.5, 0.5)
    end

    function fragment(uv, time, resolution, curve, glow, lines, mask, roll, scan)
      local bent = warp(uv, curve)

      -- Past the edge of the glass there is no picture at all. Clamping
      -- instead would smear the border pixels around the bezel, which is the
      -- usual giveaway that a CRT filter was done quickly.
      local on_glass = step(0.0, bent.x) * step(bent.x, 1.0)
        * step(0.0, bent.y) * step(bent.y, 1.0)

      local lit = texture(bent).xyz

      -- Phosphor does not stop where the beam stops; it spills into its
      -- neighbours, which is why old text has a halo rather than an edge. Two
      -- taps either side, kept at whichever is brighter, is enough to read as
      -- that spill without turning into a blur.
      local step_x = 2.0 / resolution.x
      local near = max(
        texture(bent + vec2(step_x, 0.0)).xyz,
        texture(bent - vec2(step_x, 0.0)).xyz
      )
      local far = max(
        texture(bent + vec2(step_x * 2.5, 0.0)).xyz,
        texture(bent - vec2(step_x * 2.5, 0.0)).xyz
      )
      lit = lit + (near * 0.6 + far * 0.3) * glow

      -- The beam interlaces: every other line is drawn dark. Tying the count
      -- to a parameter rather than to the pixel height keeps the lines the
      -- same thickness whatever output this lands on.
      local beam = sin(bent.y * lines * 3.14159265) * 0.5 + 0.5
      lit = lit * (1.0 - scan * beam)

      -- The shadow mask. Each physical pixel is three stripes of phosphor, and
      -- only one of them can be that colour, so the stripe a pixel lands on
      -- decides which channel it is allowed to be bright in.
      local stripe = i32(floor(uv.x * resolution.x)) % 3
      local tint = vec3(1.0, 1.0, 1.0)
      if stripe == 0 then
        tint = vec3(1.0, 1.0 - mask, 1.0 - mask)
      elseif stripe == 1 then
        tint = vec3(1.0 - mask, 1.0, 1.0 - mask)
      else
        tint = vec3(1.0 - mask, 1.0 - mask, 1.0)
      end
      lit = lit * tint

      -- One bright band drifting down the face: the refresh beating against
      -- itself. Slow, and shallow enough to notice only once it has passed.
      local band = fract(bent.y - time * roll)
      lit = lit + lit * smoothstep(0.94, 1.0, band) * 0.35

      -- The corners of a tube are further from the gun and darker for it.
      local off = bent - vec2(0.5, 0.5)
      lit = lit * clamp(1.0 - dot(off, off) * 1.15, 0.0, 1.0)

      -- The mask and the scanlines take away more light than they should, so
      -- the whole picture is lifted back up rather than left dim.
      lit = lit * 1.45

      -- Alpha comes from the source: the tube is only ever as opaque as what
      -- it was given, so the desktop still shows through the dark parts.
      local a = texture(bent).w
      return vec4(lit * on_glass, a * on_glass)
    end
  ]],
})

--------------------------------------------------------------------------------
-- What is on the screen.
--------------------------------------------------------------------------------

local GLASS = "#050a06"
local BEZEL = "#141712"
local PHOSPHOR = "#8dffa8"
local DIM = "#3f8f56"

local BOOT = {
  { DIM, "MORF SYSTEM MONITOR                       REV 0.1.3" },
  { DIM, "-----------------------------------------------------" },
  { PHOSPHOR, "> selftest" },
  { DIM, "  scene graph .......................... OK" },
  { DIM, "  layout solver ........................ OK" },
  { DIM, "  distance fields ...................... OK" },
  { DIM, "  shader compiler ...................... OK" },
  { DIM, "  wayland surfaces ..................... OK" },
  { PHOSPHOR, "> shader --list" },
  { DIM, "  crt          effect    6 params  animated" },
  { DIM, "  chromatic    effect    3 params  animated" },
  { PHOSPHOR, "> shader --explain crt" },
  { DIM, "  written in lua, compiled to wgsl at load," },
  { DIM, "  executed on the gpu once per pixel per frame." },
  { PHOSPHOR, "> _" },
}

local PAD_X = 54
local PAD_Y = 46
local LINE = 22
local FONT = 14

-- Everything inside the tube. An effect shader samples the layer its own node
-- became, and a layer holds that node's subtree — so what the tube is meant to
-- rework has to be *inside* it. A transparent rectangle laid over its siblings
-- is not a wrapper: its layer holds only itself, and the effect faithfully
-- returns nothing.
local inside = { width = W, height = H, shader = "crt" }

-- The bezel, then the glass. Both are ordinary rectangles; the shader does not
-- care what it is given.
inside[#inside + 1] = ui.Rect { width = W, height = H, radius = 26, color = BEZEL }
inside[#inside + 1] = ui.Rect {
  x = 16, y = 16, width = W - 32, height = H - 32,
  radius = 18, color = GLASS,
}

local lines = {}
for index, entry in ipairs(BOOT) do
  local node = ui.Text {
    x = PAD_X,
    y = PAD_Y + (index - 1) * LINE,
    width = W - PAD_X * 2,
    text = "",
    font_family = "monospace",
    font_size = FONT,
    color = entry[1],
  }
  lines[index] = node
  inside[#inside + 1] = node
end

local children = { width = W, height = H, ui.Item(inside) }

-- The log types itself in, one character at a time, because a terminal that
-- arrives complete does not look like a terminal.
local row = 1
local column = 0
children[#children + 1] = ui.Timer {
  interval = 18,
  ["repeat"] = true,
  running = true,
  on_triggered = function()
    if row > #BOOT then
      return
    end
    local full = BOOT[row][2]
    column = column + 2
    if column >= #full then
      lines[row].text = full
      row = row + 1
      column = 0
    else
      lines[row].text = full:sub(1, column)
    end
  end,
}

ui.Item(children)
