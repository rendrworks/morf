-- The on-screen keyboard, as a board rather than as a program.
--
-- This is the keyboard. `examples/keyboard.lua` is this board given its own
-- surface so it can type into other programs; a greeter draws the same board
-- inside its own surface because a kiosk compositor shows one window and a
-- second surface is simply never seen. Two hosts, one keyboard — the layout,
-- the fusing keys, the layer morph and the shift behaviour exist once.
--
-- The seam between the board and whoever is hosting it is a single function:
-- `key(code, shift, label)`. The board knows which key was struck and nothing
-- about what that should do. The standalone surface turns it into an evdev
-- press through `zwp_virtual_keyboard_v1`; a greeter puts the label straight
-- into its own password field. Neither has to know about the other.
--
-- On the motion: pressing a key swells its field until it touches its
-- neighbours, and the field fuses them. That is not a picture of a merge — the
-- keys are one signed distance field with a seam radius, so the join is
-- computed at the edge and is exact at any size. It is also why a row is *one*
-- node with ten shapes rather than ten rectangles: a rectangle cannot fuse with
-- the rectangle beside it.

local morf = require("morf")
local ui = require("morf.ui")

local board = {}

--- Builds the board.
---
--- `width` is what it is laid out across, `x`/`y` where it goes, and `key` is
--- called with `(code, shift, label)` every time one is struck. Returns the
--- node to place and the height it came out at, because the host usually has to
--- make room for it.
function board.build(options)
  local key_sink = options.key
  local W = options.width
  -- A keypad rather than a keyboard, for a numeric password. It is the same
  -- board — the same fusing keys, the same press, the same seam — laid out
  -- three across instead of ten, with a key that opens the full one.
  local keypad = options.keypad or false
  local on_full = options.on_full
  local ORIGIN_X = options.x or 0
  local ORIGIN_Y = options.y or 0

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

  --- Hands one struck key to whoever is hosting the board.
  ---
  --- Shift is resolved here rather than there, because whether shift is held is a
  --- property of *this* board and not of the thing being typed into. What leaves
  --- is a key and whether it was shifted, which is all either host needs.
  local function emit(code, shift, label)
    local hold = shift or (layer:get() == LETTERS and shifted:get())
    key_sink(code, hold, hold and label and label:upper() or label)
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
  --
  -- A keypad is three across, so the same width buys keys three times the size
  -- — which is the point of it: a numeric password typed with a thumb wants big
  -- targets far more than it wants letters.
  local ACROSS = keypad and 3 or 10
  local BOARD_W = math.min(W - 40, keypad and 560 or 1100)
  local KEY = math.floor(BOARD_W / (ACROSS + 1.5))
  local GAP = math.floor((BOARD_W - KEY * ACROSS) / math.max(1, ACROSS - 1))
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
          emit(entry.code, entry.shift, entry.label)
        end,
      }
    end
    return entries
  end

  if keypad then
    -- 1 2 3 / 4 5 6 / 7 8 9 / keyboard 0 delete. The digits are evdev codes
    -- like everything else, so a host that turns them into presses and a host
    -- that reads the label both work without knowing this layout exists.
    local line = PAD
    for row = 0, 2 do
      local keys = {}
      for column = 1, 3 do
        local digit = row * 3 + column
        keys[#keys + 1] = {
          id = "n" .. digit,
          label = tostring(digit),
          tap = function() emit(DIGIT[digit], false, tostring(digit)) end,
        }
      end
      lay_row(line, keys)
      line = line + KEY + GAP
    end
    lay_row(line, {
      {
        id = "full",
        tone = "control",
        label = "abc",
        -- The way out. A numeric keypad with no way back is a screen that has
        -- decided for you what your password can be made of.
        tap = function() if on_full then on_full() end end,
      },
      { id = "n0", label = "0", tap = function() emit(DIGIT[10], false, "0") end },
      {
        id = "delete",
        tone = "control",
        label = "del",
        tap = function() emit(BACKSPACE, false, nil) end,
      },
    })
  else

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
      tap = function() emit(BACKSPACE, false, nil) end,
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
      { id = "comma", label = ",", tap = function() emit(COMMA, false, ",") end },
      { id = "space", units = 4, label = "space", tap = function() emit(SPACE, false, " ") end },
      { id = "dot", label = ".", tap = function() emit(DOT, false, ".") end },
      {
        id = "enter",
        units = 2,
        tone = "accent",
        label = "enter",
        tap = function() emit(ENTER, false, nil) end,
      },
    })
  end

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
    x = ORIGIN_X,
    y = ORIGIN_Y,
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

  return ui.Item(parts), BOARD_H
end

return board

