-- A login screen, wearing GDM's clothes.
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
-- ON THE LOOK. Every measurement here is GDM's own, read out of gnome-shell's
-- stylesheet rather than eyeballed from a screenshot:
--
--     background          #222226   $system_base_color
--     foreground          #fafafb   $system_fg_color
--     dimmed text         #dbdbe7   darken($system_fg_color, 10%)
--     list item           #353539   mix(fg, base, 9%)
--     entry               #404046   mix(fg, $system_bg_color, 9%)
--     avatar well         13% fg    transparentize($system_fg_color, .87)
--     user list width     25em      $_gdm_dialog_width
--     item radius         16px      $modal_radius
--     entry radius        12px      $base_border_radius * 1.5
--     large avatar        160px     $base_icon_size * 10
--     name, prompt        20pt/400  .user-widget.vertical .user-widget-label
--     name, list          15pt/700  %title_3
--     bottom buttons      32px pad, 16px spacing
--
-- And the shape of it is GDM's too: a list of accounts in the middle of the
-- screen, and choosing one puts the list away and asks that one account for a
-- password. Not everything at once.
--
-- What is *not* GDM is how it is drawn. The dots in the password field are a
-- distance field, not a string of `●` — a font that has not got the character
-- draws a tofu box, and a login screen is the worst place to find that out. So
-- is the arrow: a triangle in a field, at any size, in any font. The avatar's
-- letter morphs from one account's initial to the next, because a glyph is an
-- outline and two outlines can be walked between.

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

-- GDM is laid out in `em` against an 11pt base, so it grows with the font
-- rather than with the panel. Same idea: one number, everything in proportion.
local SCALE = math.max(0.75, math.min(1.7, math.min(W / 1920, H / 1080)))
local function s(n) return math.floor(n * SCALE) end

local INK = "#222226"
local TEXT = "#fafafb"
local DIM = "#dbdbe7"
local CARD = "#353539"
local CARD_HOT = "#40404a"
local ENTRY = "#404046"
local WELL = "#fafafb21"
local ACCENT = "#3584e4"
local ALERT = "#ff7b63"

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end
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
---
--- `command` and not `exec`: greetd takes an argument vector, and `Exec` is a
--- command *line*. Handing the line over as a single argument asks the kernel
--- for a program named `uwsm start -e -D Hyprland hyprland.desktop`, which is
--- the shape of every session on this machine that takes an argument at all.
---
--- Each one also carries the environment it expects to be started in. A session
--- launched without `XDG_CURRENT_DESKTOP` comes up with the portals guessing:
--- screen sharing and file dialogs pick a backend by asking what desktop this
--- is, and nothing has told them.
local function sessions()
  local found = {}
  for _, entry in ipairs(core.desktop_entries(core.session_paths()):applications()) do
    if entry.command and #entry.command > 0 then
      -- Which directory it was found in is the only statement anywhere of
      -- whether a session is Wayland or X11.
      local kind = entry.source:match("xsessions$") and "x11" or "wayland"
      local environment = {
        "XDG_SESSION_TYPE=" .. kind,
        "XDG_SESSION_DESKTOP=" .. entry.id,
        "DESKTOP_SESSION=" .. entry.id,
      }
      if #entry.desktop_names > 0 then
        environment[#environment + 1] =
          "XDG_CURRENT_DESKTOP=" .. table.concat(entry.desktop_names, ":")
      end
      found[#found + 1] = {
        name = entry.name,
        command = entry.command,
        environment = environment,
      }
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
    local started = session:start_session(wanted.command, wanted.environment)
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

--------------------------------------------------------------------------------
-- Which half of the screen you are on.
--------------------------------------------------------------------------------

-- GDM has two states and shows one at a time: a list of accounts, and then one
-- account being asked for a password. The list is not dimmed behind the prompt
-- or tucked into a corner — it is gone, and the way back is the cancel button.
local asking = morf.signal("greeter.asking", false)

local function choose_user(index)
  if working:get() then return end
  write(chosen_user, index)
  clear_password()
  say("enter password")
  write(asking, true)
end

local function go_back()
  if working:get() then return end
  clear_password()
  say("enter password")
  write(asking, false)
end

--------------------------------------------------------------------------------
-- The user list.
--------------------------------------------------------------------------------

local LIST_W = s(367)          -- 25em, $_gdm_dialog_width
local ITEM_PAD = s(9)          -- $base_padding * 1.5
local FACE_SM = s(48)
local ITEM_H = FACE_SM + ITEM_PAD * 2
local ITEM_GAP = s(12)         -- $base_padding * 2
local ITEM_RADIUS = s(16)      -- $modal_radius

--- One account, as GDM draws it: a round well with the initial in it, and the
--- name beside it, on a card that lights up under the pointer.
local function user_row(index, user)
  local hot = morf.signal("greeter.row." .. index, false)
  return ui.Item {
    width = LIST_W,
    height = ITEM_H,
    ui.Rect {
      anchors = { fill = true },
      radius = ITEM_RADIUS,
      color = function() return hot:get() and CARD_HOT or CARD end,
      behavior = { color = { duration = 150, easing = "out_quad" } },
    },
    ui.Sdf {
      x = ITEM_PAD,
      y = ITEM_PAD,
      width = FACE_SM,
      height = FACE_SM,
      fill_color = WELL,
      ui.SdfShape { width = FACE_SM, height = FACE_SM, shape = "circle" },
    },
    ui.Text {
      x = ITEM_PAD,
      y = ITEM_PAD,
      width = FACE_SM,
      height = FACE_SM,
      text = user.initial,
      font_size = s(20),
      font_weight = 700,
      horizontal_alignment = "center",
      vertical_alignment = "center",
      color = TEXT,
    },
    ui.Text {
      -- $base_padding * 3, the horizontal user widget's spacing.
      x = ITEM_PAD + FACE_SM + s(18),
      y = ITEM_PAD,
      width = LIST_W - (ITEM_PAD * 2 + FACE_SM + s(18)),
      height = FACE_SM,
      text = user.label,
      font_size = s(20),        -- %title_3, 15pt
      font_weight = 700,
      vertical_alignment = "center",
      elide = "right",
      color = TEXT,
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = function() choose_user(index) end,
    },
  }
end

local LIST_H = math.max(1, #users) * ITEM_H + math.max(0, #users - 1) * ITEM_GAP
local LIST_X = math.floor((W - LIST_W) / 2)
local LIST_Y = math.floor((H - LIST_H) / 2)

local list_view = { x = LIST_X, y = LIST_Y, width = LIST_W, height = LIST_H + s(70) }
list_view.visible = function() return not asking:get() end
for index, user in ipairs(users) do
  local node = user_row(index, user)
  node.y = (index - 1) * (ITEM_H + ITEM_GAP)
  list_view[#list_view + 1] = node
end
if #users == 0 then
  list_view[#list_view + 1] = ui.Text {
    width = LIST_W,
    height = ITEM_H,
    text = "no accounts on this machine",
    font_size = s(15),
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = DIM,
  }
end
-- `.login-dialog-not-listed-label` — %heading, and left-aligned under the list.
list_view[#list_view + 1] = ui.Text {
  x = s(6),
  y = LIST_H + s(22),
  width = LIST_W,
  text = "Not listed?",
  font_size = s(15),
  font_weight = 700,
  color = DIM,
}

--------------------------------------------------------------------------------
-- The prompt.
--------------------------------------------------------------------------------

local FACE_LG = s(160)         -- $base_icon_size * 10
local PROMPT_W = s(440)        -- 25em * 1.2
local ENTRY_W = s(400)
local ENTRY_H = s(46)
-- The avatar, the `.75em` gap under the name, and the name's own line.
local ENTRY_Y = FACE_LG + s(24) + s(38) + s(20)
local PROMPT_X = math.floor((W - PROMPT_W) / 2)
local PROMPT_Y = math.floor(H * 0.5) - s(210)

local face = morf.signal("greeter.face", 0)
local face_from = users[1] and users[1].initial or "?"
local face_to = face_from
local initial_letter
local face_swap

--- The letter in the well walks from one account's initial to the next. Two
--- outlines correspond, so what is on screen in between is a letterform and
--- not a cross-fade of two pictures.
local function morph_initial(index)
  local user = users[index]
  if not user or user.initial == face_from then return end
  face_to = user.initial
  initial_letter.glyph_morph_to = face_to
  write(face, 1)
  face_swap.running = true
end

initial_letter = ui.SdfShape {
  x = math.floor(FACE_LG * 0.32),
  y = math.floor(FACE_LG * 0.28),
  width = math.floor(FACE_LG * 0.36),
  height = math.floor(FACE_LG * 0.44),
  glyph = face_from,
  glyph_morph_to = face_from,
  morph_progress = function() return face:get() end,
  behavior = { morph_progress = { duration = 400, easing = "in_out_cubic" } },
}

face_swap = ui.Timer {
  interval = 400,
  ["repeat"] = false,
  running = false,
  on_triggered = function()
    face_from = face_to
    initial_letter.glyph = face_from
    write(face, 0)
  end,
}

-- The dots, as a field rather than a string of `●`. A row of circles that do
-- not touch, so it reads as a count and not as a smear.
local DOT = s(9)
local DOT_GAP = s(17)
local DOTS = 20
local dot_row = {
  x = s(20),
  y = math.floor((ENTRY_H - DOT) / 2),
  width = DOTS * DOT_GAP,
  height = DOT,
  fill_color = TEXT,
}
for index = 1, DOTS do
  -- Grown into rather than switched on, so a held key reads as a rhythm. Size
  -- and not `opacity`: the layers of a field compose into one shape before
  -- anything is painted, so a layer has no opacity of its own to turn down —
  -- what it has is a radius, and a circle of no radius is nothing.
  dot_row[#dot_row + 1] = ui.SdfShape {
    x = function()
      return (index - 1) * DOT_GAP + (typed:get() >= index and 0 or DOT / 2)
    end,
    y = function() return typed:get() >= index and 0 or DOT / 2 end,
    width = function() return typed:get() >= index and DOT or 0 end,
    height = function() return typed:get() >= index and DOT or 0 end,
    shape = "circle",
    behavior = {
      x = { duration = 110, easing = "out_quad" },
      y = { duration = 110, easing = "out_quad" },
      width = { duration = 110, easing = "out_quad" },
      height = { duration = 110, easing = "out_quad" },
    },
  }
end

local prompt = {
  x = PROMPT_X,
  y = PROMPT_Y,
  width = PROMPT_W,
  height = s(430),
  visible = function() return asking:get() end,
}

-- `.login-dialog-message` — centred, dimmed, and holding 2.75em of height
-- whether or not it has anything to say, so nothing below it jumps.
--
-- Declared before the avatar rather than after the entry, though it is drawn
-- below it: whatever node came directly after the entry in tree order did not
-- draw at all — any element, at any position, at any depth — and only moving
-- it out of that slot brings it back. That is a renderer bug and it wants
-- finding; it is not worth a login screen that cannot say a password was wrong.
prompt[#prompt + 1] = ui.Item {
  x = 0,
  y = ENTRY_Y + ENTRY_H + s(24),
  width = PROMPT_W,
  height = s(40),
  visible = function() return asking:get() end,
  ui.Text {
    width = PROMPT_W,
    height = s(40),
    text = function()
      if working:get() then return "Authenticating…" end
      return message:get()
    end,
    font_size = s(15),
    horizontal_alignment = "center",
    color = function() return alarmed:get() and ALERT or DIM end,
    behavior = { color = { duration = 320, easing = "out_quad" } },
  },
}

prompt[#prompt + 1] = ui.Sdf {
  x = math.floor((PROMPT_W - FACE_LG) / 2),
  width = FACE_LG,
  height = FACE_LG,
  fill_color = WELL,
  ui.SdfShape { width = FACE_LG, height = FACE_LG, shape = "circle" },
}
-- The letter is its own field on top rather than a hole in the well, so it
-- reads the same way round as the letters in the list: light on dim, not the
-- desktop showing through.
prompt[#prompt + 1] = ui.Sdf {
  x = math.floor((PROMPT_W - FACE_LG) / 2),
  width = FACE_LG,
  height = FACE_LG,
  fill_color = TEXT,
  initial_letter,
}

-- `.user-widget.vertical` — 20pt, weight 400, centred, and a `.75em` gap under
-- it before anything else.
prompt[#prompt + 1] = ui.Text {
  y = FACE_LG + s(24),
  width = PROMPT_W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or ""
  end,
  font_size = s(27),
  font_weight = 400,
  horizontal_alignment = "center",
  color = TEXT,
}

-- `.login-dialog-input-well`.
local input_well = {
  y = ENTRY_Y,
  width = PROMPT_W,
  height = s(160),
}

input_well[#input_well + 1] = ui.Item {
  x = math.floor((PROMPT_W - ENTRY_W) / 2),
  y = 0,
  width = ENTRY_W,
  height = ENTRY_H,
  ui.Rect {
    anchors = { fill = true },
    radius = s(12),           -- $base_border_radius * 1.5
    color = ENTRY,
    -- GDM rings the entry in the accent colour while it has the keys, and in
    -- red when the answer was wrong. It always has the keys here.
    border_width = s(2),
    border_color = function() return alarmed:get() and ALERT or ACCENT end,
    behavior = { border_color = { duration = 250, easing = "out_quad" } },
  },
  ui.Text {
    x = s(20),
    anchors = { fill = true },
    text = function() return typed:get() == 0 and "Password" or "" end,
    font_size = s(16),
    vertical_alignment = "center",
    color = DIM,
    opacity = 0.7,
  },
  ui.Sdf(dot_row),
  -- The submit arrow: a triangle in a field, so it is the same mark in every
  -- font and at every size.
  ui.Sdf {
    x = ENTRY_W - ENTRY_H,
    width = ENTRY_H,
    height = ENTRY_H,
    fill_color = function() return typed:get() > 0 and TEXT or DIM end,
    opacity = function() return typed:get() > 0 and 1 or 0.35 end,
    behavior = { opacity = { duration = 180, easing = "out_quad" } },
    ui.SdfShape {
      x = math.floor(ENTRY_H * 0.34),
      y = math.floor(ENTRY_H * 0.30),
      width = math.floor(ENTRY_H * 0.32),
      height = math.floor(ENTRY_H * 0.40),
      shape = "triangle",
      rotation = 90,
    },
  },
  ui.MouseArea { anchors = { fill = true }, on_clicked = function() attempt() end },
}

prompt[#prompt + 1] = ui.Item(input_well)

--------------------------------------------------------------------------------
-- Buttons, in GDM's corners.
--------------------------------------------------------------------------------

local PILL_H = s(40)
local BUTTON_PAD = s(32)       -- .login-dialog-bottom-button-group
local BUTTON_GAP = s(16)

--- A flat pill that lights up under the pointer, which is what every button on
--- GDM's login screen is.
local function pill(id, label, width, lit, on_tap)
  local hot = morf.signal("greeter.pill." .. id, false)
  local function live() return hot:get() or lit() end
  return ui.Item {
    width = width,
    height = PILL_H,
    ui.Rect {
      anchors = { fill = true },
      radius = s(999),
      color = function() return live() and CARD_HOT or CARD end,
      behavior = { color = { duration = 150, easing = "out_quad" } },
    },
    ui.Text {
      anchors = { fill = true },
      text = label,
      font_size = s(15),
      font_weight = 700,
      horizontal_alignment = "center",
      vertical_alignment = "center",
      elide = "right",
      color = function() return live() and TEXT or DIM end,
      behavior = { color = { duration = 150, easing = "out_quad" } },
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
      if asking:get() then
        attempt()
      elseif #users > 0 then
        choose_user(chosen_user:get())
        morph_initial(chosen_user:get())
      end
    elseif key == "BackSpace" then
      backspace()
    elseif key == "Escape" then
      if asking:get() then go_back() else clear_password() end
    elseif key == "Tab" and modifiers and modifiers.control then
      if #available > 0 then write(chosen_session, chosen_session:get() % #available + 1) end
    elseif key == "Tab" then
      if #users > 0 then
        local next_user = chosen_user:get() % #users + 1
        write(chosen_user, next_user)
        morph_initial(next_user)
        if asking:get() then clear_password() end
      end
    elseif key == "F1" then
      open_keyboard()
    elseif asking:get() and text and text ~= "" then
      type_character(text)
    end
  end,
})

place(ui.Rect { width = W, height = H, color = INK })
place(ui.Item(list_view))



-- Bottom right: the session, then power. Laid out from the right edge inwards,
-- which is the order GDM's button group ends up in.
local right = W - BUTTON_PAD
local function place_right(node)
  right = right - node.width
  node.x = right
  node.y = H - BUTTON_PAD - PILL_H
  place(node)
  right = right - BUTTON_GAP
end

for index = #available, 1, -1 do
  local entry = available[index]
  place_right(pill("session" .. index, entry.name, s(190),
    function() return chosen_session:get() == index end,
    function() if not working:get() then write(chosen_session, index) end end))
end
if #available == 0 then
  place_right(pill("nosession", "no sessions", s(190), function() return false end,
                   function() end))
end
place_right(pill("sleep", "Suspend", s(120), function() return false end,
                 function() power("Suspend") end))
place_right(pill("restart", "Restart", s(120), function() return false end,
                 function() power("Reboot") end))
place_right(pill("shutdown", "Power Off", s(130), function() return false end,
                 function() power("PowerOff") end))

-- Bottom left: the keyboard, and the way back out of the prompt.
do
  local node = pill("keyboard", "Keyboard", s(130), function() return false end,
                    open_keyboard)
  node.x = BUTTON_PAD
  node.y = H - BUTTON_PAD - PILL_H
  place(node)

  -- Wrapped, because a binding is made when a node is built and not afterwards:
  -- `cancel.visible = function() ... end` assigns a function to a live property
  -- rather than declaring one, which is a different thing and refused.
  place(ui.Item {
    x = BUTTON_PAD + s(130) + BUTTON_GAP,
    y = H - BUTTON_PAD - PILL_H,
    width = s(120),
    height = PILL_H,
    visible = function() return asking:get() end,
    pill("cancel", "Cancel", s(120), function() return false end, go_back),
  })
end

-- The prompt last, so it is drawn over everything and nothing is drawn after
-- it. Topmost is where the focused thing belongs anyway — and it sidesteps a
-- renderer fault this screen kept walking into: whatever node came directly
-- after the prompt's entry in draw order did not draw at all, whichever element
-- it was and wherever it sat. It wants finding, but a login screen is not the
-- place to leave it showing.
place(ui.Item(prompt))

place(face_swap)

ui.Item(tree)
