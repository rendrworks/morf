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
local SLAB = "#7fc3dd"
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
-- The clock: one slab, with the time cut out of it.
--------------------------------------------------------------------------------

-- Six tiles close enough together that the field fuses them, so the clock is a
-- single piece of something rather than six boxes in a row — and the time is
-- subtracted from it. A glyph is an outline and the composition takes outlines,
-- so cutting a numeral out is the same arithmetic as cutting a circle out, and
-- the hole lands on the seam between two tiles as cleanly as in the middle of
-- one, because by then there are no tiles left, only the slab they made.
--
-- Twelve layers: six to build the slab, six to cut it. A field composes at most
-- sixteen, which is the reason the clock is `HHMMSS` in a block of two rather
-- than a line of six.
local TILE = s(196)
-- Negative: the tiles overlap. Butted up with a seam between them, the smooth
-- union of a grid leaves a diamond of nothing where four corners meet — the
-- seam between two is filled by the blend, but the point where two seams cross
-- is filled by neither. Overlapping removes the junction rather than blending
-- across it.
local TILE_GAP = -s(30)
local CLOCK_MORPH = 460
local CLOCK_X = math.floor(W * 0.10)
local CLOCK_W = TILE * 2 + TILE_GAP
local CLOCK_H = TILE * 3 + TILE_GAP * 2
local CLOCK_Y = math.floor((H - CLOCK_H) / 2)

local travel = morf.signal("greeter.travel", 0)
local arriving = core.system_clock():format("%H%M%S")
local shown = arriving
local holes = {}
local clock_swap

local function retime()
  local next_time = core.system_clock():format("%H%M%S")
  if next_time == arriving then return end
  arriving = next_time
  for index, hole in ipairs(holes) do
    hole.glyph_morph_to = arriving:sub(index, index)
  end
  write(travel, 1)
  clock_swap.running = true
end

local function slot_position(index)
  local column = (index - 1) % 2
  local row = math.floor((index - 1) / 2)
  return column * (TILE + TILE_GAP), row * (TILE + TILE_GAP)
end

local clock_field = {
  x = CLOCK_X,
  y = CLOCK_Y,
  width = CLOCK_W,
  height = CLOCK_H,
  fill_color = SLAB,
  -- Wider than the gap, so the tiles are already one piece at rest.
  blend = s(22),
}
for index = 1, 6 do
  local x, y = slot_position(index)
  clock_field[#clock_field + 1] = ui.SdfShape {
    x = x,
    y = y,
    width = TILE,
    height = TILE,
    shape = "rect",
    radius = math.floor(TILE * 0.30),
    operation = index == 1 and "union" or "smooth_union",
  }
end
for index = 1, 6 do
  local x, y = slot_position(index)
  local digit = shown:sub(index, index)
  holes[index] = ui.SdfShape {
    x = x + math.floor(TILE * 0.22),
    y = y + math.floor(TILE * 0.18),
    width = math.floor(TILE * 0.56),
    height = math.floor(TILE * 0.64),
    glyph = digit,
    glyph_morph_to = digit,
    morph_progress = function() return travel:get() end,
    operation = "subtract",
    behavior = { morph_progress = { duration = CLOCK_MORPH, easing = "in_out_cubic" } },
  }
  clock_field[#clock_field + 1] = holes[index]
end

--- Lands the new time and drops the progress. Once the figure shown and the
--- figure arriving are the same, walking between them changes nothing.
clock_swap = ui.Timer {
  interval = CLOCK_MORPH,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    for index, hole in ipairs(holes) do
      hole.glyph = arriving:sub(index, index)
    end
    write(travel, 0)
  end,
}

--------------------------------------------------------------------------------
-- The login, off to one side.
--------------------------------------------------------------------------------

-- Nothing here is centred on the screen. A login screen with everything stacked
-- down the middle is the shape every login screen has, and the clock is far too
-- big to sit above anything — so the slab holds the left and the login answers
-- it from the right.
local COLUMN_X = math.floor(W * 0.50)
local COLUMN_W = math.min(s(520), W - COLUMN_X - s(80))

local AVATAR = s(150)
local AVATAR_Y = math.floor(H * 0.50) - s(200)

local face = morf.signal("greeter.face", 0)
local face_from = users[1] and users[1].initial or "?"
local face_to = face_from
local initial_hole
local face_swap

local function show_user(index)
  local user = users[index]
  if not user or user.initial == face_from then return end
  face_to = user.initial
  initial_hole.glyph_morph_to = face_to
  write(face, 1)
  face_swap.running = true
end

initial_hole = ui.SdfShape {
  x = math.floor(AVATAR * 0.27),
  y = math.floor(AVATAR * 0.23),
  width = math.floor(AVATAR * 0.46),
  height = math.floor(AVATAR * 0.54),
  glyph = face_from,
  glyph_morph_to = face_from,
  morph_progress = function() return face:get() end,
  operation = "subtract",
  behavior = { morph_progress = { duration = 400, easing = "in_out_cubic" } },
}

face_swap = ui.Timer {
  interval = 400,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    face_from = face_to
    initial_hole.glyph = face_from
    write(face, 0)
  end,
}

local avatar = ui.Sdf {
  x = COLUMN_X,
  y = AVATAR_Y,
  width = AVATAR,
  height = AVATAR,
  fill_color = function() return alarmed:get() and ALERT or SLAB end,
  behavior = { fill_color = { duration = 320, easing = "out_quad" } },
  ui.SdfShape {
    width = AVATAR,
    height = AVATAR,
    shape = "circle",
    -- Squares off once there is a password being typed, so the shape itself
    -- reports that the screen is listening.
    morph_to = "rect",
    radius = math.floor(AVATAR * 0.32),
    morph_progress = function() return typed:get() > 0 and 1 or 0 end,
    behavior = {
      morph_progress = { kind = "spring", mass = 1, damping = 16, stiffness = 190,
                         epsilon = 0.001 },
    },
  },
  initial_hole,
}

-- The password as drops in one field: a typed character appears and runs into
-- the one before it, so what builds up is a single length of something rather
-- than a row of ticks. The neck between two drops is where their distances
-- agree — the same arithmetic that cuts the numerals out of the slab.
local DROP = s(17)
local DROPS = 12
local DROP_GAP = s(25)
local DROPS_Y = AVATAR_Y + AVATAR + s(126)

local drops = {
  x = COLUMN_X,
  y = DROPS_Y,
  width = (DROPS - 1) * DROP_GAP + DROP * 2,
  height = DROP * 2,
  fill_color = function() return alarmed:get() and ALERT or SLAB end,
  blend = math.floor(DROP * 0.95),
  behavior = { fill_color = { duration = 320, easing = "out_quad" } },
}
for index = 1, DROPS do
  -- An untyped slot is a small mark rather than nothing at all: the row says
  -- where the password goes, and each grows into a drop as it is filled.
  local REST = math.floor(DROP * 0.3)
  local size = function() return typed:get() >= index and DROP * 2 or REST end
  local inset = function() return typed:get() >= index and 0 or DROP - REST / 2 end
  drops[#drops + 1] = ui.SdfShape {
    x = function() return (index - 1) * DROP_GAP + inset() end,
    y = inset,
    width = size,
    height = size,
    shape = "circle",
    operation = index == 1 and "union" or "smooth_union",
    behavior = {
      x = { kind = "spring", mass = 1, damping = 14, stiffness = 300, epsilon = 0.05 },
      y = { kind = "spring", mass = 1, damping = 14, stiffness = 300, epsilon = 0.05 },
      width = { kind = "spring", mass = 1, damping = 14, stiffness = 300, epsilon = 0.05 },
      height = { kind = "spring", mass = 1, damping = 14, stiffness = 300, epsilon = 0.05 },
    },
  }
end

--------------------------------------------------------------------------------
-- Everything that is a word.
--------------------------------------------------------------------------------

--- A row that can be chosen, left-aligned like everything in this column.
--- Words rather than symbols: ⏻ is in most fonts but ⟳ and ⌨ are not, and a
--- greeter that renders a tofu box has told the person in front of it nothing.
local ROW_H = s(36)
local function row(id, label, width, lit, on_tap)
  local hot = morf.signal("greeter.hot." .. id, false)
  local function live() return hot:get() or lit() end
  return ui.Item {
    width = width,
    height = ROW_H,
    ui.Rect {
      anchors = { fill = true },
      radius = ROW_H / 2,
      color = function() return live() and ACCENT_IN or "#00000000" end,
      border_width = s(1),
      border_color = function() return live() and ACCENT or LINE end,
      behavior = {
        color = { duration = 180, easing = "out_quad" },
        border_color = { duration = 180, easing = "out_quad" },
      },
    },
    ui.Text {
      anchors = { fill = true },
      text = label,
      font_size = s(13),
      horizontal_alignment = "center",
      vertical_alignment = "center",
      elide = "right",
      color = function() return live() and TEXT or MUTED end,
      behavior = { color = { duration = 180, easing = "out_quad" } },
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
-- would sit over every row and swallow the taps meant for them. Key handlers
-- are collected by walking the tree rather than by z-order, so being underneath
-- costs it nothing.
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
      if #users > 0 then
        local next_user = chosen_user:get() % #users + 1
        pick_user(next_user)
        show_user(next_user)
      end
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
place(drift(1, math.floor(W * 0.16), math.floor(H * 0.62), s(420), "#15364a", s(130)))
place(drift(2, math.floor(W * 0.80), math.floor(H * 0.24), s(320), "#241f3d", s(100)))

place(ui.Sdf(clock_field))
place(ui.Text {
  x = CLOCK_X,
  y = CLOCK_Y + CLOCK_H + s(26),
  width = CLOCK_W,
  text = core.system_clock():format("%A"),
  font_size = s(19),
  font_weight = 500,
  color = TEXT,
})
local today = ui.Text {
  x = CLOCK_X,
  y = CLOCK_Y + CLOCK_H + s(54),
  width = CLOCK_W,
  text = core.system_clock():format("%d %B"),
  font_size = s(15),
  color = MUTED,
}
place(today)

place(avatar)
place(ui.Text {
  x = COLUMN_X,
  y = AVATAR_Y + AVATAR + s(30),
  width = COLUMN_W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or "no accounts found"
  end,
  font_size = s(30),
  font_weight = 600,
  color = TEXT,
})
place(ui.Text {
  x = COLUMN_X,
  y = AVATAR_Y + AVATAR + s(74),
  width = COLUMN_W,
  text = function()
    if working:get() then return "checking…" end
    return message:get()
  end,
  font_size = s(14),
  color = function()
    if working:get() then return ACCENT end
    return alarmed:get() and ALERT or MUTED
  end,
  behavior = { color = { duration = 320, easing = "out_quad" } },
})
place(ui.Sdf(drops))

-- The choices, stacked under the column rather than centred beneath everything.
local choice_y = DROPS_Y + DROP * 2 + s(46)
for index, user in ipairs(users) do
  local node = row("user" .. index, user.label, s(168),
    function() return chosen_user:get() == index end,
    function()
      pick_user(index)
      show_user(index)
    end)
  node.x = COLUMN_X + (index - 1) * (s(168) + s(10))
  node.y = choice_y
  place(node)
end
for index, entry in ipairs(available) do
  local node = row("session" .. index, entry.name, s(200),
    function() return chosen_session:get() == index end,
    function()
      if not working:get() then write(chosen_session, index) end
    end)
  node.x = COLUMN_X + (index - 1) * (s(200) + s(10))
  node.y = choice_y + ROW_H + s(12)
  place(node)
end

local ACTION_W = s(98)
for index, action in ipairs({
  { "shut down", function() power("PowerOff") end },
  { "restart", function() power("Reboot") end },
  { "sleep", function() power("Suspend") end },
}) do
  local node = row(action[1], action[1], ACTION_W, function() return false end, action[2])
  node.x = W - (ACTION_W + s(12)) * index - s(12)
  node.y = H - ROW_H - s(28)
  place(node)
end
do
  local node = row("keyboard", "keyboard", ACTION_W, function() return false end, open_keyboard)
  node.x = s(28)
  node.y = H - ROW_H - s(28)
  place(node)
end

place(ui.Timer {
  interval = 7000, ["repeat"] = true, running = true,
  on_triggered = function() write(tide, tide:get() == 1 and 0 or 1) end,
})
place(ui.Timer {
  interval = 500, ["repeat"] = true, running = true,
  on_triggered = function()
    retime()
    today.text = core.system_clock():format("%d %B")
  end,
})
place(clock_swap)
place(face_swap)

ui.Item(tree)
