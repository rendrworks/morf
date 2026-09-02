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
-- The board itself is `examples/lib/board.lua`, because it is also wanted inside
-- the greeter — a kiosk compositor shows one window, so a login screen cannot
-- put its keyboard in a second surface and see it. This file is the half that
-- is about having a surface of one's own; that file is the keyboard.
--
-- Two consequences worth being explicit about.
--
-- It must never take keyboard focus. A keyboard that holds focus types into
-- itself and the program the person is looking at receives nothing, so the
-- surface asks for `none` and means it.
--
-- And it emits *keycodes*, not text. `a` is evdev 30 and `A` is evdev 30 with
-- shift held, which is why the board hands over a code and a shift flag rather
-- than a character. This is the difference between an on-screen keyboard and a
-- text box: the receiving program sees a key press, so its own shortcuts,
-- repeat, and input method all behave exactly as they do for real hardware.

local morf = require("morf")
local ui = require("morf.ui")
local board = require("lib.board")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920

-- The bit the compositor's keymap gives shift. Group zero throughout: a layout
-- group is a whole other keymap, which is not what an on-screen shift means.
local SHIFT_MASK = 1

--- Presses and releases one key, holding shift across it when the key needs it.
---
--- The modifier is set before the press and cleared after the release rather
--- than being toggled around the whole gesture: a client reads the modifier
--- state that arrives with the key, and one left latched would capitalise
--- whatever the person typed next on their real keyboard.
local function press(code, shift)
  if shift then morf.virtual_keyboard.modifiers(SHIFT_MASK, 0, 0, 0) end
  morf.virtual_keyboard.key(code, true)
  morf.virtual_keyboard.key(code, false)
  if shift then morf.virtual_keyboard.modifiers(0, 0, 0, 0) end
end

local panel, height = board.build { width = W, key = press }

morf.surface.width = W
morf.surface.height = height
morf.surface.anchors = { left = true, right = true, bottom = true }
morf.surface.layer = "overlay"
-- The whole point. A keyboard that holds focus types into itself.
morf.surface.keyboard_focus = "none"
-- No exclusive zone: an on-screen keyboard floats over what it is typing into
-- rather than resizing it, because the thing being typed into is usually a
-- password field that has already been laid out.
morf.surface.exclusive_zone = 0

ui.Item { width = W, height = height, panel }
