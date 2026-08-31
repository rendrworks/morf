-- Fields as the foundation, not as a special kind of drawing.
--
-- Both rows below are built by the same function. They contain ordinary
-- `ui.Rect` nodes inside an ordinary `ui.Row`, positioned by the ordinary
-- layout engine. Nothing in them was written for a distance field.
--
-- The only difference is one number on the container: `blend`. A field absorbs
-- every shape beneath it — through the positioners, however deeply they nest —
-- and composes them in one fragment shader. With no blend they compose with
-- hard edges and look exactly like the rects they are. With a blend they fuse:
-- neighbours draw out of one another, the selection melts into the tab under
-- it, and the seam between them stops existing.
--
-- Text has no field of its own, so it paints over the composition untouched.
-- That is the rule in full: anything with a shape becomes part of the surface,
-- anything else draws on top.

local morf = require("morf")
local ui = require("morf.ui")

local W, H = 900, 400
morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local INK = "#0e1213"
local SURFACE = "#b4e1ea"
local ACCENT = "#f0b47a"
local MUTED = "#6a8389"

-- Each tab has its own fill. An absorbed rect brings its colour into the
-- composition, and the fills cross-fade with the same weight the seam uses, so
-- fusing them does not flatten them to one colour — the surface is continuous
-- and the colours still read.
-- `#RRGGBBAA`, so Signals is a little over a third opaque and Render is two
-- thirds. Alpha is carried per layer like any other channel, so it cross-fades
-- across a seam exactly as the colour does: where a solid tab fuses into a
-- transparent one the surface itself becomes gradually see-through, rather
-- than one shape being drawn over the other and showing a hard edge.
local TABS = {
  { "Overview", "#b4e1ea" },
  { "Signals", "#8fd0c45c" },
  { "Layout", "#f0b47a" },
  { "Render", "#e08f8faa" },
}
local TAB_W, TAB_H, GAP = 150, 62, 26

local selected = morf.signal("foundation.selected", 1)

--- One tab bar. `blend` is the only thing that differs between the two rows.
---
--- The selection is a rect like any other. It is not drawn on top of the tabs
--- and it is not a border on one of them — it is another shape in the same
--- composition, which is why it can melt into its neighbour.
local function bar(y, blend)
  local cells = {
    x = 40,
    y = y,
    width = W - 80,
    height = TAB_H + 34,
    fill_color = SURFACE,
    blend = blend,
  }

  cells[#cells + 1] = ui.Row {
    x = 0,
    y = 17,
    spacing = GAP,
    -- Every child here is a plain rect. The row lays them out; the field
    -- composes whatever the row produced.
    ui.Rect { width = TAB_W, height = TAB_H, radius = 18, color = TABS[1][2] },
    ui.Rect { width = TAB_W, height = TAB_H, radius = 18, color = TABS[2][2] },
    ui.Rect { width = TAB_W, height = TAB_H, radius = 18, color = TABS[3][2] },
    ui.Rect { width = TAB_W, height = TAB_H, radius = 18, color = TABS[4][2] },
  }

  -- The selection: one more rect, sliding between the tabs. A behavior on `x`
  -- is the entire animation — the fusing is the field's doing, per frame, with
  -- nothing tracking it.
  cells[#cells + 1] = ui.Rect {
    y = 0,
    width = TAB_W + 16,
    height = TAB_H + 34,
    radius = 26,
    color = function() return TABS[selected:get()][2] end,
    x = function() return (selected:get() - 1) * (TAB_W + GAP) - 8 end,
    -- One table, both properties, the same curve and the same duration: the
    -- pill takes on the colour of where it is going *while* it travels, rather
    -- than changing colour and then setting off. Two separate `behavior` keys
    -- would not do this — the second would replace the first and one of the
    -- two would snap.
    behavior = {
      x = { duration = 620, easing = "in_out_cubic" },
      color = { duration = 620, easing = "in_out_cubic" },
    },
  }

  return ui.Sdf(cells)
end

--- The labels, which are not part of any field.
local function labels(y, tint)
  local out = { x = 40, y = y, width = W - 80, height = TAB_H + 34 }
  for index, tab in ipairs(TABS) do
    out[#out + 1] = ui.Text {
      x = (index - 1) * (TAB_W + GAP),
      y = 40,
      width = TAB_W,
      text = tab[1],
      font_size = 15,
      horizontal_alignment = "center",
      color = tint,
    }
  end
  return ui.Item(out)
end

local function caption(y, text)
  return ui.Text { x = 40, y = y, width = W - 80, text = text, font_size = 13, color = MUTED }
end

ui.Item {
  width = W,
  height = H,
  -- No background: a surface only shows what it paints, so the bars float over
  -- whatever is beneath them, and with nothing interactive in the tree the
  -- input region stays empty and every click goes straight through.
  caption(40, "blend = 0 — the same rects, hard edges; two are semi-transparent"),
  bar(66, 0),
  labels(66, INK),

  caption(226, "blend = 30 — fused; colour and alpha both cross-fade at the seams"),
  bar(252, 30),
  labels(252, INK),

  caption(H - 40, "one number differs between the two rows"),

  ui.Timer {
    interval = 1500,
    ["repeat"] = true,
    running = true,
    on_triggered = function()
      local ok, error = selected:set(selected:get() % #TABS + 1)
      assert(ok, error)
    end,
  },
}
