-- A login screen you can drive without a keyboard.
--
-- `greeter.lua` is the minimal one: it shows the protocol and nothing else, and
-- it assumes a physical keyboard. This is the same greetd conversation wearing
-- a face — every account is a tile, every session a pill, and the keyboard is
-- on the screen, because the machine a greeter runs on is not always one you
-- can already type into. A tablet, a kiosk, a machine whose keyboard layout is
-- the thing being configured: all of them can still log in here.
--
-- Try it nested, inside a session you are already in:
--
--     cage -- morf examples/greeter-touch.lua
--
-- `cage` has no layer shell, so morf stands the surface up as a fullscreen
-- toplevel instead. Nothing below has to know that.
--
-- Deployment is `greeter.lua`'s header verbatim — the same
-- `/etc/greetd/config.toml`, the same warning that it runs as user `greeter`
-- and not as you.
--
-- On motion: nothing here animates from Lua. Every transition is a `behavior`
-- on a property, so Lua writes a target once and morf's frame tick carries it
-- there. The drift in the background is a single timer flipping one number
-- every few seconds; everything else is a spring reacting to a tap.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local io = require("morf.io")

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080

morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = "overlay"
morf.surface.keyboard_focus = "exclusive"

-- A greeter is drawn once and looked at from a metre away, so it is sized in
-- proportion to the screen rather than in fixed pixels. Clamped at the bottom
-- so a small nested `cage` window stays usable, and at the top so a 4K panel
-- does not turn the thing into signage.
local SCALE = math.max(0.75, math.min(1.6, math.min(W / 1920, H / 1080)))
local function s(n) return math.floor(n * SCALE) end

local INK = "#080b11"
local PANEL = "#141a26"
local RAISED = "#1c2634"
local LINE = "#2a3546"
local TEXT = "#e9edf5"
local MUTED = "#78849a"
local ACCENT = "#6fb3cc"
local ACCENT_IN = "#16323f"
local ALERT = "#e5735a"

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--------------------------------------------------------------------------------
-- Who can log in, and into what.
--------------------------------------------------------------------------------

--- Ordinary human accounts, read straight out of `/etc/passwd`.
---
--- No D-Bus and no AccountsService: the file is world-readable, it is the
--- authority, and a login screen that cannot list users because a daemon is not
--- running is worse than one that reads a file. Below 1000 is the system's own,
--- and 65534 is `nobody`.
local function accounts()
  local found = {}
  local text = io.file("/etc/passwd"):read()
  if not text then return found end
  for line in text:gmatch("[^\n]+") do
    local name, uid, gecos, shell = line:match("^([^:]+):[^:]*:(%d+):[^:]*:([^:]*):[^:]*:([^:]*)$")
    local id = tonumber(uid or "")
    if name and id and id >= 1000 and id < 65534 then
      -- A shell that cannot be logged into is an account that cannot log in.
      if not (shell:match("nologin$") or shell:match("/false$")) then
        local label = (gecos ~= "" and gecos:match("^[^,]*")) or name
        found[#found + 1] = {
          name = name,
          label = label,
          initial = label:sub(1, 1):upper(),
        }
      end
    end
  end
  table.sort(found, function(a, b) return a.name < b.name end)
  return found
end

--- The sessions this machine can start, as ordinary desktop entries.
local function sessions()
  local found = {}
  for _, entry in ipairs(core.desktop_entries(core.session_paths()):applications()) do
    if entry.exec and entry.exec ~= "" then
      found[#found + 1] = { name = entry.name, exec = entry.exec }
    end
  end
  return found
end

local users = accounts()
local available = sessions()

--------------------------------------------------------------------------------
-- State.
--------------------------------------------------------------------------------

-- The password itself is a plain local and never a signal. Signals are named,
-- observable and interpolated; a secret wants none of those. What the screen
-- needs is how *many* characters have been typed, which is all `typed` carries.
local password = ""

local chosen_user = morf.signal("greeter.user", 1)
local chosen_session = morf.signal("greeter.session", 1)
local typed = morf.signal("greeter.typed", 0)
local shifted = morf.signal("greeter.shifted", false)
local keys_open = morf.signal("greeter.keys_open", true)
local working = morf.signal("greeter.working", false)
local alarmed = morf.signal("greeter.alarmed", false)
local tide = morf.signal("greeter.tide", 0)
local message = morf.signal("greeter.message", "enter password")

local function say(text, bad)
  write(message, text)
  write(alarmed, bad or false)
end

local function clear_password()
  password = ""
  write(typed, 0)
end

--------------------------------------------------------------------------------
-- Talking to greetd.
--------------------------------------------------------------------------------

--- Runs one login attempt to completion.
---
--- greetd's protocol is a conversation: a session is created, PAM asks whatever
--- it wants to ask, and each answer may produce another question. This handles
--- the common shape — one password prompt — and cancels on anything it does not
--- understand rather than guessing, because a greeter that guesses at an
--- authentication prompt is answering a question it did not read.
local function attempt()
  if working:get() or #users == 0 or #available == 0 then return end
  write(working, true)
  say("checking")

  local ok, session = pcall(morf.greetd.connect)
  if not (ok and session) then
    say("no greetd here — run me under greetd", true)
    write(working, false)
    clear_password()
    return
  end

  local reply = session:create_session(users[chosen_user:get()].name)
  while reply and reply.type == "auth_message" do
    if reply.auth_message_type == "secret" or reply.auth_message_type == "visible" then
      reply = session:respond(password)
    else
      -- An informational or error message carries no answer.
      reply = session:respond(nil)
    end
  end

  if reply and reply.type == "success" then
    local wanted = available[chosen_session:get()]
    local started = session:start_session({ wanted.exec }, {})
    if started and started.type == "success" then
      say("starting " .. wanted.name)
      -- greetd takes it from here: this process is about to be replaced by the
      -- session it just asked for.
      return
    end
    say((started and started.description) or "could not start the session", true)
    session:cancel_session()
  else
    say((reply and reply.description) or "that did not work", true)
    session:cancel_session()
  end

  clear_password()
  write(working, false)
end

--- Asks logind to suspend, reboot or power off.
---
--- The `false` is logind's `interactive` flag: a greeter has nobody to prompt
--- for a polkit password, so an interactive check would hang waiting for an
--- agent that is not running.
local function power(method)
  local ok, manager = pcall(io.dbus.proxy, "system", "org.freedesktop.login1",
                            "/org/freedesktop/login1", "org.freedesktop.login1.Manager")
  if not (ok and manager) then
    say("cannot reach logind", true)
    return
  end
  local called, err = pcall(manager.call_with, manager, method, false)
  if not called then say(tostring(err), true) end
end

--------------------------------------------------------------------------------
-- Typing.
--------------------------------------------------------------------------------

local CAPACITY = 32

local function type_character(character)
  if working:get() then return end
  if #password >= CAPACITY then return end
  password = password .. character
  write(typed, #password)
  if alarmed:get() then say("enter password") end
end

local function backspace()
  if working:get() then return end
  password = password:sub(1, -2)
  write(typed, #password)
end

local function pick_user(index)
  if working:get() then return end
  write(chosen_user, index)
  clear_password()
  say("enter password")
end

--------------------------------------------------------------------------------
-- The background.
--------------------------------------------------------------------------------

-- Three soft fields leaning one way and then the other. One timer flips `tide`
-- every few seconds and the long easing does the rest, so the whole thing costs
-- Lua one call per flip and nothing per frame.
local function drift(index, home_x, home_y, radius, colour, reach)
  return ui.Sdf {
    x = function() return home_x + (tide:get() == 1 and reach or -reach) end,
    y = function() return home_y + (tide:get() == 1 and -reach or reach) end,
    width = radius * 2,
    height = radius * 2,
    fill_color = colour,
    opacity = 0.55,
    -- The one knob that turns a crisp field edge into a glow. A field is
    -- resolution independent, so this is a soft shape rather than a blurred
    -- picture of one, and it costs the same at any size.
    softness = radius * 0.85,
    behavior = {
      x = { duration = 7000 + index * 900, easing = "in_out_sine" },
      y = { duration = 8200 - index * 700, easing = "in_out_sine" },
    },
    ui.SdfShape {
      width = radius * 2,
      height = radius * 2,
      shape = "circle",
    },
  }
end

--------------------------------------------------------------------------------
-- Accounts and sessions.
--------------------------------------------------------------------------------

local TILE = s(104)
local TILE_GAP = s(28)

local function avatar(index, user)
  local function mine() return chosen_user:get() == index end
  return ui.Item {
    width = TILE,
    height = TILE + s(36),
    ui.Rect {
      width = TILE,
      height = TILE,
      radius = TILE / 2,
      color = function() return mine() and ACCENT_IN or PANEL end,
      border_width = s(2),
      border_color = function() return mine() and ACCENT or LINE end,
      -- A spring rather than a duration: a tap should feel answered, and the
      -- small overshoot is what reads as an answer.
      scale = function() return mine() and 1.06 or 1.0 end,
      behavior = {
        color = { duration = 220, easing = "out_quad" },
        border_color = { duration = 220, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 13, stiffness = 240, epsilon = 0.001 },
      },
    },
    ui.Text {
      width = TILE,
      height = TILE,
      y = math.floor(TILE / 2) - s(20),
      text = user.initial,
      font_size = s(34),
      font_weight = 600,
      horizontal_alignment = "center",
      color = function() return mine() and TEXT or MUTED end,
      behavior = { color = { duration = 220, easing = "out_quad" } },
    },
    ui.Text {
      y = TILE + s(10),
      width = TILE,
      text = user.label,
      font_size = s(13),
      horizontal_alignment = "center",
      elide = "right",
      color = function() return mine() and TEXT or MUTED end,
      behavior = { color = { duration = 220, easing = "out_quad" } },
    },
    ui.MouseArea {
      width = TILE,
      height = TILE + s(36),
      on_clicked = function() pick_user(index) end,
    },
  }
end

local function pill(index, entry, width)
  local function mine() return chosen_session:get() == index end
  return ui.Item {
    width = width,
    height = s(34),
    ui.Rect {
      anchors = { fill = true },
      radius = s(17),
      color = function() return mine() and ACCENT_IN or "#00000000" end,
      border_width = s(1),
      border_color = function() return mine() and ACCENT or LINE end,
      behavior = {
        color = { duration = 200, easing = "out_quad" },
        border_color = { duration = 200, easing = "out_quad" },
      },
    },
    ui.Text {
      anchors = { fill = true },
      text = entry.name,
      font_size = s(13),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      elide = "right",
      color = function() return mine() and TEXT or MUTED end,
      behavior = { color = { duration = 200, easing = "out_quad" } },
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_clicked = function()
        if not working:get() then write(chosen_session, index) end
      end,
    },
  }
end

--------------------------------------------------------------------------------
-- The keyboard on the screen.
--------------------------------------------------------------------------------

local KEY = s(74)
local KEY_GAP = s(9)
local BOARD_W = KEY * 10 + KEY_GAP * 9
local BOARD_X = math.floor((W - BOARD_W) / 2)

-- Digits carry their shifted symbols because a password that needs one should
-- not require finding a physical keyboard to type it.
local SHIFTED_DIGIT = {
  ["1"] = "!", ["2"] = "@", ["3"] = "#", ["4"] = "$", ["5"] = "%",
  ["6"] = "^", ["7"] = "&", ["8"] = "*", ["9"] = "(", ["0"] = ")",
}

--- One key. `units` is its width as a multiple of the square key.
local function keycap(label, x, y, units, tone, on_tap)
  local width = math.floor(KEY * units + KEY_GAP * (units - 1))
  local pressed = morf.signal("greeter.key." .. label .. "." .. x .. "." .. y, false)
  return ui.Item {
    x = x,
    y = y,
    width = width,
    height = KEY,
    ui.Rect {
      anchors = { fill = true },
      radius = s(12),
      color = function()
        if pressed:get() then return ACCENT end
        return tone == "accent" and ACCENT_IN or RAISED
      end,
      border_width = s(1),
      border_color = function() return pressed:get() and ACCENT or LINE end,
      -- Stiff and well damped: the cap should be down before the eye reaches
      -- it and back without a wobble, which is the whole of what makes a key
      -- feel answered rather than animated.
      scale = function() return pressed:get() and 0.94 or 1.0 end,
      behavior = {
        color = { duration = 90, easing = "out_quad" },
        border_color = { duration = 90, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 16, stiffness = 420, epsilon = 0.001 },
      },
    },
    ui.Text {
      anchors = { fill = true },
      text = label,
      vertical_alignment = "center",
      font_size = s(17),
      font_weight = 500,
      horizontal_alignment = "center",
      color = function() return pressed:get() and INK or TEXT end,
      behavior = { color = { duration = 90, easing = "out_quad" } },
    },
    ui.MouseArea {
      anchors = { fill = true },
      -- Pressed, not hovered: a touchscreen has no hover to give, and a key
      -- that only lights under a pointer would stay dark on the machines this
      -- keyboard exists for. `on_exited` covers a finger sliding off the key
      -- before it lifts, which leaves no release behind.
      on_pressed = function() write(pressed, true) end,
      on_released = function() write(pressed, false) end,
      on_exited = function() write(pressed, false) end,
      on_clicked = on_tap,
    },
  }
end

--- Lays one row of keys out centred on the board.
local function keyrow(y, entries)
  local total = 0
  for _, entry in ipairs(entries) do
    total = total + KEY * entry.units + KEY_GAP * (entry.units - 1) + KEY_GAP
  end
  total = total - KEY_GAP

  local nodes = {}
  local x = BOARD_X + math.floor((BOARD_W - total) / 2)
  for _, entry in ipairs(entries) do
    local width = math.floor(KEY * entry.units + KEY_GAP * (entry.units - 1))
    nodes[#nodes + 1] = keycap(entry.label, x, y, entry.units, entry.tone, entry.tap)
    x = x + width + KEY_GAP
  end
  return nodes
end

local function letters(characters)
  local entries = {}
  for character in characters:gmatch(".") do
    entries[#entries + 1] = {
      label = character,
      units = 1,
      tap = function()
        type_character(shifted:get() and character:upper() or character)
      end,
    }
  end
  return entries
end

local function digits(characters)
  local entries = {}
  for character in characters:gmatch(".") do
    entries[#entries + 1] = {
      label = character,
      units = 1,
      tap = function()
        type_character(shifted:get() and SHIFTED_DIGIT[character] or character)
      end,
    }
  end
  return entries
end

local function punctuation(characters)
  local entries = {}
  for character in characters:gmatch(".") do
    entries[#entries + 1] = { label = character, units = 1,
      tap = function() type_character(character) end }
  end
  return entries
end

-- The board is one item that slides as a whole, so hiding it is a single
-- animated number rather than a rebuild.
local BOARD_H = KEY * 5 + KEY_GAP * 4 + s(36)
local BOARD_OPEN_Y = H - BOARD_H - s(24)

local rows = {}
local function add(list)
  for _, node in ipairs(list) do rows[#rows + 1] = node end
end

local line_y = s(18)
add(keyrow(line_y, digits("1234567890")))
line_y = line_y + KEY + KEY_GAP
add(keyrow(line_y, letters("qwertyuiop")))
line_y = line_y + KEY + KEY_GAP
add(keyrow(line_y, letters("asdfghjkl")))
line_y = line_y + KEY + KEY_GAP
do
  local row = { { label = "shift", units = 1.5, tone = "accent",
                  tap = function() write(shifted, not shifted:get()) end } }
  for _, entry in ipairs(letters("zxcvbnm")) do row[#row + 1] = entry end
  row[#row + 1] = { label = "del", units = 1.5, tone = "accent", tap = backspace }
  add(keyrow(line_y, row))
end
line_y = line_y + KEY + KEY_GAP
do
  local row = punctuation("-_.@/")
  table.insert(row, { label = "space", units = 3.5,
                      tap = function() type_character(" ") end })
  table.insert(row, { label = "log in", units = 2, tone = "accent", tap = function() attempt() end })
  add(keyrow(line_y, row))
end

-- The panel and its keys go in as one list, for the reason given at the tree
-- below: a `table.unpack` here would keep the first key and drop the rest.
local board_parts = {
  x = 0,
  y = function() return keys_open:get() and BOARD_OPEN_Y or H + s(20) end,
  width = W,
  height = BOARD_H,
  opacity = function() return keys_open:get() and 1 or 0 end,
  -- A spring, so the board arrives with a little weight instead of stopping
  -- dead on the frame the easing runs out.
  behavior = {
    y = { kind = "spring", mass = 1, damping = 20, stiffness = 150, epsilon = 0.5 },
    opacity = { duration = 200, easing = "out_quad" },
  },
  ui.Rect {
    x = BOARD_X - s(18),
    width = BOARD_W + s(36),
    height = BOARD_H,
    radius = s(22),
    color = "#0d121bd0",
    border_width = s(1),
    border_color = LINE,
  },
}
for _, node in ipairs(rows) do
  board_parts[#board_parts + 1] = node
end

local board = ui.Item(board_parts)

--------------------------------------------------------------------------------
-- The card, and everything above the keyboard.
--------------------------------------------------------------------------------

local CARD_W = math.min(s(560), W - s(80))
local CARD_H = s(150)
local CARD_X = math.floor((W - CARD_W) / 2)
local CARD_Y = math.floor(H * 0.35)

local DOTS = 14
local DOT = s(11)
local DOT_GAP = s(10)

local function dots()
  local nodes = {}
  local span = DOTS * DOT + (DOTS - 1) * DOT_GAP
  local left = CARD_X + math.floor((CARD_W - span) / 2)
  for index = 1, DOTS do
    nodes[#nodes + 1] = ui.Rect {
      x = left + (index - 1) * (DOT + DOT_GAP),
      y = CARD_Y + s(100),
      width = DOT,
      height = DOT,
      radius = DOT / 2,
      color = function()
        if alarmed:get() then return ALERT end
        return typed:get() >= index and ACCENT or LINE
      end,
      -- Each dot lands a fraction after the one before it, so a typed
      -- character reads as one thing arriving rather than a row redrawing.
      scale = function() return typed:get() >= index and 1.0 or 0.55 end,
      behavior = {
        color = { duration = 160, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 12, stiffness = 320, epsilon = 0.001 },
      },
    }
  end
  return nodes
end

local strip_span = #users * TILE + math.max(0, #users - 1) * TILE_GAP
local strip_x = math.floor((W - strip_span) / 2)
local strip_y = math.floor(H * 0.19)

local avatars = {}
for index, user in ipairs(users) do
  local tile = avatar(index, user)
  tile.x = strip_x + (index - 1) * (TILE + TILE_GAP)
  tile.y = strip_y
  avatars[#avatars + 1] = tile
end

local PILL_W = s(190)
local pill_span = #available * PILL_W + math.max(0, #available - 1) * s(12)
local pill_x = math.floor((W - pill_span) / 2)
local pill_y = CARD_Y + CARD_H + s(22)

local pills = {}
for index, entry in ipairs(available) do
  local node = pill(index, entry, PILL_W)
  node.x = pill_x + (index - 1) * (PILL_W + s(12))
  node.y = pill_y
  pills[#pills + 1] = node
end

--- A small labelled button, for the things that are not logging in.
---
--- Words rather than symbols: ⏻ is in most fonts but ⟳ and ⌨ are not, and a
--- greeter that renders a tofu box has told the person in front of it nothing.
--- A greeter is also the one screen where guessing wrong is expensive.
local ACTION_W = s(96)
local function action(x, y, label, on_tap)
  local hot = morf.signal("greeter.action." .. label, false)
  return ui.Item {
    x = x, y = y, width = ACTION_W, height = s(34),
    ui.Rect {
      anchors = { fill = true },
      radius = s(17),
      color = function() return hot:get() and RAISED or "#00000000" end,
      border_width = s(1),
      border_color = function() return hot:get() and ACCENT or LINE end,
      behavior = {
        color = { duration = 160, easing = "out_quad" },
        border_color = { duration = 160, easing = "out_quad" },
      },
    },
    ui.Text {
      anchors = { fill = true },
      text = label,
      font_size = s(13),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      color = function() return hot:get() and TEXT or MUTED end,
      behavior = { color = { duration = 160, easing = "out_quad" } },
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = on_tap,
    },
  }
end

local clock = ui.Text {
  y = math.floor(H * 0.06),
  width = W,
  text = "",
  font_size = s(76),
  font_weight = 300,
  horizontal_alignment = "center",
  color = TEXT,
}

local today = ui.Text {
  y = math.floor(H * 0.06) + s(88),
  width = W,
  text = "",
  font_size = s(15),
  horizontal_alignment = "center",
  color = MUTED,
}

--------------------------------------------------------------------------------
-- The tree.
--------------------------------------------------------------------------------

-- Children are collected into one list rather than written inline, because a
-- `table.unpack` in the middle of a table constructor keeps only its first
-- value: every account past the first would have been built, orphaned, and
-- never drawn.
local tree = { width = W, height = H }
local function place(node) tree[#tree + 1] = node end
local function place_all(nodes)
  for _, node in ipairs(nodes) do place(node) end
end

place(ui.Rect { width = W, height = H, color = INK })
place(drift(1, math.floor(W * 0.18), math.floor(H * 0.22), s(300), "#1d3a4a", s(90)))
place(drift(2, math.floor(W * 0.74), math.floor(H * 0.30), s(260), "#2a2340", s(70)))
place(drift(3, math.floor(W * 0.50), math.floor(H * 0.78), s(340), "#152b33", s(110)))

place(clock)
place(today)
place_all(avatars)

-- The card. Its border is the one thing that answers a refused password, and it
-- answers by turning red and easing back rather than by blinking.
place(ui.Rect {
  x = CARD_X,
  y = CARD_Y,
  width = CARD_W,
  height = CARD_H,
  radius = s(20),
  color = "#111823e0",
  border_width = s(1),
  border_color = function() return alarmed:get() and ALERT or LINE end,
  behavior = { border_color = { duration = 320, easing = "out_quad" } },
})
place(ui.Text {
  x = CARD_X,
  y = CARD_Y + s(26),
  width = CARD_W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or "no accounts found"
  end,
  font_size = s(22),
  font_weight = 600,
  horizontal_alignment = "center",
  color = TEXT,
})
place(ui.Text {
  x = CARD_X,
  y = CARD_Y + s(62),
  width = CARD_W,
  text = function()
    if working:get() then return "checking…" end
    return message:get()
  end,
  font_size = s(13),
  horizontal_alignment = "center",
  color = function()
    if working:get() then return ACCENT end
    return alarmed:get() and ALERT or MUTED
  end,
  behavior = { color = { duration = 320, easing = "out_quad" } },
})
place_all(dots())
place(ui.Text {
  x = CARD_X,
  y = CARD_Y + s(124),
  width = CARD_W,
  text = function() return shifted:get() and "SHIFT" or "" end,
  font_size = s(11),
  font_weight = 600,
  horizontal_alignment = "center",
  color = ACCENT,
})
place_all(pills)

place(action(W - ACTION_W - s(24), s(24), "shut down", function() power("PowerOff") end))
place(action(W - ACTION_W * 2 - s(34), s(24), "restart", function() power("Reboot") end))
place(action(W - ACTION_W * 3 - s(44), s(24), "sleep", function() power("Suspend") end))
place(action(s(24), s(24), "keyboard", function() write(keys_open, not keys_open:get()) end))

place(board)

-- One flip every few seconds; the long easings in `drift` carry the motion.
place(ui.Timer {
  interval = 6000,
  ["repeat"] = true,
  running = true,
  on_triggered = function() write(tide, tide:get() == 1 and 0 or 1) end,
})

place(ui.Timer {
  interval = 1000,
  ["repeat"] = true,
  running = true,
  on_triggered = function()
    local now = core.system_clock()
    clock.text = now:format("%H:%M")
    today.text = now:format("%A %d %B")
  end,
})

-- Keyboard focus is exclusive, so a physical keyboard still works: the panel on
-- screen is an addition, not a replacement. This sits last so it is under
-- nothing — it claims no pointer area the keys wanted, only the keys.
place(ui.MouseArea {
  width = W,
  height = H,
  on_key_pressed = function(key, modifiers, text)
    if working:get() then return end
    if key == "Return" or key == "KP_Enter" then
      attempt()
    elseif key == "BackSpace" then
      backspace()
    elseif key == "Tab" and modifiers and modifiers.control then
      if #available > 0 then write(chosen_session, chosen_session:get() % #available + 1) end
    elseif key == "Tab" then
      if #users > 0 then pick_user(chosen_user:get() % #users + 1) end
    elseif key == "Escape" then
      clear_password()
    elseif key == "F1" then
      write(keys_open, not keys_open:get())
    elseif text and text ~= "" then
      type_character(text)
    end
  end,
})

ui.Item(tree)

do
  local now = core.system_clock()
  clock.text = now:format("%H:%M")
  today.text = now:format("%A %d %B")
end
