-- Frosted glass: the compositor blurs what is behind this surface.
--
--     oslo make run --example examples/frosted-panel.lua
--
-- This is the one effect a client cannot do for itself. Everything else in
-- `examples/` reads pixels morf drew — an effect shader samples the layer its
-- own node became, and it can do that because we rendered it. What is *behind*
-- the surface belongs to other windows, and Wayland will not show it to us.
--
-- What it will do, through `ext-background-effect-v1`, is blur it for us. The
-- exchange is one-way:
--
--   1. the compositor draws the desktop and the windows
--   2. it blurs that result, but only inside a region we named
--   3. it blends this surface over the top
--
-- We never receive step 2. Which is why the alpha here matters more than
-- anything else in the file: a panel painted opaque sits on a blurred backdrop
-- nobody can see. The blur is revealed by what we *don't* paint. Everything
-- that makes it read as glass rather than as a blur filter — the tint, the
-- grain, the lit top edge — is painted on top of a backdrop this process never
-- touches.
--
-- Wanted by KDE 6.7+, niri, GNOME 51 and COSMIC. Where the protocol is absent
-- the panel is simply translucent over a sharp desktop, which is why it is
-- built to look deliberate either way.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local screen = morf.screens[1]
local SW = (screen and screen.width) or 1280
local SH = (screen and screen.height) or 720

local W = 720
local H = 260

morf.surface.width = W
morf.surface.height = H
morf.surface.layer = "overlay"
morf.surface.anchors = { top = true }
morf.surface.margin_top = 60
morf.surface.keyboard_focus = "none"
morf.surface.exclusive_zone = -1

local RADIUS = 28
local INK = "#f2f4f8"
local MUTED = "#f2f4f899"

--------------------------------------------------------------------------------
-- The glass.
--------------------------------------------------------------------------------

-- Grain, a lit top edge and a soft inner shadow. None of this can touch the
-- blurred backdrop — it is composited underneath us and we have no read of it —
-- so this is a *surface* shader: it owns its own coverage, decides its own
-- alpha, and everything it leaves transparent is where the blur shows through.
morf.shader("glass", {
  kind = "surface",
  params = {
    -- How milky the glass is. This is the number that decides how much of the
    -- blur survives: at 1.0 there is no glass left to see through.
    frost = 0.16,
    -- Corner radius in pixels, so the shader's edge matches the blur region's.
    radius = 28.0,
    -- Film grain. Large flat areas of a blur band without it.
    grain = 0.035,
  },
  fragment = [[
    -- A real 32-bit hash, so the grain is grain and not a visible pattern.
    function hash(seed)
      local h = seed * u32(747796405) + u32(2891336453)
      local word = ((h >> ((h >> u32(28)) + u32(4))) ~ h) * u32(277803737)
      return f32((word >> u32(22)) ~ word & u32(65535)) / 65535.0
    end

    -- Distance to a rounded rectangle, negative inside. The shader draws its
    -- own shape because in surface mode the node's geometry is not consulted —
    -- and because this edge has to line up with the region the compositor was
    -- given, which is the same rounded rectangle.
    function rounded(p, half, radius)
      local q = abs(p) - half + vec2(radius, radius)
      return length(max(q, vec2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius
    end

    function fragment(uv, time, resolution, frost, grain, radius)
      -- Pixels, from the derivative of the coordinates: a surface shader is not
      -- told the size of the node it is filling, and the rate `uv` changes per
      -- pixel is exactly that, measured where it is being drawn.
      local wide = 1.0 / max(fwidth(uv.x), 0.000001)
      local tall = 1.0 / max(fwidth(uv.y), 0.000001)
      local half = vec2(wide, tall) * 0.5
      local p = (uv - vec2(0.5, 0.5)) * vec2(wide, tall)

      local d = rounded(p, half, radius)
      -- One pixel of softness, whatever the panel's size.
      local edge = 1.0 - smoothstep(0.0 - fwidth(d), fwidth(d), d)

      -- The glass itself: a pale film, thicker towards the bottom the way a
      -- pane of real glass is thicker where light does not reach through it.
      local milk = frost * (0.85 + uv.y * 0.35)

      -- A lit top edge. Glass catches light along the rim that faces it, and
      -- this one line is most of what stops a rounded rectangle reading as a
      -- flat translucent card.
      local rim = 1.0 - smoothstep(0.0, 2.5, abs(d + 1.2))
      local lip = rim * (1.0 - smoothstep(0.0, 0.55, uv.y)) * 0.5

      -- Grain, so a large blurred area does not band.
      local cell = floor(uv * vec2(wide, tall))
      local speck = hash(u32(cell.x) * u32(374761393) + u32(cell.y) * u32(668265263))
      local dust = (speck - 0.5) * grain

      local lit = vec3(1.0, 1.0, 1.0) * (milk + lip + dust)
      -- Alpha is the whole point. Where this is low the compositor's blurred
      -- backdrop comes through untouched; where it is high we have covered it
      -- up. `frost` is that dial.
      local alpha = clamp(milk + lip * 1.4, 0.0, 1.0) * edge
      return vec4(lit, alpha)
    end
  ]],
})

--------------------------------------------------------------------------------
-- The panel.
--------------------------------------------------------------------------------

local supported = "asking the compositor to blur behind this panel"

ui.Item {
  width = W,
  height = H,

  -- The node that asks for the blur. Its rectangle and radii are what get
  -- rasterised into the region handed to the compositor — a span per scanline,
  -- so the rounded corners are exact rather than approximated. What a region
  -- cannot carry is a soft edge: membership is one bit per pixel. The glass
  -- above is drawn antialiased over the top, which is what hides the step.
  ui.Rect {
    width = W,
    height = H,
    radius = RADIUS,
    -- Nearly nothing of its own. Every pixel of colour here is a pixel of the
    -- blur painted over.
    color = "#0e121a2b",
    backdrop_blur = true,
  },

  -- The glass, exactly over it.
  ui.Rect {
    width = W,
    height = H,
    shader = "glass",
    shader_params = { frost = 0.16, radius = RADIUS, grain = 0.035 },
  },

  ui.Text {
    x = 40,
    y = 52,
    width = W - 80,
    text = "Frosted",
    font_size = 46,
    color = INK,
  },
  ui.Text {
    x = 40,
    y = 118,
    width = W - 80,
    text = supported,
    font_size = 14,
    color = MUTED,
  },
  ui.Text {
    x = 40,
    y = 150,
    width = W - 80,
    text = "ext-background-effect-v1 · the blur happens on the far side of this surface",
    font_size = 12,
    color = MUTED,
  },

  -- Something behind it worth blurring, if this is run over a bare desktop:
  -- drag a window under the panel and the blur follows it.
  ui.Text {
    x = 40,
    y = 194,
    width = W - 80,
    text = "move a window underneath — the blur is the compositor's, not ours",
    font_size = 12,
    color = MUTED,
  },
}
