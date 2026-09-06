-- A line of text being typed, and the keys that edit it.
--
-- The engine hands a key to `on_key_pressed(keysym, text)`; what a keysym
-- means to a text field is decided here, once, for the launcher, the
-- clipboard search and the Wi-Fi password. A field is a state table --
-- `text`, `caret`, `focused` -- and a node drawing it, with a blinking
-- caret while it has focus.

local morf = require("morf")
local ui = require("morf.ui")
local theme = require("theme")

local S = theme.S
local C = theme.color

local field = {}

field.keys = {
  RETURN = 0xff0d, KP_ENTER = 0xff8d, BACKSPACE = 0xff08, DELETE = 0xffff,
  ESCAPE = 0xff1b, TAB = 0xff09,
  LEFT = 0xff51, UP = 0xff52, RIGHT = 0xff53, DOWN = 0xff54,
  HOME = 0xff50, END = 0xff57, PAGE_UP = 0xff55, PAGE_DOWN = 0xff56,
}
local K = field.keys

local counter = 0

--- Makes a field. `options.on_change(text)`, `on_submit(text)`,
--- `on_escape()`, `on_key(keysym, text)` for anything else (return true
--- when handled), `secret` to draw dots, `placeholder`.
function field.new(options)
  options = options or {}
  counter = counter + 1
  local self = {
    text = morf.signal("panacea.field." .. counter .. ".text", ""),
    focused = morf.signal("panacea.field." .. counter .. ".focused", true),
    options = options,
  }

  function self.clear()
    if self.text:get() ~= "" then
      self.text:set("")
      if options.on_change then options.on_change("") end
    end
  end

  function self.set(text)
    self.text:set(text)
    if options.on_change then options.on_change(text) end
  end

  --- Feeds one key. Returns true when the field used it.
  function self.handle(keysym, text)
    if keysym == K.RETURN or keysym == K.KP_ENTER then
      if options.on_submit then options.on_submit(self.text:get()) end
      return true
    elseif keysym == K.ESCAPE then
      if options.on_escape then options.on_escape() end
      return true
    elseif keysym == K.BACKSPACE then
      local current = self.text:get()
      if current ~= "" then
        -- Whole UTF-8 sequence, not one byte of it.
        local cut = #current
        while cut > 1 and current:byte(cut) >= 0x80 and current:byte(cut) < 0xc0 do cut = cut - 1 end
        self.set(current:sub(1, cut - 1))
      end
      return true
    elseif options.on_key and options.on_key(keysym, text) then
      return true
    elseif type(text) == "string" and text ~= "" and keysym < 0xff00 and text:byte(1) >= 0x20 then
      self.set(self.text:get() .. text)
      return true
    end
    return false
  end

  return self
end

--- The node: a dark rounded box with the text, or the placeholder, and a
--- caret. `values` are the Rect's: `width`, `height`, `radius`.
function field.node(self, values)
  values = values or {}
  local height = values.height or S(52)
  local options = self.options
  local blink = morf.signal("panacea.field.blink." .. tostring(self), true)
  local shown = function()
    local text = self.text:get()
    if options.secret then return string.rep("•", #text) end
    return text
  end
  local caret = ui.Rect {
    width = S(2),
    height = height - S(24),
    color = C.text,
    opacity = function() return (self.focused:get() and blink:get()) and 1 or 0 end,
  }
  local text = theme.text {
    text = shown,
    size = values.size or 16,
    color = C.text,
  }
  return ui.Rect {
    width = values.width or S(400),
    height = height,
    radius = S(values.radius or 14),
    color = values.color or C.card,
    border_color = C.edge,
    border_width = 1,
    -- The blink is a repaint every half second, so only while a page is
    -- open, which is the only time a field can be seen.
    ui.Timer {
      interval = 530, ["repeat"] = true,
      running = function() return require("island").page:get() ~= "" end,
      on_triggered = function() blink:set(not blink:get()) end,
    },
    theme.text {
      text = options.placeholder or "",
      size = values.size or 16,
      color = C.faint,
      anchors = { left = true, left_margin = S(20), top = true, top_margin = (height - S((values.size or 16) * 1.3)) / 2 },
      visible = function() return self.text:get() == "" end,
    },
    -- The row grows with the text; the clip keeps a long line inside.
    ui.ClipRect {
      color = "transparent",
      anchors = { left = true, right = true, top = true, bottom = true, left_margin = S(20), right_margin = S(20) },
      ui.Row {
        gap = S(2),
        align = "center",
        height = height,
        text,
        caret,
      },
    },
  }
end

return field
