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
local PANEL_SOLID = "#1a2534"
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
-- The clock, as six tiles with the time cut out of them.
--------------------------------------------------------------------------------

-- Each digit is a hole in a shape rather than a letter drawn on top of one.
-- A glyph is an outline, and the composition takes outlines, so subtracting one
-- from a rounded square is the same arithmetic as subtracting a circle: solved
-- at the edge, per pixel, at whatever size the tile happens to be.
--
-- On a tick, the tiles that changed morph. Both halves move at once and from
-- the same number — the tile breathes from squircle towards circle and back,
-- and the figure inside it walks to the next figure — because there is only one
-- shape here, and the hole is part of it.
local TILE = s(104)
local TILE_GAP = s(10)
local GROUP_GAP = s(30)
local CLOCK_MORPH = 420

local travel = morf.signal("greeter.travel", 0)
local arriving = "000000"
local tiles = {}
local clock_swap

local function clock_digits()
  return core.system_clock():format("%H%M%S")
end

--- Puts a new time up, morphing whichever tiles differ.
local function retime()
  local next_time = clock_digits()
  if next_time == arriving then return end
  arriving = next_time
  for index, tile in ipairs(tiles) do
    tile.glyph_morph_to = arriving:sub(index, index)
  end
  write(travel, 1)
  clock_swap.running = true
end

local shown = clock_digits()
arriving = shown

local clock_w = TILE * 6 + TILE_GAP * 3 + GROUP_GAP * 2
local clock_x = math.floor((W - clock_w) / 2)
local clock_y = math.floor(H * 0.12)

local clock_nodes = {}
do
  local x = clock_x
  for index = 1, 6 do
    local hole = ui.SdfShape {
      x = math.floor(TILE * 0.18),
      y = math.floor(TILE * 0.16),
      width = math.floor(TILE * 0.64),
      height = math.floor(TILE * 0.68),
      glyph = shown:sub(index, index),
      glyph_morph_to = shown:sub(index, index),
      morph_progress = function() return travel:get() end,
      operation = "subtract",
      behavior = { morph_progress = { duration = CLOCK_MORPH, easing = "in_out_cubic" } },
    }
    tiles[index] = hole
    clock_nodes[#clock_nodes + 1] = ui.Sdf {
      x = x,
      y = clock_y,
      width = TILE,
      height = TILE,
      fill_color = PANEL_SOLID,
      ui.SdfShape {
        width = TILE,
        height = TILE,
        shape = "rect",
        radius = math.floor(TILE * 0.30),
        -- The tile eases towards a circle at the middle of the change and back,
        -- so the whole thing moves rather than only the number in it.
        morph_to = "circle",
        morph_progress = function() return 0.5 - math.abs(travel:get() - 0.5) end,
        behavior = { morph_progress = { duration = CLOCK_MORPH, easing = "in_out_cubic" } },
      },
      hole,
    }
    x = x + TILE + (index % 2 == 0 and GROUP_GAP or TILE_GAP)
  end
end

--- Lands the new time and drops the progress.
---
--- Once the figure shown and the figure arriving are the same, walking between
--- them changes nothing, so the progress returns to zero with nothing to see.
clock_swap = ui.Timer {
  interval = CLOCK_MORPH,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    for index, tile in ipairs(tiles) do
      tile.glyph = arriving:sub(index, index)
    end
    write(travel, 0)
  end,
}

local today = ui.Text {
  y = clock_y + TILE + s(22),
  width = W,
  text = core.system_clock():format("%A %d %B"),
  font_size = s(15),
  horizontal_alignment = "center",
  color = MUTED,
}

--------------------------------------------------------------------------------
-- The person, as their initial cut out of a shape.
--------------------------------------------------------------------------------

-- The same idea as the clock, and the reason the avatar is a field rather than
-- a circle with a letter on it: choosing another account walks the initial to
-- the new one while the shape itself eases, and neither has to be kept in step
-- with the other because they are one shape.
local AVATAR = s(132)
local AVATAR_X = math.floor((W - AVATAR) / 2)
local AVATAR_Y = math.floor(H * 0.38)

local face = morf.signal("greeter.face", 0)
local face_from = users[1] and users[1].initial or "?"
local face_to = face_from
local initial_hole
local face_swap

local function show_user(index)
  local user = users[index]
  if not user then return end
  face_to = user.initial
  if face_to == face_from then return end
  initial_hole.glyph_morph_to = face_to
  write(face, 1)
  face_swap.running = true
end

initial_hole = ui.SdfShape {
  x = math.floor(AVATAR * 0.26),
  y = math.floor(AVATAR * 0.24),
  width = math.floor(AVATAR * 0.48),
  height = math.floor(AVATAR * 0.52),
  glyph = face_from,
  glyph_morph_to = face_from,
  morph_progress = function() return face:get() end,
  operation = "subtract",
  behavior = { morph_progress = { duration = 380, easing = "in_out_cubic" } },
}

face_swap = ui.Timer {
  interval = 380,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    face_from = face_to
    initial_hole.glyph = face_from
    write(face, 0)
  end,
}

local avatar = ui.Sdf {
  x = AVATAR_X,
  y = AVATAR_Y,
  width = AVATAR,
  height = AVATAR,
  fill_color = function() return alarmed:get() and ALERT or ACCENT end,
  behavior = { fill_color = { duration = 320, easing = "out_quad" } },
  ui.SdfShape {
    width = AVATAR,
    height = AVATAR,
    shape = "circle",
    -- Rounds off towards a squircle once there is a password being typed, so
    -- the shape itself reports that the screen is listening.
    morph_to = "rect",
    radius = math.floor(AVATAR * 0.34),
    morph_progress = function() return typed:get() > 0 and 1 or 0 end,
    behavior = {
      morph_progress = { kind = "spring", mass = 1, damping = 16, stiffness = 190,
                         epsilon = 0.001 },
    },
  },
  initial_hole,
}

--------------------------------------------------------------------------------
-- The password, as drops that run together.
--------------------------------------------------------------------------------

-- One field with a seam radius, so a typed character is a drop that appears and
-- merges with the one before it. Nothing here is a picture of merging: the
-- neck between two drops is where their distances agree, computed at the edge,
-- which is the same reason the numerals cut clean holes.
local DROP = s(15)
local DROP_GAP = s(21)
local DROPS = 18
local DROPS_W = (DROPS - 1) * DROP_GAP + DROP * 2
local DROPS_X = math.floor((W - DROPS_W) / 2)
-- Directly under the name, so the shape, the name and what is being typed read
-- as one thing rather than three placed on the same screen.
local DROPS_Y = AVATAR_Y + AVATAR + s(74)

local drops = {
  x = DROPS_X,
  y = DROPS_Y,
  width = DROPS_W,
  height = DROP * 2,
  fill_color = function() return alarmed:get() and ALERT or ACCENT end,
  blend = math.floor(DROP * 0.9),
  behavior = { fill_color = { duration = 320, easing = "out_quad" } },
}
for index = 1, DROPS do
  -- An untyped slot is a small mark rather than nothing at all: a row of them
  -- says where the password goes, and each grows into a drop as it is filled.
  local REST = math.floor(DROP * 0.34)
  local size = function()
    return typed:get() >= index and DROP * 2 or REST
  end
  local inset = function()
    return typed:get() >= index and 0 or DROP - REST / 2
  end
  drops[#drops + 1] = ui.SdfShape {
    x = function() return (index - 1) * DROP_GAP + inset() end,
    y = function() return inset() end,
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
-- Sessions, accounts and the machine's own controls.
--------------------------------------------------------------------------------

--- A small labelled button. Words rather than symbols: ⏻ is in most fonts but
--- ⟳ and ⌨ are not, and a greeter that renders a tofu box has told the person
--- in front of it nothing. It is also the one screen where guessing is costly.
local BUTTON_H = s(34)
local function button(id, label, width, lit, on_tap)
  local hot = morf.signal("greeter.hot." .. id, false)
  local function live() return hot:get() or lit() end
  return ui.Item {
    width = width,
    height = BUTTON_H,
    ui.Rect {
      anchors = { fill = true },
      radius = BUTTON_H / 2,
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
-- would sit over every button and swallow the taps meant for them. Key handlers
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
place(drift(1, math.floor(W * 0.24), math.floor(H * 0.28), s(360), "#15364a", s(110)))
place(drift(2, math.floor(W * 0.78), math.floor(H * 0.72), s(300), "#241f3d", s(90)))

for _, node in ipairs(clock_nodes) do place(node) end
place(today)
place(avatar)

place(ui.Text {
  y = AVATAR_Y + AVATAR + s(20),
  width = W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or "no accounts found"
  end,
  font_size = s(23),
  font_weight = 500,
  horizontal_alignment = "center",
  color = TEXT,
})

place(ui.Sdf(drops))

place(ui.Text {
  y = DROPS_Y + DROP * 2 + s(18),
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

-- Accounts and sessions along one line, so the two choices this screen offers
-- sit together instead of bracketing it.
local CHOICE_Y = DROPS_Y + DROP * 2 + s(54)
local choices = {}
for index, user in ipairs(users) do
  choices[#choices + 1] = {
    label = user.label,
    width = s(150),
    lit = function() return chosen_user:get() == index end,
    tap = function()
      pick_user(index)
      show_user(index)
    end,
    id = "user" .. index,
  }
end
for index, entry in ipairs(available) do
  choices[#choices + 1] = {
    label = entry.name,
    width = s(186),
    lit = function() return chosen_session:get() == index end,
    tap = function()
      if not working:get() then write(chosen_session, index) end
    end,
    id = "session" .. index,
  }
end
do
  local span = -s(10)
  for _, choice in ipairs(choices) do span = span + choice.width + s(10) end
  local x = math.floor((W - span) / 2)
  for _, choice in ipairs(choices) do
    local node = button(choice.id, choice.label, choice.width, choice.lit, choice.tap)
    node.x = x
    node.y = CHOICE_Y
    place(node)
    x = x + choice.width + s(10)
  end
end

local ACTION_W = s(98)
local function corner(index, label, on_tap)
  local node = button(label, label, ACTION_W, function() return false end, on_tap)
  node.x = W - ACTION_W * index - s(24) * index
  node.y = s(24)
  return node
end
place(corner(1, "shut down", function() power("PowerOff") end))
place(corner(2, "restart", function() power("Reboot") end))
place(corner(3, "sleep", function() power("Suspend") end))
do
  local node = button("keyboard", "keyboard", ACTION_W, function() return false end, open_keyboard)
  node.x = s(24)
  node.y = s(24)
  place(node)
end

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

place(clock_swap)
place(face_swap)

ui.Item(tree)
