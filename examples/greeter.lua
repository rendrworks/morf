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

-- `HH:MM:SS`, one node per character. A second rolling over moves the one or
-- two slots that changed and leaves the rest alone — and each of those *morphs*,
-- because a glyph is a distance field and two of them interpolate: `5` becomes
-- `6` through outlines the font does not contain.
--
-- The colons need no special case. They never change, so their two glyphs are
-- identical and the interpolation between them is the identity.
local CLOCK_FORMAT = "%H:%M:%S"
local CLOCK_MORPH = 340
local DIGIT_W = s(34)
local COLON_W = s(15)

local travel = morf.signal("greeter.travel", 0)
local arriving = ""
local digits = {}
local clock_swap

local function clock_text()
  return core.system_clock():format(CLOCK_FORMAT)
end

--- Puts a new time up, morphing whichever slots differ.
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

local shape = clock_text()
local clock_w = 0
for index = 1, #shape do
  clock_w = clock_w + (shape:sub(index, index) == ":" and COLON_W or DIGIT_W)
end

local slot_x = 0
for index = 1, #shape do
  local width = shape:sub(index, index) == ":" and COLON_W or DIGIT_W
  digits[index] = ui.Text {
    x = slot_x,
    width = width,
    height = s(72),
    text = shape:sub(index, index),
    morph_to = shape:sub(index, index),
    morph_progress = function() return travel:get() end,
    font_size = s(56),
    font_weight = 250,
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = TEXT,
    behavior = { morph_progress = { duration = CLOCK_MORPH, easing = "in_out_cubic" } },
  }
  slot_x = slot_x + width
end
arriving = shape

--- Lands the new time and drops the progress.
---
--- Once `text` and `morph_to` name the same glyph the interpolation between
--- them is the identity, so the progress goes back to zero without anything
--- moving — no second animation, and nothing to see.
clock_swap = ui.Timer {
  interval = CLOCK_MORPH,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    for index, node in ipairs(digits) do
      node.text = arriving:sub(index, index)
    end
    write(travel, 0)
  end,
}

local today = ui.Text {
  width = W,
  text = core.system_clock():format("%A %d %B"),
  font_size = s(14),
  horizontal_alignment = "center",
  color = MUTED,
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
    opacity = 0.5,
    softness = radius * 0.85,
    behavior = {
      x = { duration = 8000 + index * 1100, easing = "in_out_sine" },
      y = { duration = 9400 - index * 800, easing = "in_out_sine" },
    },
    ui.SdfShape { width = radius * 2, height = radius * 2, shape = "circle" },
  }
end

--------------------------------------------------------------------------------
-- The person, and the field they type into.
--------------------------------------------------------------------------------

-- One stack, close together and centred. A login screen is a single question
-- with a single answer, and spreading its parts down the screen makes them look
-- like separate things to deal with.
local AVATAR = s(108)
local FIELD_W = math.min(s(360), W - s(80))
local FIELD_H = s(50)
local FIELD_X = math.floor((W - FIELD_W) / 2)

local STACK_TOP = math.floor(H * 0.30)
local NAME_Y = STACK_TOP + AVATAR + s(22)
local FIELD_Y = NAME_Y + s(52)
local MESSAGE_Y = FIELD_Y + FIELD_H + s(14)
local SESSION_Y = MESSAGE_Y + s(30)

local blink = morf.signal("greeter.blink", true)

local DOT = s(9)
local DOT_GAP = s(9)
local DOT_LEFT = s(22)

-- The dots and the caret sit inside the field, laid out from its left edge the
-- way typed text does, so it reads as somewhere to type rather than as a
-- progress bar. A dot appears where the character went.
local function field_dots()
  local nodes = {}
  for index = 1, 24 do
    nodes[#nodes + 1] = ui.Rect {
      x = DOT_LEFT + (index - 1) * (DOT + DOT_GAP),
      y = math.floor((FIELD_H - DOT) / 2),
      width = DOT,
      height = DOT,
      radius = DOT / 2,
      color = function() return alarmed:get() and ALERT or TEXT end,
      opacity = function() return typed:get() >= index and 1 or 0 end,
      scale = function() return typed:get() >= index and 1.0 or 0.4 end,
      behavior = {
        opacity = { duration = 120, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 12, stiffness = 340, epsilon = 0.001 },
        color = { duration = 200, easing = "out_quad" },
      },
    }
  end
  return nodes
end

local field_parts = {
  x = FIELD_X,
  y = FIELD_Y,
  width = FIELD_W,
  height = FIELD_H,
  ui.Rect {
    anchors = { fill = true },
    radius = s(12),
    color = RAISED,
    border_width = s(2),
    -- Always lit. This screen has exactly one thing to type into and nothing
    -- else can take the keyboard, so a field that looked unfocused would be
    -- lying about what a keypress will do.
    border_color = function() return alarmed:get() and ALERT or ACCENT end,
    behavior = { border_color = { duration = 300, easing = "out_quad" } },
  },
  -- The placeholder leaves as soon as there is anything to show.
  ui.Text {
    x = DOT_LEFT,
    width = FIELD_W - DOT_LEFT * 2,
    height = FIELD_H,
    text = "Password",
    font_size = s(15),
    vertical_alignment = "center",
    color = MUTED,
    opacity = function() return typed:get() == 0 and 1 or 0 end,
    behavior = { opacity = { duration = 140, easing = "out_quad" } },
  },
  -- The caret. It sits after the last dot, so it moves as you type.
  ui.Rect {
    x = function() return DOT_LEFT + typed:get() * (DOT + DOT_GAP) end,
    y = math.floor(FIELD_H * 0.26),
    width = s(2),
    height = math.floor(FIELD_H * 0.48),
    color = ACCENT,
    opacity = function() return blink:get() and 1 or 0 end,
    behavior = {
      x = { kind = "spring", mass = 1, damping = 15, stiffness = 420, epsilon = 0.1 },
      opacity = { duration = 90, easing = "out_quad" },
    },
  },
  ui.MouseArea { anchors = { fill = true }, on_clicked = open_keyboard },
}
for _, dot in ipairs(field_dots()) do
  field_parts[#field_parts + 1] = dot
end

--- One account, as a row that can be picked. The chosen one is shown large
--- above the field; the rest sit at the foot of the screen, which is where a
--- login screen puts the people who are not logging in.
local function other(index, user, x, y)
  local hot = morf.signal("greeter.other." .. user.name, false)
  return ui.Item {
    x = x, y = y, width = s(150), height = s(38),
    ui.Rect {
      anchors = { fill = true },
      radius = s(19),
      color = function() return hot:get() and RAISED or "#00000000" end,
      behavior = { color = { duration = 160, easing = "out_quad" } },
    },
    ui.Text {
      anchors = { fill = true },
      text = user.label,
      font_size = s(14),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      elide = "right",
      color = function() return hot:get() and TEXT or MUTED end,
      behavior = { color = { duration = 160, easing = "out_quad" } },
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = function() pick_user(index) end,
    },
  }
end

--------------------------------------------------------------------------------
-- Sessions and the machine's own controls.
--------------------------------------------------------------------------------

local function pill(index, entry, width)
  local function mine() return chosen_session:get() == index end
  return ui.Item {
    width = width,
    height = s(32),
    ui.Rect {
      anchors = { fill = true },
      radius = s(16),
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
      font_size = s(12),
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

--- Words rather than symbols: ⏻ is in most fonts but ⟳ and ⌨ are not, and a
--- greeter that renders a tofu box has told the person in front of it nothing.
--- A greeter is also the one screen where guessing wrong is expensive.
local ACTION_W = s(96)
local function action(x, y, label, on_tap)
  local hot = morf.signal("greeter.action." .. label, false)
  return ui.Item {
    x = x, y = y, width = ACTION_W, height = s(32),
    ui.Rect {
      anchors = { fill = true },
      radius = s(16),
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
      font_size = s(12),
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
-- The tree.
--------------------------------------------------------------------------------

-- Children go into a list rather than inline: a `table.unpack` in the middle of
-- a table constructor keeps only its first value, so every account past the
-- first would be built, orphaned, and never drawn.
local tree = { width = W, height = H }
local function place(node) tree[#tree + 1] = node end

-- First, so it is at the bottom of the stack. A hit test returns the *topmost*
-- MouseArea over the point, and this one covers the screen: placed last it
-- would sit over the field and the account rows and swallow every tap on them.
-- Key handlers are collected by walking the tree rather than by z-order, so
-- sitting underneath costs it nothing.
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
      open_keyboard()
    elseif text and text ~= "" then
      type_character(text)
    end
  end,
})

place(ui.Rect { width = W, height = H, color = INK })
place(drift(1, math.floor(W * 0.22), math.floor(H * 0.24), s(340), "#16374a", s(110)))
place(drift(2, math.floor(W * 0.74), math.floor(H * 0.74), s(300), "#241f3d", s(90)))

-- The clock, at the top and out of the way. It is not the point of this screen
-- — the field is — so it is a heading rather than the thing being looked at.
local clock_parts = {
  x = math.floor((W - clock_w) / 2),
  y = math.floor(H * 0.09),
  width = clock_w,
  height = s(72),
}
for _, digit in ipairs(digits) do
  clock_parts[#clock_parts + 1] = digit
end
place(ui.Item(clock_parts))

today.y = math.floor(H * 0.09) + s(72)
place(today)

-- The person logging in: their initial, large, over their name and the field.
place(ui.Item {
  x = math.floor((W - AVATAR) / 2),
  y = STACK_TOP,
  width = AVATAR,
  height = AVATAR,
  ui.Rect {
    anchors = { fill = true },
    radius = AVATAR / 2,
    color = ACCENT_IN,
    border_width = s(2),
    border_color = ACCENT,
  },
  ui.Text {
    anchors = { fill = true },
    text = function()
      local user = users[chosen_user:get()]
      return user and user.initial or "?"
    end,
    font_size = s(40),
    font_weight = 600,
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = TEXT,
  },
})

place(ui.Text {
  y = NAME_Y,
  width = W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or "no accounts found"
  end,
  font_size = s(22),
  font_weight = 500,
  horizontal_alignment = "center",
  color = TEXT,
})

place(ui.Item(field_parts))

place(ui.Text {
  y = MESSAGE_Y,
  width = W,
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

local PILL_W = s(180)
local pill_span = #available * PILL_W + math.max(0, #available - 1) * s(10)
local pill_x = math.floor((W - pill_span) / 2)
for index, entry in ipairs(available) do
  local node = pill(index, entry, PILL_W)
  node.x = pill_x + (index - 1) * (PILL_W + s(10))
  node.y = SESSION_Y
  place(node)
end

-- Everyone who is not the person above, along the foot of the screen.
if #users > 1 then
  local row_span = #users * s(150) + (#users - 1) * s(8)
  local row_x = math.floor((W - row_span) / 2)
  for index, user in ipairs(users) do
    place(other(index, user, row_x + (index - 1) * (s(150) + s(8)), H - s(76)))
  end
end

place(action(W - ACTION_W - s(24), s(24), "shut down", function() power("PowerOff") end))
place(action(W - ACTION_W * 2 - s(34), s(24), "restart", function() power("Reboot") end))
place(action(W - ACTION_W * 3 - s(44), s(24), "sleep", function() power("Suspend") end))
place(action(s(24), s(24), "keyboard", open_keyboard))

place(ui.Timer {
  interval = 7000, ["repeat"] = true, running = true,
  on_triggered = function() write(tide, tide:get() == 1 and 0 or 1) end,
})

-- Twice a second, so a second never lands more than half a second late.
place(ui.Timer {
  interval = 500, ["repeat"] = true, running = true,
  on_triggered = function()
    retime()
    today.text = core.system_clock():format("%A %d %B")
  end,
})

place(ui.Timer {
  interval = 560, ["repeat"] = true, running = true,
  on_triggered = function() write(blink, not blink:get()) end,
})

place(clock_swap)

ui.Item(tree)
