-- An on-screen keyboard, as its own process.
--
-- It types into *other* programs. Not through a socket, a pipe or an agreement
-- with the thing being typed into — through `zwp_virtual_keyboard_v1`, which
-- hands key events to the compositor, which routes them to whatever holds
-- focus. So this keyboard works with every Wayland client on the machine, and
-- nothing has to be written to accept it. Run it and type:
--
--     morf examples/keyboard.lua
--
-- Two consequences worth being explicit about.
--
-- It must never take keyboard focus. A keyboard that holds focus types into
-- itself and the program the person is looking at receives nothing, so the
-- surface asks for `none` and means it.
--
-- And it emits *keycodes*, not text. `a` is evdev 30 and `A` is evdev 30 with
-- shift held, which is why every key below carries a code and a shift flag
-- rather than a character. This is the difference between an on-screen keyboard
-- and a text box: the receiving program sees a key press, so its own shortcuts,
-- repeat, and input method all behave exactly as they do for real hardware.
--
-- The layout is US ANSI. A different layout is a different table at the top of
-- this file and nothing else, because the codes are positional — evdev 16 is
-- the key left of 17 whatever is printed on it.
--
-- On the motion: pressing a key swells its field until it touches its
-- neighbours, and the field fuses them. That is not a picture of a merge — the
-- keys are one signed distance field with a seam radius, so the join is
-- computed at the edge and is exact at any size. It is also why the keys are
-- *one* node with thirty shapes rather than thirty rectangles: a rectangle
-- cannot fuse with the rectangle beside it.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080

--------------------------------------------------------------------------------
-- Keys.
--------------------------------------------------------------------------------

-- evdev codes. Positional, so they hold for any printed layout.
local ESC, BACKSPACE, TAB, ENTER, SPACE = 1, 14, 15, 28, 57
local MINUS, EQUAL, LBRACE, RBRACE = 12, 13, 26, 27
local SEMICOLON, APOSTROPHE, GRAVE, BACKSLASH = 39, 40, 41, 43
local COMMA, DOT, SLASH = 51, 52, 53
local DIGIT = { 2, 3, 4, 5, 6, 7, 8, 9, 10, 11 }

--- One key: what it prints, which code produces it, and whether shift is held.
local function k(label, code, shift)
  return { label = label, code = code, shift = shift or false }
end

local function span(label, first, count)
  local row = {}
  for index = 0, count - 1 do
    row[#row + 1] = k(label:sub(index + 1, index + 1), first + index)
  end
  return row
end

local LETTERS, NUMBERS, SYMBOLS = 1, 2, 3

local LAYERS = {
  [LETTERS] = {
    span("qwertyuiop", 16, 10),
    span("asdfghjkl", 30, 9),
    span("zxcvbnm", 44, 7),
  },
  [NUMBERS] = {
    { k("1", DIGIT[1]), k("2", DIGIT[2]), k("3", DIGIT[3]), k("4", DIGIT[4]), k("5", DIGIT[5]),
      k("6", DIGIT[6]), k("7", DIGIT[7]), k("8", DIGIT[8]), k("9", DIGIT[9]), k("0", DIGIT[10]) },
    { k("-", MINUS), k("/", SLASH), k(":", SEMICOLON, true), k(";", SEMICOLON),
      k("(", DIGIT[9], true), k(")", DIGIT[10], true), k("$", DIGIT[4], true),
      k("&", DIGIT[7], true), k("@", DIGIT[2], true) },
    { k(".", DOT), k(",", COMMA), k("?", SLASH, true), k("!", DIGIT[1], true),
      k("'", APOSTROPHE), k('"', APOSTROPHE, true), k("~", GRAVE, true) },
  },
  [SYMBOLS] = {
    { k("[", LBRACE), k("]", RBRACE), k("{", LBRACE, true), k("}", RBRACE, true),
      k("#", DIGIT[3], true), k("%", DIGIT[5], true), k("^", DIGIT[6], true),
      k("*", DIGIT[8], true), k("+", EQUAL, true), k("=", EQUAL) },
    { k("_", MINUS, true), k("\\", BACKSLASH), k("|", BACKSLASH, true), k("<", COMMA, true),
      k(">", DOT, true), k("~", GRAVE, true), k("`", GRAVE), k(";", SEMICOLON),
      k(":", SEMICOLON, true) },
    { k(".", DOT), k(",", COMMA), k("?", SLASH, true), k("!", DIGIT[1], true),
      k("'", APOSTROPHE), k('"', APOSTROPHE, true), k("=", EQUAL) },
  },
}

--------------------------------------------------------------------------------
-- Emitting.
--------------------------------------------------------------------------------

-- The bit the compositor's keymap gives shift. Group zero throughout: a layout
-- group is a whole other keymap, which is not what an on-screen shift means.
local SHIFT_MASK = 1

local layer = morf.signal("keyboard.layer", LETTERS)
local shifted = morf.signal("keyboard.shifted", false)
local locked = morf.signal("keyboard.locked", false)

-- Whether the board is mid-morph. While it is, every key swells past its
-- neighbours and the row's field fuses them into one bar, and the letters melt
-- out of their own distance fields. The layer is swapped at the bottom of that,
-- where there is nothing legible on screen to swap — so the board is never seen
-- to *replace* its keys, only to re-form as different ones.
local fusing = morf.signal("keyboard.fusing", false)

-- The layer being travelled towards, and how far along the letters are. While
-- these differ from `layer` every label carries two glyphs and the renderer
-- interpolates between them as distance fields — so `q` reaches `1` through
-- outlines that belong to neither, rather than one letter fading out under
-- another. When the swap lands, `layer` catches up and both name the same
-- glyph, which is why the progress can be dropped without anything jumping.
local target_layer = morf.signal("keyboard.target_layer", LETTERS)
local travel = morf.signal("keyboard.travel", 0)

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--- Presses and releases one key, holding shift across it when the key needs it.
---
--- The modifier is set before the press and cleared after the release rather
--- than being toggled around the whole gesture: a client reads the modifier
--- state that arrives with the key, and one left latched would capitalise
--- whatever the person typed next on their real keyboard.
local function emit(code, shift)
  local hold = shift or (layer:get() == LETTERS and shifted:get())
  if hold then morf.virtual_keyboard.modifiers(SHIFT_MASK, 0, 0, 0) end
  morf.virtual_keyboard.key(code, true)
  morf.virtual_keyboard.key(code, false)
  if hold then morf.virtual_keyboard.modifiers(0, 0, 0, 0) end
  -- Single-shot unless locked, the way every touch keyboard behaves: one
  -- capital is what shift is nearly always wanted for.
  if shifted:get() and not locked:get() then write(shifted, false) end
end

-- How long the board spends liquid, and how long a key waits behind the one
-- to its left. The stagger is what makes it a wave rather than a blink: ten
-- columns at `MORPH_STAGGER` apart still finish inside `MORPH_MELT`, so every
-- key has fused before the layer underneath changes.
local MORPH_MELT = 210
local MORPH_REFORM = 150
local MORPH_STAGGER = 14

local morph_timer
local morph_target = LETTERS
local morph_stage = 0

--- Melts the board, changes the layer underneath it, and lets it re-form.
local function switch_layer(target)
  if morph_stage ~= 0 then return end
  morph_target = target
  morph_stage = 1
  write(target_layer, target)
  write(travel, 1)
  write(fusing, true)
  morph_timer.interval = MORPH_MELT
  morph_timer.running = true
end

--------------------------------------------------------------------------------
-- Shape.
--------------------------------------------------------------------------------

-- Full width on a phone, a centred board on a desktop. A key wants to be about
-- a finger wide wherever it is, and ten of them across a 4K panel would not be.
local BOARD_W = math.min(W - 40, 1100)
local KEY = math.floor(BOARD_W / 11.5)
local GAP = math.floor((BOARD_W - KEY * 10) / 9)
local PAD = math.floor(KEY * 0.22)
local BOARD_H = KEY * 4 + GAP * 3 + PAD * 2
local BOARD_X = math.floor((W - BOARD_W) / 2)

-- How far a pressed key grows on every side, and the seam radius the field
-- joins with. The two are chosen against the gap: a resting key sits `GAP`
-- from its neighbour and stays separate because the gap is wider than the
-- seam, and a pressed key closes that to `GAP - SWELL`, which is inside the
-- seam — so it fuses. Change one and the merge either never happens or never
-- stops.
local SWELL = math.floor(GAP * 0.62)
local SEAM = math.floor(GAP * 0.85)
-- Enough to close the gap outright rather than merely reach across it: a press
-- should read as two keys touching, a morph as the row having no keys in it.
local FUSE = math.floor(GAP * 0.95)

morf.surface.width = W
morf.surface.height = BOARD_H
morf.surface.anchors = { left = true, right = true, bottom = true }
morf.surface.layer = "overlay"
-- The whole point. A keyboard that holds focus types into itself.
morf.surface.keyboard_focus = "none"
-- No exclusive zone: an on-screen keyboard floats over what it is typing into
-- rather than resizing it, because the thing being typed into is usually a
-- password field that has already been laid out.
morf.surface.exclusive_zone = 0

local PANEL = "#0c1017e6"
local KEYFACE = "#243043"
local LIVE = "#6fb3cc"
local EDGE = "#33415a"
local LABEL = "#e9edf5"
local DIM = "#8a97ad"

--------------------------------------------------------------------------------
-- Laying the board out.
--------------------------------------------------------------------------------

-- Every key is described once, then drawn three times over: as a shape in the
-- shared field, as a label, and as a target for the pointer. Keeping the three
-- passes separate is what puts the whole field under every label and every
-- label under every touch target, without depending on the order keys happen
-- to be declared in.
local keys = {}
local rows = {}

local function place(entry)
  entry.down = morf.signal("keyboard.down." .. entry.id, false)
  keys[#keys + 1] = entry
  return entry
end

local function row_width(units_list)
  local total = -GAP
  for _, units in ipairs(units_list) do
    total = total + math.floor(KEY * units + GAP * (units - 1)) + GAP
  end
  return total
end

--- Lays out one row, centred, from a list of `{ units, ... }` descriptors.
local function lay_row(y, entries)
  local row = { y = y, keys = {} }
  rows[#rows + 1] = row
  local widths = {}
  for index, entry in ipairs(entries) do widths[index] = entry.units or 1 end
  local x = BOARD_X + math.floor((BOARD_W - row_width(widths)) / 2)
  for index, entry in ipairs(entries) do
    local units = widths[index]
    entry.x = x
    entry.y = y
    entry.width = math.floor(KEY * units + GAP * (units - 1))
    entry.height = KEY
    entry.id = entry.id or ("r" .. y .. "c" .. index)
    entry.order = index - 1
    place(entry)
    row.keys[#row.keys + 1] = entry
    x = x + entry.width + GAP
  end
end

--- The key at row `row`, slot `slot` of whichever layer is showing.
local function slot(row, slot_index)
  local entry = LAYERS[layer:get()][row][slot_index]
  return entry or LAYERS[LETTERS][row][slot_index]
end

--- What that key prints on a given layer, which is not always the one showing:
--- a morphing key has to name the letter it is turning into as well.
local function slot_label(row, slot_index, which)
  local entry = LAYERS[which][row][slot_index] or LAYERS[LETTERS][row][slot_index]
  if which == LETTERS and shifted:get() then return entry.label:upper() end
  return entry.label
end

local function character_row(row, count)
  local entries = {}
  for index = 1, count do
    entries[#entries + 1] = {
      id = "k" .. row .. "_" .. index,
      label = function(which) return slot_label(row, index, which) end,
      tap = function()
        local entry = slot(row, index)
        emit(entry.code, entry.shift)
      end,
    }
  end
  return entries
end

local line = PAD
lay_row(line, character_row(1, 10))
line = line + KEY + GAP
lay_row(line, character_row(2, 9))
line = line + KEY + GAP
local row_three = {
  {
    id = "modifier",
    units = 1.5,
    tone = function()
      if layer:get() ~= LETTERS then return "control" end
      return (locked:get() or shifted:get()) and "live" or "control"
    end,
    label = function(which)
      if which == NUMBERS then return "#+=" end
      if which == SYMBOLS then return "123" end
      return locked:get() and "SHIFT" or "shift"
    end,
    -- On the letters layer this is shift, and a second press locks it: the
    -- ordinary touch-keyboard gesture, and the only way to type a run of
    -- capitals without holding anything.
    tap = function()
      if layer:get() == NUMBERS then
        switch_layer(SYMBOLS)
      elseif layer:get() == SYMBOLS then
        switch_layer(NUMBERS)
      elseif locked:get() then
        write(locked, false)
        write(shifted, false)
      elseif shifted:get() then
        write(locked, true)
      else
        write(shifted, true)
      end
    end,
  },
}
for _, entry in ipairs(character_row(3, 7)) do row_three[#row_three + 1] = entry end
row_three[#row_three + 1] = {
  id = "delete",
  units = 1.5,
  tone = "control",
  label = "del",
  tap = function() emit(BACKSPACE) end,
}
lay_row(line, row_three)

line = line + KEY + GAP
lay_row(line, {
  {
    id = "layer",
    units = 1.5,
    tone = "control",
    label = function(which) return which == LETTERS and "123" or "abc" end,
    tap = function() switch_layer(layer:get() == LETTERS and NUMBERS or LETTERS) end,
  },
  { id = "comma", label = ",", tap = function() emit(COMMA) end },
  { id = "space", units = 4, label = "space", tap = function() emit(SPACE) end },
  { id = "dot", label = ".", tap = function() emit(DOT) end },
  {
    id = "enter",
    units = 2,
    tone = "accent",
    label = "enter",
    tap = function() emit(ENTER) end,
  },
})

--------------------------------------------------------------------------------
-- Drawing it.
--------------------------------------------------------------------------------

local function tone_of(entry)
  return type(entry.tone) == "function" and entry.tone() or entry.tone
end

local function text_of(entry, which)
  return type(entry.label) == "function" and entry.label(which) or entry.label
end

-- Pass one: each row as its own distance field.
--
-- The seam is what makes a press fluid. `smooth_union` joins two shapes over
-- `blend` of distance, so a key growing into its neighbour's gap pulls a neck
-- out of it rather than overlapping it — the join is solved at the edge, per
-- pixel, which is why it stays exact when the board is scaled for a phone.
--
-- One field *per row* rather than one for the board, for two reasons that
-- happen to agree. A field composes at most sixteen layers, and forty keys is
-- well past that — beyond the cap the extra shapes are silently dropped, which
-- is what a whole missing bottom half of a keyboard looks like. And a
-- composition is resolved in one fragment shader, so every layer costs every
-- pixel of the node: forty layers over the whole board is forty evaluations a
-- pixel, where ten over a single row's strip is ten. Keys only ever fuse with
-- the ones beside them anyway, which is exactly what a row contains.
local function row_field(row)
  local top = row.y - SWELL
  local field = {
    x = 0,
    y = top,
    width = W,
    height = KEY + SWELL * 2,
    fill_color = KEYFACE,
    blend = SEAM,
  }
  for index, entry in ipairs(row.keys) do
    -- How far this key is currently past its own edges: nothing at rest, a
    -- little under a finger, and enough to swallow the gap while the board is
    -- morphing.
    local function grown()
      if fusing:get() then return FUSE end
      return entry.down:get() and SWELL or 0
    end
    -- Each key waits a little longer than the one to its left, so the row
    -- liquefies as a wave running across it instead of blinking.
    local lag = entry.order * MORPH_STAGGER
    local function swell_motion()
      return { kind = "spring", mass = 1, damping = 15, stiffness = 380, epsilon = 0.01,
               delay = lag }
    end
    field[#field + 1] = ui.SdfShape {
      -- Positioned against the strip, so the shape's own coordinates stay
      -- inside the node the field resolves over.
      x = function() return entry.x - grown() end,
      y = function() return entry.y - top - grown() end,
      width = function() return entry.width + grown() * 2 end,
      height = function() return entry.height + grown() * 2 end,
      shape = "box",
      -- The corners round right out as the key dissolves, so what the row fuses
      -- into is a bar and not a slab with square ends.
      radius = function()
        return math.floor(KEY * (fusing:get() and 0.5 or 0.26))
      end,
      operation = index == 1 and "union" or "smooth_union",
      behavior = {
        x = swell_motion(),
        y = swell_motion(),
        width = swell_motion(),
        height = swell_motion(),
        radius = { duration = MORPH_MELT, easing = "in_out_quad", delay = lag },
      },
    }
  end
  return ui.Sdf(field)
end

local parts = {
  width = W,
  height = BOARD_H,
  ui.Rect {
    width = W,
    height = BOARD_H,
    color = PANEL,
  },
}
for _, row in ipairs(rows) do
  parts[#parts + 1] = row_field(row)
end

-- Pass two: the labels, over the whole field.
for _, entry in ipairs(keys) do
  parts[#parts + 1] = ui.Text {
    x = entry.x,
    y = entry.y,
    width = entry.width,
    height = entry.height,
    text = function() return text_of(entry, layer:get()) end,
    -- The letter this one is turning into, and how far it has got. Both are
    -- ordinary properties, so the morph is animated by the engine and Lua
    -- writes the target once.
    morph_to = function() return text_of(entry, target_layer:get()) end,
    morph_progress = function() return travel:get() end,
    font_size = function()
      return #text_of(entry, layer:get()) > 2 and math.floor(KEY * 0.26)
        or math.floor(KEY * 0.40)
    end,
    font_weight = 500,
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = function()
      if entry.down:get() then return "#0a0e14" end
      if tone_of(entry) == "live" then return LIVE end
      return tone_of(entry) == "control" and DIM or LABEL
    end,
    behavior = {
      color = { duration = 110, easing = "out_quad" },
      -- Each letter starts a little after the one to its left, so the change
      -- reads as a wave crossing the board rather than every key turning over
      -- at once.
      morph_progress = { duration = MORPH_MELT, easing = "in_out_quad",
                         delay = entry.order * MORPH_STAGGER },
    },
  }
end

-- Pass three: the touch targets, over the labels.
for _, entry in ipairs(keys) do
  parts[#parts + 1] = ui.MouseArea {
    x = entry.x,
    y = entry.y,
    width = entry.width,
    height = entry.height,
    -- Pressed and released, not clicked-only: the swell has to start when the
    -- finger lands, and a click arrives after it lifts. `on_exited` covers a
    -- finger that slides off the key, which leaves no release behind.
    on_pressed = function() write(entry.down, true) end,
    on_released = function() write(entry.down, false) end,
    on_exited = function() write(entry.down, false) end,
    on_clicked = entry.tap,
  }
end

-- One timer runs the whole morph. Forty `on_finished` handlers would each fire
-- their own swap, and the board would change layer forty times in a row.
morph_timer = ui.Timer {
  interval = MORPH_MELT,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    if morph_stage == 1 then
      -- The bottom of the melt: nothing legible is on screen, so this is where
      -- the keys become different keys.
      write(layer, morph_target)
      write(travel, 0)
      write(shifted, false)
      write(locked, false)
      write(fusing, false)
      morph_stage = 2
      morph_timer.interval = MORPH_REFORM
      morph_timer.running = true
    else
      morph_stage = 0
    end
  end,
}
parts[#parts + 1] = morph_timer

ui.Item(parts)
