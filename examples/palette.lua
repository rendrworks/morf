-- A colour is a value. One accent is picked, and everything on the board is
-- derived from it in Lua with the operations a colour value carries: lighter
-- and darker steps, the complement, a hue rotation in OkLCh, a ramp sampled
-- between two ends, a palette of colours as far apart as they can be, and
-- the black or white that reads against each swatch.
--
-- `morf ipc call shift` picks a new vivid accent; every swatch follows it,
-- and travels in OkLCh the long way round the wheel rather than fading
-- through grey. `morf ipc call describe` prints the accent in every notation
-- the value can write.

local morf = require("morf")
local ui = require("morf.ui")

morf.surface.width = 720
morf.surface.height = 360
morf.surface.anchors = { top = true, left = true }
morf.surface.keyboard_focus = "none"

local accent = morf.signal("palette.accent", morf.color "#3b82f6")

local function derived(step)
  return function() return step(accent:get()) end
end

local travel = { duration = 700, easing = "in_out_cubic", space = "oklch", hue = "longer" }

-- A swatch shows one derived colour and names it in whichever of black or
-- white reads against it.
local function swatch(label, step)
  return ui.Rect {
    width = 100,
    height = 64,
    radius = 10,
    color = derived(step),
    behavior = { color = travel },
    ui.Column {
      anchors = { fill = true, margins = 8 },
      gap = 2,
      ui.Text {
        text = label,
        font_size = 12,
        color = derived(function(c) return step(c):text_color() end),
      },
      ui.Text {
        text = derived(function(c) return tostring(step(c)) end),
        font_size = 11,
        color = derived(function(c) return step(c):text_color():alpha(0.7) end),
      },
    },
  }
end

local board = ui.Rect {
  width = 720,
  height = 360,
  color = derived(function(c) return c:with { l = 0.18, c = 0.03, space = "oklch" } end),
  behavior = { color = travel },
  ui.Column {
    anchors = { fill = true, margins = 20 },
    gap = 12,
    ui.Row {
      gap = 12,
      swatch("accent", function(c) return c end),
      swatch("lighter", function(c) return c:lighten(0.15) end),
      swatch("darker", function(c) return c:darken(0.15) end),
      swatch("complement", function(c) return c:complement() end),
      swatch("rotated", function(c) return c:with { h = c:oklch().h + 60, space = "oklch" } end),
      swatch("half alpha", function(c) return c:alpha(0.5) end),
    },
    -- A ramp from the accent to its complement, mixed in OkLab so the middle
    -- keeps its brightness.
    (function()
      local ramp = { gap = 4 }
      for index = 1, 8 do
        ramp[#ramp + 1] = ui.Rect {
          width = 81,
          height = 40,
          radius = 6,
          color = derived(function(c)
            return morf.color.scale { c, c:complement() }:sample((index - 1) / 7)
          end),
          behavior = { color = travel },
        }
      end
      return ui.Row(ramp)
    end)(),
    -- Six colours as far apart as CIEDE2000 can put them, the accent fixed
    -- as the first. The search is random, so it runs once per accent and
    -- every swatch reads the same answer.
    (function()
      local last, palette
      local function apart_from(c)
        if last ~= c then
          last, palette = c, morf.color.distinct(6, { fixed = { c }, order = true, iterations = 30000 })
        end
        return palette
      end
      local apart = { gap = 4 }
      for index = 1, 6 do
        apart[#apart + 1] = ui.Rect {
          width = 109,
          height = 40,
          radius = 6,
          color = derived(function(c)
            return apart_from(c)[index]
          end),
          behavior = { color = travel },
        }
      end
      return ui.Row(apart)
    end)(),
    ui.Text {
      text = derived(function(c)
        return string.format(
          "%s  contrast on white %.2f  nearest %s",
          c:oklch_string(),
          c:contrast("white"),
          c:nearest_name()
        )
      end),
      font_size = 13,
      color = derived(function(c) return c:lighten(0.3) end),
    },
  },
}

morf.ipc.shift = function()
  accent:set(morf.color.random("vivid"))
  return tostring(accent:get())
end

morf.ipc.describe = function()
  local c = accent:get()
  return table.concat({
    c:hex(),
    c:rgb_string(),
    c:hsl_string(),
    c:lab_string(),
    c:oklab_string(),
    c:oklch_string(),
    c:cmyk_string(),
    "ansi " .. c:ansi8(),
  }, "\n")
end

