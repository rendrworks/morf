-- A login screen.
--
-- Not a display manager. `greetd` owns authentication and starts sessions; a
-- greeter is an ordinary Wayland client it runs, and this is one. greetd draws
-- nothing itself — it launches a compositor and the greeter inside it, so morf
-- needs no DRM, no root and no rendering it does not already do. The same
-- binary, in a different host.
--
--     systemd
--      └─ greetd.service         root, no graphics, owns the VT
--          ├─ cage               a Wayland compositor, for the login only
--          │   └─ morf greeter.lua
--          │        └── $GREETD_SOCK ──► greetd
--          └─ on success: kills cage, starts the session
--
-- Try it nested first, inside a session you are already logged into:
--
--     cage -- morf examples/greeter.lua
--
-- A greeter you can only test by logging out is a greeter you will eventually
-- be locked out by. Then, when it behaves, `/etc/greetd/config.toml`:
--
--     [terminal]
--     vt = 1
--
--     [default_session]
--     command = "cage -- morf /etc/morf/greeter.lua"
--     user = "greeter"
--
-- Two things that bite. It runs as user `greeter`, not as you — every font,
-- asset and path here must be readable by that user, which is the usual way
-- these fail. And `GREETD_SOCK` exists only in the environment greetd provides,
-- so run it nested and the password field will say so rather than pretending.
--
-- `cage` has no layer shell, so morf stands the surface up as a fullscreen
-- toplevel instead. Nothing below has to know that.
--
-- On the motion: nothing here animates from Lua. Every transition is a
-- `behavior` on a property — Lua writes a target once and morf's frame tick
-- carries it there. The clock is the exception worth looking at: its digits do
-- not cut from one number to the next, they *morph*, because a glyph is a
-- distance field and two of them can be interpolated. `9` becomes `0` through
-- shapes the font does not contain.

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
-- The greeter is the only thing on screen and the password has to go somewhere.
morf.surface.keyboard_focus = "exclusive"

-- Sized in proportion to the screen: a greeter is read from a metre away, and a
-- login screen laid out in fixed pixels is either a postage stamp on a 4K panel
-- or off the edge of a small one.
local SCALE = math.max(0.75, math.min(1.7, math.min(W / 1920, H / 1080)))
local function s(n) return math.floor(n * SCALE) end

local INK = "#05070c"
local PANEL = "#121a27cc"
local RAISED = "#1b2536"
local LINE = "#2b3648"
local TEXT = "#eef2f8"
local MUTED = "#7d8a9f"
local ACCENT = "#79b8d1"
local ACCENT_IN = "#12303d"
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
        found[#found + 1] = { name = name, label = label, initial = label:sub(1, 1):upper() }
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

local revealed = morf.signal("greeter.revealed", false)
local chosen_user = morf.signal("greeter.user", 1)
local chosen_session = morf.signal("greeter.session", 1)
local typed = morf.signal("greeter.typed", 0)
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

--- Lifts the shade. Any key or any touch does it, the way a lock screen works:
--- the clock is what it shows at rest, and the login is what it shows once
--- somebody is there.
local function reveal()
  if not revealed:get() then write(revealed, true) end
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

--- Starts the on-screen keyboard, once.
---
--- Its own process, and deliberately: it types through the compositor with the
--- virtual-keyboard protocol, so it needs no agreement with this screen and
--- this screen needs no code to receive it. The keys arrive here exactly as a
--- physical keyboard's would. It also means a machine with a keyboard never
--- pays for one it does not use.
local keyboard_running = false
local function open_keyboard()
  if keyboard_running then return end
  local ok, path = pcall(core.shell_path, "keyboard.lua")
  if not ok then
    say("no keyboard beside this configuration", true)
    return
  end
  local started = pcall(morf.exec_detached, { "morf", path })
  if started then
    keyboard_running = true
  else
    say("could not start the keyboard", true)
  end
end

--------------------------------------------------------------------------------
-- Typing.
--------------------------------------------------------------------------------

local CAPACITY = 64

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
-- The clock.
--------------------------------------------------------------------------------

-- Five slots: `HH:MM`. Each is its own node, so a minute rolling over moves the
-- two digits that changed and leaves the rest alone — and each of those two
-- *morphs*, because a glyph is a distance field and two of them interpolate.
-- `9` becomes `0` through outlines the font does not contain, rather than one
-- number cutting to another.
--
-- The colon is a slot like any other. It never changes, so its two glyphs are
-- identical and the interpolation between them is the identity: it is left
-- perfectly alone without having to be a special case.
local CLOCK_MORPH = 420
-- Narrower than the glyphs' own advance, so the numerals sit as a clock
-- rather than as five separate letters. A digit is centred in its slot, so
-- tightening the slot tightens the spacing without moving anything off centre.
local DIGIT_W = s(64)
local COLON_W = s(28)

local travel = morf.signal("greeter.travel", 0)
local shown = "     "
local arriving = "     "
local digits = {}
local clock_swap

local function clock_text()
  return core.system_clock():format("%H:%M")
end

--- Puts a new time on screen, morphing whichever slots differ.
local function retime()
  local next_time = clock_text()
  if next_time == arriving then return end
  arriving = next_time
  for index, node in ipairs(digits) do
    node.morph_to = arriving:sub(index, index)
  end
  write(travel, 1)
  clock_swap.running = true
end

local slot_x = math.floor((W - (DIGIT_W * 4 + COLON_W)) / 2)
for index = 1, 5 do
  local width = index == 3 and COLON_W or DIGIT_W
  digits[index] = ui.Text {
    x = slot_x,
    width = width,
    text = " ",
    morph_to = " ",
    morph_progress = function() return travel:get() end,
    font_size = s(112),
    font_weight = 300,
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = TEXT,
    behavior = {
      morph_progress = { duration = CLOCK_MORPH, easing = "in_out_cubic" },
    },
  }
  slot_x = slot_x + width
end

local today = ui.Text {
  width = W,
  text = "",
  font_size = s(17),
  horizontal_alignment = "center",
  color = MUTED,
}

--- Lands the new time and drops the progress.
---
--- Once `text` and `morph_to` name the same glyph the interpolation between
--- them is the identity, so the progress can go back to zero without anything
--- moving — no second animation, and nothing to see.
clock_swap = ui.Timer {
  interval = CLOCK_MORPH,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    shown = arriving
    for index, node in ipairs(digits) do
      node.text = shown:sub(index, index)
    end
    write(travel, 0)
  end,
}

--------------------------------------------------------------------------------
-- The background.
--------------------------------------------------------------------------------

-- Two soft fields leaning one way and then the other. One timer flips `tide`
-- every few seconds and the long easings do the rest, so the drift costs Lua
-- one call per flip and nothing per frame. `softness` is what makes these
-- genuinely soft shapes rather than blurred pictures of shapes — a field has no
-- resolution, so the glow costs the same at any size.
local function drift(index, home_x, home_y, radius, colour, reach)
  return ui.Sdf {
    x = function() return home_x + (tide:get() == 1 and reach or -reach) end,
    y = function() return home_y + (tide:get() == 1 and -reach or reach) end,
    width = radius * 2,
    height = radius * 2,
    fill_color = colour,
    opacity = 0.55,
    softness = radius * 0.85,
    behavior = {
      x = { duration = 8000 + index * 1100, easing = "in_out_sine" },
      y = { duration = 9400 - index * 800, easing = "in_out_sine" },
    },
    ui.SdfShape { width = radius * 2, height = radius * 2, shape = "circle" },
  }
end

--------------------------------------------------------------------------------
-- Accounts and sessions.
--------------------------------------------------------------------------------

local TILE = s(96)
local TILE_GAP = s(26)

local function avatar(index, user)
  local function mine() return chosen_user:get() == index end
  return ui.Item {
    width = TILE,
    height = TILE + s(34),
    ui.Rect {
      width = TILE,
      height = TILE,
      radius = TILE / 2,
      color = function() return mine() and ACCENT_IN or RAISED end,
      border_width = s(2),
      border_color = function() return mine() and ACCENT or LINE end,
      -- A spring rather than a duration: a tap should feel answered, and the
      -- small overshoot is what reads as an answer.
      scale = function() return mine() and 1.07 or 1.0 end,
      behavior = {
        color = { duration = 220, easing = "out_quad" },
        border_color = { duration = 220, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 13, stiffness = 250, epsilon = 0.001 },
      },
    },
    ui.Text {
      width = TILE,
      height = TILE,
      text = user.initial,
      font_size = s(32),
      font_weight = 600,
      horizontal_alignment = "center",
      vertical_alignment = "center",
      color = function() return mine() and TEXT or MUTED end,
      behavior = { color = { duration = 220, easing = "out_quad" } },
    },
    ui.Text {
      y = TILE + s(9),
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
      height = TILE + s(34),
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

--- A labelled button, for the things that are not logging in.
---
--- Words rather than symbols: ⏻ is in most fonts but ⟳ and ⌨ are not, and a
--- greeter that renders a tofu box has told the person in front of it nothing.
--- A greeter is also the one screen where guessing wrong is expensive.
local ACTION_W = s(98)
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

--------------------------------------------------------------------------------
-- The card, and the shade over it.
--------------------------------------------------------------------------------

local CARD_W = math.min(s(520), W - s(80))
local CARD_H = s(128)
local CARD_X = math.floor((W - CARD_W) / 2)
local CARD_Y = math.floor(H * 0.42)

local DOTS = 16
local DOT = s(10)
local DOT_GAP = s(9)

local function dots()
  local nodes = {}
  local span = DOTS * DOT + (DOTS - 1) * DOT_GAP
  local left = math.floor((CARD_W - span) / 2)
  for index = 1, DOTS do
    nodes[#nodes + 1] = ui.Rect {
      x = left + (index - 1) * (DOT + DOT_GAP),
      y = s(86),
      width = DOT,
      height = DOT,
      radius = DOT / 2,
      color = function()
        if alarmed:get() then return ALERT end
        return typed:get() >= index and ACCENT or LINE
      end,
      scale = function() return typed:get() >= index and 1.0 or 0.5 end,
      behavior = {
        color = { duration = 160, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 12, stiffness = 330, epsilon = 0.001 },
      },
    }
  end
  return nodes
end

-- Everything below the clock lives in one item, so the shade is a single
-- animated number rather than a dozen. It arrives from underneath on a spring,
-- which is the difference between a screen appearing and a screen being handed
-- to you.
local card_parts = {
  x = CARD_X,
  y = function() return revealed:get() and CARD_Y or CARD_Y + s(60) end,
  width = CARD_W,
  height = CARD_H,
  opacity = function() return revealed:get() and 1 or 0 end,
  behavior = {
    y = { kind = "spring", mass = 1, damping = 19, stiffness = 170, epsilon = 0.5 },
    opacity = { duration = 260, easing = "out_quad" },
  },
  ui.Rect {
    anchors = { fill = true },
    radius = s(20),
    color = PANEL,
    border_width = s(1),
    -- The border is the one thing that answers a refused password, and it
    -- answers by colouring and easing back rather than by blinking.
    border_color = function() return alarmed:get() and ALERT or LINE end,
    behavior = { border_color = { duration = 320, easing = "out_quad" } },
  },
  ui.Text {
    y = s(24),
    width = CARD_W,
    text = function()
      local user = users[chosen_user:get()]
      return user and user.label or "no accounts found"
    end,
    font_size = s(21),
    font_weight = 600,
    horizontal_alignment = "center",
    color = TEXT,
  },
  ui.Text {
    y = s(56),
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
  },
  -- The whole card is the way to ask for a keyboard. On a machine with no
  -- keys, touching the thing you are being asked to type into is the gesture
  -- that has to work.
  ui.MouseArea {
    anchors = { fill = true },
    on_clicked = open_keyboard,
  },
}
for _, dot in ipairs(dots()) do
  card_parts[#card_parts + 1] = dot
end
local card = ui.Item(card_parts)

--------------------------------------------------------------------------------
-- The tree.
--------------------------------------------------------------------------------

-- Children go into a list rather than inline: a `table.unpack` in the middle of
-- a table constructor keeps only its first value, so every account past the
-- first would be built, orphaned, and never drawn.
local tree = { width = W, height = H }
local function place(node) tree[#tree + 1] = node end

-- First, so it is at the bottom of the stack. A hit test returns the *topmost*
-- MouseArea over the point, and this one covers the screen: placed last it
-- would sit over the card and the account tiles and swallow every tap on them.
-- Key handlers are collected by walking the tree rather than by z-order, so
-- sitting underneath costs it nothing.
place(ui.MouseArea {
  width = W,
  height = H,
  on_clicked = reveal,
  on_key_pressed = function(key, modifiers, text)
    reveal()
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
      open_keyboard()
    elseif text and text ~= "" then
      type_character(text)
    end
  end,
})

place(ui.Rect { width = W, height = H, color = INK })
place(drift(1, math.floor(W * 0.20), math.floor(H * 0.26), s(320), "#17384a", s(100)))
place(drift(2, math.floor(W * 0.76), math.floor(H * 0.70), s(280), "#26203f", s(80)))

-- The clock lifts and shrinks as the shade goes up, which is the whole of the
-- transition: at rest it is the screen, and once somebody is there it is a
-- heading over the thing they came to use.
local clock_parts = {
  x = 0,
  y = function() return revealed:get() and math.floor(H * 0.12) or math.floor(H * 0.34) end,
  width = W,
  height = s(150),
  scale = function() return revealed:get() and 0.72 or 1.0 end,
  behavior = {
    y = { kind = "spring", mass = 1, damping = 21, stiffness = 150, epsilon = 0.5 },
    scale = { kind = "spring", mass = 1, damping = 21, stiffness = 150, epsilon = 0.001 },
  },
}
for _, digit in ipairs(digits) do
  clock_parts[#clock_parts + 1] = digit
end
place(ui.Item(clock_parts))

today.y = 0
place(ui.Item {
  x = 0,
  y = function() return revealed:get() and math.floor(H * 0.12) + s(126) or math.floor(H * 0.34) + s(160) end,
  width = W,
  height = s(30),
  behavior = { y = { kind = "spring", mass = 1, damping = 21, stiffness = 150, epsilon = 0.5 } },
  today,
})

-- The hint is only true while the shade is down, and says so by leaving.
place(ui.Text {
  y = math.floor(H * 0.34) + s(210),
  width = W,
  text = "press any key",
  font_size = s(14),
  horizontal_alignment = "center",
  color = MUTED,
  opacity = function() return revealed:get() and 0 or 1 end,
  behavior = { opacity = { duration = 240, easing = "out_quad" } },
})

-- Everything the shade covers goes in one item, so lifting it is a single
-- animated number rather than one per tile, pill and panel. The card keeps its
-- own spring inside this, which is what gives the arrival its slight lag behind
-- the fade.
local login = { x = 0, y = 0, width = W, height = H,
  opacity = function() return revealed:get() and 1 or 0 end,
  behavior = { opacity = { duration = 280, easing = "out_quad" } },
}

local strip_span = #users * TILE + math.max(0, #users - 1) * TILE_GAP
local strip_x = math.floor((W - strip_span) / 2)
for index, user in ipairs(users) do
  local tile = avatar(index, user)
  tile.x = strip_x + (index - 1) * (TILE + TILE_GAP)
  tile.y = math.floor(H * 0.27)
  login[#login + 1] = tile
end

login[#login + 1] = card

local PILL_W = s(190)
local pill_span = #available * PILL_W + math.max(0, #available - 1) * s(12)
local pill_x = math.floor((W - pill_span) / 2)
for index, entry in ipairs(available) do
  local node = pill(index, entry, PILL_W)
  node.x = pill_x + (index - 1) * (PILL_W + s(12))
  node.y = CARD_Y + CARD_H + s(22)
  login[#login + 1] = node
end

place(ui.Item(login))

place(action(W - ACTION_W - s(24), s(24), "shut down", function() power("PowerOff") end))
place(action(W - ACTION_W * 2 - s(34), s(24), "restart", function() power("Reboot") end))
place(action(W - ACTION_W * 3 - s(44), s(24), "sleep", function() power("Suspend") end))
place(action(s(24), s(24), "keyboard", open_keyboard))

-- One flip every few seconds; the long easings in `drift` carry the motion.
place(ui.Timer {
  interval = 7000,
  ["repeat"] = true,
  running = true,
  on_triggered = function() write(tide, tide:get() == 1 and 0 or 1) end,
})

place(ui.Timer {
  interval = 1000,
  ["repeat"] = true,
  running = true,
  on_triggered = function()
    retime()
    today.text = core.system_clock():format("%A %d %B")
  end,
})

place(clock_swap)

ui.Item(tree)

-- The first time is put up without a morph: there is nothing to travel from.
shown = clock_text()
arriving = shown
for index, node in ipairs(digits) do
  node.text = shown:sub(index, index)
  node.morph_to = shown:sub(index, index)
end
today.text = core.system_clock():format("%A %d %B")
