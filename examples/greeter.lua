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
-- The keyboard. Not a keyboard drawn to look like it — the same file
-- `examples/keyboard.lua` puts in a surface of its own. It cannot have one here
-- because a kiosk compositor shows a single window, so a login screen's second
-- surface is never seen; drawn into this one it is the same board either way.
local board = require("lib.board")

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

-- Worked out from the stylesheet rather than picked by eye.
local INK = "#222226"       -- $system_base_color
local TEXT = "#fafafb"      -- $system_fg_color
local DIM = "#dedee4"       -- darken($system_fg_color, 10%)
local CARD = "#353539"      -- button(normal) over the base colour
local CARD_HOT = "#3a3a3f"
local BUTTON = "#404045"    -- button(normal) over $system_bg_color, and %system_entry
local BUTTON_HOT = "#45454b"
local WELL = "#fafafb21"    -- transparentize($system_fg_color, .87)
local ACCENT = "#86b5ef"    -- the dark-variant accent, for the ring under a key
local ACCENT_RING = "#86b5ef33" -- the accent at a fifth, which is the focus ring
local ALERT = "#ff7b63"     -- not GDM's, which says a refusal in words alone —
                            -- but a screen that only answers in text is a screen
                            -- you have to read to know it heard you

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
-- A key struck, and a password refused. Both are set to one and dropped back to
-- zero a moment later; the springs on the properties they drive do the rest, so
-- neither costs Lua anything per frame.
local kick = morf.signal("greeter.kick", 0)
local shake = morf.signal("greeter.shake", 0)
local drift = morf.signal("greeter.drift", 0)
local kick_rest, shake_rest, unmount_rest
-- `asking` flips the instant a choice is made, so the fade has something to
-- run against; these say which panel is still *mounted*. The one arriving is
-- mounted at once and the one leaving stays until it has finished going, or
-- there is nothing on screen to watch it go.
local list_up = morf.signal("greeter.up.list", true)
local prompt_up = morf.signal("greeter.up.prompt", false)

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
    -- Nested inside a session there is no `$GREETD_SOCK`, so there is nothing
    -- to authenticate against and Return can do nothing. The password is left
    -- where it is: nobody typed it in order to have it thrown away, and the
    -- screen is otherwise perfectly usable to look at and to type in.
    say("no greetd to ask — nested, so Return does nothing", true)
    write(working, false)
    write(shake, 1)
    if shake_rest then shake_rest.running = true end
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
  write(shake, 1)
  if shake_rest then shake_rest.running = true end
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

--- Shows and hides the board.
local shown = morf.signal("greeter.board", false)
local function open_keyboard()
  write(shown, not shown:get())
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
  -- Every key nudges the entry, and the spring settles it. Written here rather
  -- than in the key handler so a character arriving from the on-screen keyboard
  -- looks the same as one from a real one.
  write(kick, 1)
  if kick_rest then kick_rest.running = true end
  if alarmed:get() then say("enter password") end
end

local function backspace()
  if working:get() then return end
  password = password:sub(1, -2)
  write(typed, #password)
end


--------------------------------------------------------------------------------
-- Icons, as distance fields.
--------------------------------------------------------------------------------

--- GNOME draws symbolic SVGs from an icon theme, and so does this — except
--- that here the drawing is not turned into a picture on the way. An SVG is a
--- set of closed curves, a field takes closed curves, so the file *is* the
--- shape: it composes, it is cut out of things, and it could morph into a
--- letter if it were asked to. Nothing is rasterised, so the same file is exact
--- at any size on any panel.
local function icon(name, box, extra)
  local field = {
    width = box,
    height = box,
    fill_color = TEXT,
    ui.SdfShape {
      width = box,
      height = box,
      source = core.shell_path("assets/icon-" .. name .. ".svg"),
    },
  }
  for key, value in pairs(extra or {}) do field[key] = value end
  return ui.Sdf(field)
end

--------------------------------------------------------------------------------
-- Which of the two screens you are on.
--------------------------------------------------------------------------------

-- GDM shows one at a time: a list of accounts, and then one account being asked
-- for a password. The list is not dimmed behind the prompt or moved into a
-- corner — it is gone, and the way back is the button left of the entry.
local asking = morf.signal("greeter.asking", false)
local face = morf.signal("greeter.face", 0)
local face_from = users[1] and users[1].initial or "?"
local face_to = face_from
local initial_letter
local face_swap

--- The letter in the well walks from one account's initial to the next. Two
--- outlines correspond, so what is on screen in between is a letterform rather
--- than a cross-fade of two pictures.
local function morph_initial(index)
  local user = users[index]
  if not user or user.initial == face_from then return end
  face_to = user.initial
  initial_letter.glyph_morph_to = face_to
  write(face, 1)
  face_swap.running = true
end

--- Crosses from one panel to the other: the arriving one is mounted at once so
--- it has somewhere to fade in from, and the leaving one is taken down only
--- after it has finished leaving.
local function cross(to_prompt)
  write(asking, to_prompt)
  write(to_prompt and prompt_up or list_up, true)
  if unmount_rest then unmount_rest.running = true end
end

local function choose_user(index)
  if working:get() then return end
  write(chosen_user, index)
  morph_initial(index)
  clear_password()
  say("")
  cross(true)
end

local function go_back()
  if working:get() then return end
  clear_password()
  say("")
  cross(false)
end

--------------------------------------------------------------------------------
-- The user list.
--------------------------------------------------------------------------------

-- `.login-dialog-user-list-view` is 25em wide; an item is a 64px avatar with
-- `$base_padding * 1.5` around it, `$modal_radius` corners and `$base_padding
-- * 2` between them, and the horizontal user widget puts `$base_padding * 3`
-- between the avatar and the name.
local LIST_W = s(400)
local FACE_SM = s(64)
local ITEM_PAD = s(9)
local ITEM_H = FACE_SM + ITEM_PAD * 2
local ITEM_GAP = s(12)
local ITEM_RADIUS = s(16)
local NAME_GAP = s(18)

local function user_row(index, user)
  local hot = morf.signal("greeter.row." .. index, false)
  local held = morf.signal("greeter.rowheld." .. index, false)
  -- Under the pointer, or under the keyboard. Tab walks the list without a
  -- pointer anywhere near it, and a row that only lights up for the mouse is a
  -- list you cannot see yourself moving through.
  local function live() return hot:get() or chosen_user:get() == index end
  return ui.Item {
    width = LIST_W,
    height = ITEM_H,
    -- The row comes towards you rather than lighting up in place: a little
    -- larger, and lifted, with the press putting it back down again.
    scale = function()
      if held:get() then return 0.985 end
      return live() and 1.02 or 1.0
    end,
    translate_x = function() return live() and s(4) or 0 end,
    behavior = {
      scale = { kind = "spring", mass = 1, damping = 15, stiffness = 340, epsilon = 0.002 },
      translate_x = { kind = "spring", mass = 1, damping = 15, stiffness = 340,
                      epsilon = 0.05 },
    },
    ui.Rect {
      anchors = { fill = true },
      radius = ITEM_RADIUS,
      color = function() return live() and CARD_HOT or CARD end,
      -- The accent arrives on the edge before it arrives anywhere else, which
      -- is how a list says which row is *the* row without shouting.
      border_width = s(2),
      border_color = function() return live() and ACCENT or "#00000000" end,
      behavior = {
        color = { duration = 150, easing = "out_quad" },
        border_color = { duration = 200, easing = "out_quad" },
      },
    },
    ui.Sdf {
      x = ITEM_PAD, y = ITEM_PAD, width = FACE_SM, height = FACE_SM,
      fill_color = function() return live() and "#fafafb2e" or WELL end,
      scale = function() return live() and 1.06 or 1.0 end,
      behavior = {
        fill_color = { duration = 180, easing = "out_quad" },
        scale = { kind = "spring", mass = 1, damping = 12, stiffness = 400, epsilon = 0.002 },
      },
      ui.SdfShape { width = FACE_SM, height = FACE_SM, shape = "circle" },
    },
    ui.Text {
      x = ITEM_PAD, y = ITEM_PAD, width = FACE_SM, height = FACE_SM,
      text = user.initial,
      font_size = s(24),
      font_weight = 700,
      horizontal_alignment = "center",
      vertical_alignment = "center",
      color = TEXT,
    },
    ui.Text {
      x = ITEM_PAD + FACE_SM + NAME_GAP,
      y = ITEM_PAD,
      width = LIST_W - (ITEM_PAD * 2 + FACE_SM + NAME_GAP),
      height = FACE_SM,
      text = user.label,
      font_size = s(16),          -- %title_3
      font_weight = 700,
      vertical_alignment = "center",
      elide = "right",
      color = TEXT,
    },
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function()
        write(hot, true)
        -- Hovering also *selects*, so the keyboard and the pointer never
        -- disagree about which row is the one Return would take.
        if not working:get() then
          write(chosen_user, index)
          morph_initial(index)
        end
      end,
      on_exited = function()
        write(hot, false)
        write(held, false)
      end,
      on_pressed = function() write(held, true) end,
      on_released = function() write(held, false) end,
      on_clicked = function() choose_user(index) end,
    },
  }
end

local LIST_H = math.max(1, #users) * ITEM_H + math.max(0, #users - 1) * ITEM_GAP
local LIST_X = math.floor((W - LIST_W) / 2)
-- `.login-dialog-user-selection-box` reserves 4em above and 8em below, and the
-- box is centred — so the list itself sits a couple of ems above the middle.
local LIST_Y = math.floor((H - LIST_H) / 2) - s(32)

-- The two panels pass each other: the one leaving goes the way you are not
-- going and the one arriving comes from the way you are. A cut between them
-- says nothing about which way round they are; this says it without a word.
local list_view = {
  x = LIST_X, y = LIST_Y, width = LIST_W, height = LIST_H + s(56),
  visible = function() return list_up:get() end,
  opacity = function() return asking:get() and 0.0 or 1.0 end,
  translate_x = function() return asking:get() and -s(90) or 0 end,
  translate_y = function() return shown:get() and -s(180) or 0 end,
  scale = function() return asking:get() and 0.94 or 1.0 end,
  behavior = {
    -- Springs, not durations. A duration says how long the movement takes and
    -- nothing about how it feels; a spring says how heavy the thing is, and
    -- crossing back before it has arrived carries the momentum it already had
    -- rather than restarting from wherever it got to.
    opacity = { duration = 190, easing = "out_quad" },
    translate_x = { kind = "spring", mass = 1, damping = 21, stiffness = 150,
                    epsilon = 0.05 },
    translate_y = { kind = "spring", mass = 1, damping = 20, stiffness = 190,
                    epsilon = 0.05 },
    scale = { kind = "spring", mass = 1, damping = 19, stiffness = 170,
              epsilon = 0.002 },
  },
}
for index, user in ipairs(users) do
  local node = user_row(index, user)
  node.y = (index - 1) * (ITEM_H + ITEM_GAP)
  list_view[#list_view + 1] = node
end
if #users == 0 then
  list_view[#list_view + 1] = ui.Text {
    width = LIST_W, height = ITEM_H,
    text = "no accounts on this machine",
    font_size = s(16),
    horizontal_alignment = "center",
    vertical_alignment = "center",
    color = DIM,
  }
end
-- `.login-dialog-not-listed-label` is `%heading`, and the button it sits in is
-- aligned to the start of the list rather than centred under it.
list_view[#list_view + 1] = ui.Text {
  x = s(6),
  y = LIST_H + s(18),
  width = LIST_W,
  text = "Not listed?",
  font_size = s(12),
  font_weight = 700,
  color = DIM,
}

--------------------------------------------------------------------------------
-- The prompt.
--------------------------------------------------------------------------------

-- `.login-dialog-prompt-layout` is 30em wide, and the dialog puts a fixed-top
-- actor at `centre - 550/2` with `margin-top: 80px` on top of that.
local PROMPT_W = s(480)
local PROMPT_X = math.floor((W - PROMPT_W) / 2)
local PROMPT_Y = math.floor(H / 2) - s(275) + s(80)

local FACE_LG = s(160)          -- `$base_icon_size * 10`
local ROW_H = s(64)             -- `.login-dialog-button-box` is 4em tall
local CANCEL = s(40)            -- 16px icon with `$base_padding * 2` around it
local ENTRY_MARGIN = s(20)      -- `.login-dialog-prompt-entry-area`
local ENTRY_H = ROW_H - s(16)
local ENTRY_X = CANCEL + ENTRY_MARGIN
local ENTRY_W = PROMPT_W - ENTRY_X - ENTRY_MARGIN
local ENTRY_RADIUS = s(12)
local ICON = s(16)              -- `$scalable_icon_size`

-- The avatar, the 24px under it, the 22px name, and its `.75em` bottom margin.
local ROW_Y = FACE_LG + s(24) + s(26) + s(16)

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

--- A round button with a field for a face, which is every button GDM's login
--- screen has: `.icon-button` is circular, `%system_button` coloured, and the
--- icon inside it is `$scalable_icon_size`.
local function round_button(id, name, size, on_tap)
  local hot = morf.signal("greeter.button." .. id, false)
  local held = morf.signal("greeter.held." .. id, false)
  local mark_box = math.floor(size * 0.42)
  -- The circle grows a little and the mark grows more, so the icon appears to
  -- come forward out of the button rather than the whole thing simply swelling.
  -- Springs rather than easings: a pointer arriving and leaving is not a
  -- scheduled thing, and a spring can be interrupted halfway and stay smooth.
  local function pounce(rest, over, down)
    return function()
      if held:get() then return down end
      return hot:get() and over or rest
    end
  end
  return ui.Item {
    width = size,
    height = size,
    scale = pounce(1.0, 1.10, 0.94),
    behavior = {
      scale = { kind = "spring", mass = 1, damping = 13, stiffness = 380, epsilon = 0.002 },
    },
    ui.Sdf {
      width = size, height = size,
      fill_color = function() return hot:get() and BUTTON_HOT or BUTTON end,
      behavior = { fill_color = { duration = 150, easing = "out_quad" } },
      ui.SdfShape { width = size, height = size, shape = "circle" },
    },
    -- Each mark moves the way its own meaning moves: the chevrons slide the way
    -- they point, the two that are circles turn, and the rest simply come
    -- forward. An icon that animates against what it means is worse than one
    -- that does not animate at all.
    icon(name, mark_box, {
      x = math.floor((size - mark_box) / 2),
      y = math.floor((size - mark_box) / 2),
      scale = pounce(1.0, 1.14, 0.9),
      translate_x = function()
        local lean = (name == "back" and -1) or (name == "next" and 1) or 0
        return hot:get() and lean * s(3) or 0
      end,
      rotation = function()
        if not hot:get() then return 0 end
        -- A restart turns forwards and a suspend rolls back; a power button
        -- does not turn at all, because a power symbol has an up.
        if name == "restart" then return 90 end
        if name == "suspend" then return -18 end
        return 0
      end,
      behavior = {
        scale = { kind = "spring", mass = 1, damping = 11, stiffness = 460, epsilon = 0.002 },
        translate_x = { kind = "spring", mass = 1, damping = 12, stiffness = 420,
                        epsilon = 0.05 },
        rotation = { kind = "spring", mass = 1, damping = 16, stiffness = 190,
                     epsilon = 0.05 },
      },
    }),
    ui.MouseArea {
      anchors = { fill = true },
      on_entered = function() write(hot, true) end,
      on_exited = function()
        write(hot, false)
        write(held, false)
      end,
      on_pressed = function() write(held, true) end,
      on_released = function() write(held, false) end,
      on_clicked = on_tap,
    },
  }
end

local prompt = {
  x = PROMPT_X, y = PROMPT_Y, width = PROMPT_W, height = s(400),
  visible = function() return prompt_up:get() end,
  opacity = function() return asking:get() and 1.0 or 0.0 end,
  translate_x = function() return asking:get() and 0 or s(90) end,
  translate_y = function() return shown:get() and -s(180) or 0 end,
  scale = function() return asking:get() and 1.0 or 0.94 end,
  behavior = {
    -- Springs, not durations. A duration says how long the movement takes and
    -- nothing about how it feels; a spring says how heavy the thing is, and
    -- crossing back before it has arrived carries the momentum it already had
    -- rather than restarting from wherever it got to.
    opacity = { duration = 190, easing = "out_quad" },
    translate_x = { kind = "spring", mass = 1, damping = 21, stiffness = 150,
                    epsilon = 0.05 },
    translate_y = { kind = "spring", mass = 1, damping = 20, stiffness = 190,
                    epsilon = 0.05 },
    scale = { kind = "spring", mass = 1, damping = 19, stiffness = 170,
              epsilon = 0.002 },
  },
}

-- The well, and the letter on it. Two fields rather than one because the letter
-- is ink on the well and not a hole through it.
-- The well answers the typing as well as the ring does: a small push per key,
-- and a brighter face while one is held. It is the biggest thing on the screen,
-- so it moves the least.
prompt[#prompt + 1] = ui.Sdf {
  x = math.floor((PROMPT_W - FACE_LG) / 2),
  width = FACE_LG, height = FACE_LG,
  fill_color = function() return kick:get() > 0 and "#fafafb2e" or WELL end,
  scale = function() return 1.0 + kick:get() * 0.03 end,
  behavior = {
    fill_color = { duration = 220, easing = "out_quad" },
    scale = { kind = "spring", mass = 1, damping = 14, stiffness = 300, epsilon = 0.002 },
  },
  ui.SdfShape { width = FACE_LG, height = FACE_LG, shape = "circle" },
}
prompt[#prompt + 1] = ui.Sdf {
  x = math.floor((PROMPT_W - FACE_LG) / 2),
  width = FACE_LG, height = FACE_LG,
  fill_color = TEXT,
  initial_letter,
}

-- `.user-widget.vertical .user-widget-label` — 20pt, weight 400, centred.
prompt[#prompt + 1] = ui.Text {
  y = FACE_LG + s(24),
  width = PROMPT_W,
  text = function()
    local user = users[chosen_user:get()]
    return user and user.label or ""
  end,
  font_size = s(22),
  font_weight = 400,
  horizontal_alignment = "center",
  color = TEXT,
}

-- The back button, left of the entry and vertically centred in the row, which
-- is where `.cancel-button` sits in `.login-dialog-button-box`.
do
  local node = round_button("cancel", "back", CANCEL, go_back)
  node.y = ROW_Y + math.floor((ROW_H - CANCEL) / 2)
  prompt[#prompt + 1] = node
end

-- The dots, as a field. A row of circles that do not touch, so it reads as a
-- count rather than a smear — and no font has to have `●` for it to.
local DOT = s(10)
local DOT_GAP = s(18)
-- As many as fit between the entry's leading padding and the arrow at its
-- trailing edge, so a long password fills the field rather than running out of
-- it. `.login-dialog-prompt-entry` reserves 2.5em on the trailing side.
local DOTS = math.max(1, math.floor((ENTRY_W - s(14) - s(40)) / DOT_GAP))
local dot_row = {
  x = ENTRY_X + s(14),
  y = ROW_Y + math.floor((ROW_H - DOT) / 2),
  width = DOTS * DOT_GAP,
  height = DOT,
  fill_color = TEXT,
}
for index = 1, DOTS do
  -- Size, and not `opacity`: the layers of a field compose into one shape
  -- before anything is painted, so a layer has no opacity of its own to turn
  -- down. What it has is a radius, and a circle of no radius is nothing.
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

prompt[#prompt + 1] = ui.Rect {
  x = ENTRY_X,
  y = ROW_Y + math.floor((ROW_H - ENTRY_H) / 2),
  width = ENTRY_W,
  height = ENTRY_H,
  radius = ENTRY_RADIUS,
  color = BUTTON,
  -- `%system_entry:focus` is a two-pixel ring at a fifth of the accent, drawn
  -- inside the entry. It always has the keys here, so it always has the ring —
  -- and the ring is where the screen answers back. A key thickens and brightens
  -- it and a spring lets it go; a refused password swings it red and shakes the
  -- whole row. Both are one number driven from Lua once and settled by the
  -- frame tick, so holding a key down costs no more than pressing it.
  border_width = function() return s(2) + kick:get() * s(2) end,
  border_color = function()
    if shake:get() > 0 then return ALERT end
    return kick:get() > 0 and ACCENT or ACCENT_RING
  end,
  translate_x = function() return shake:get() * s(14) end,
  behavior = {
    border_width = { kind = "spring", mass = 1, damping = 12, stiffness = 420,
                     epsilon = 0.01 },
    border_color = { duration = 260, easing = "out_quad" },
    -- Barely damped, so letting go of the offset is an oscillation rather than
    -- a slide: the row swings past centre a few times and settles. A shake is a
    -- spring nobody is holding.
    translate_x = { kind = "spring", mass = 1, damping = 5.5, stiffness = 560,
                    epsilon = 0.05 },
  },
}
prompt[#prompt + 1] = ui.Text {
  x = ENTRY_X + s(14),
  y = ROW_Y,
  width = ENTRY_W - s(28),
  height = ROW_H,
  text = function() return typed:get() == 0 and "Password" or "" end,
  font_size = s(15),
  vertical_alignment = "center",
  color = DIM,
  opacity = 0.65,
}
prompt[#prompt + 1] = ui.Sdf(dot_row)

-- `.next-button` sits in `.login-dialog-default-button-well`, inside the entry
-- at its trailing edge with `0.5em` to spare, and carries no background of its
-- own — the entry it stands in is its background.
do
  prompt[#prompt + 1] = icon("next", ICON, {
    x = ENTRY_X + ENTRY_W - s(8) - ICON,
    y = ROW_Y + math.floor((ROW_H - ICON) / 2),
    opacity = function() return typed:get() > 0 and 1 or 0.4 end,
    behavior = { opacity = { duration = 180, easing = "out_quad" } },
  })
end
prompt[#prompt + 1] = ui.MouseArea {
  x = ENTRY_X, y = ROW_Y, width = ENTRY_W, height = ROW_H,
  on_clicked = function() attempt() end,
}

-- `.login-dialog-message` — centred, dimmed, and holding 2.75em of height
-- whether or not it has anything to say, so nothing below it moves. GDM does
-- not colour a failed password red; it says so and leaves the screen alone.
prompt[#prompt + 1] = ui.Text {
  y = ROW_Y + ROW_H + s(9),
  width = PROMPT_W,
  height = s(44),
  text = function()
    if working:get() then return "Authenticating…" end
    return message:get()
  end,
  font_size = s(12),
  horizontal_alignment = "center",
  color = DIM,
}

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
  -- Two arguments, and they are the keysym and the text — not
  -- `(key, modifiers, text)`, which is what this used to say. Written that way
  -- the keysym landed in `key`, the text landed in `modifiers`, and `text` was
  -- always nil: every comparison against `"Return"` failed and every character
  -- was dropped. It looked exactly like a keyboard that was not reaching the
  -- screen at all, and it was a signature that had never matched.
  on_key_pressed = function(keysym, text)
    -- X11 keysyms. There is no name for them here, so they are named here.
    local RETURN, KP_ENTER = 0xff0d, 0xff8d
    local BACKSPACE, ESCAPE = 0xff08, 0xff1b
    local UP, DOWN, F1 = 0xff52, 0xff54, 0xffbe

    if working:get() then return end

    local function step_user(by)
      if #users == 0 then return end
      local next_user = ((chosen_user:get() - 1 + by) % #users) + 1
      write(chosen_user, next_user)
      morph_initial(next_user)
      if asking:get() then clear_password() end
    end

    if keysym == RETURN or keysym == KP_ENTER then
      if asking:get() then
        attempt()
      elseif #users > 0 then
        choose_user(chosen_user:get())
      end
    elseif keysym == BACKSPACE then
      backspace()
    elseif keysym == ESCAPE then
      if asking:get() then go_back() else clear_password() end
    -- The arrows and not Tab: Tab never arrives, because the runtime takes it
    -- to move keyboard focus between handlers before a configuration sees it.
    elseif keysym == UP then
      step_user(-1)
    elseif keysym == DOWN then
      step_user(1)
    elseif keysym == F1 then
      open_keyboard()
    elseif text and text ~= "" then
      -- A key struck on the list is not a key wasted: it picks the account that
      -- is already selected and becomes the first character of its password.
      -- Nothing on this screen should swallow a keystroke and do nothing.
      if not asking:get() then
        if #users == 0 then return end
        choose_user(chosen_user:get())
      end
      type_character(text)
    end
  end,
})

place(ui.Rect { width = W, height = H, color = INK })

--------------------------------------------------------------------------------
-- The frost.
--------------------------------------------------------------------------------

-- Three soft fields leaning one way and then the other, and the whole lot put
-- behind a blur. `softness` alone gives a shape with no edge; the blur on top is
-- what turns three coloured shapes into weather. A field has no resolution, so
-- the glow costs the same however large it is drawn.
--
-- `layer = { blur }` and not `backdrop_blur`: the second asks the *compositor*
-- to blur what is behind the surface, and behind a greeter there is nothing —
-- it owns the screen. This blurs a subtree of the surface's own drawing, which
-- is the only thing there is to frost.
local function cloud(index, home_x, home_y, radius, colour, reach)
  return ui.Sdf {
    x = function() return home_x + (drift:get() == 1 and reach or -reach) end,
    y = function() return home_y + (drift:get() == 1 and -reach or reach) end,
    width = radius * 2,
    height = radius * 2,
    fill_color = colour,
    opacity = 0.5,
    softness = radius * 0.8,
    behavior = {
      x = { duration = 11000 + index * 1700, easing = "in_out_sine" },
      y = { duration = 13000 - index * 1300, easing = "in_out_sine" },
    },
    ui.SdfShape { width = radius * 2, height = radius * 2, shape = "circle" },
  }
end

place(ui.Item {
  width = W,
  height = H,
  layer = { blur = s(90) },
  cloud(1, math.floor(W * 0.20), math.floor(H * 0.30), s(360), "#2b4c7e", s(120)),
  cloud(2, math.floor(W * 0.72), math.floor(H * 0.66), s(300), "#4a3a7a", s(100)),
  cloud(3, math.floor(W * 0.50), math.floor(H * 0.18), s(260), "#2a6a6a", s(90)),
})

place(ui.Timer {
  interval = 9000, ["repeat"] = true, running = true,
  on_triggered = function() write(drift, drift:get() == 1 and 0 or 1) end,
})

-- A sheet of glass over the weather. Barely there — a hundredth of white and a
-- hairline of it at the edge — because what makes it read as glass is the blur
-- behind it, not the sheet itself.
local function sheet(x, y, width, height, up, here)
  return ui.Rect {
    x = x - s(40), y = y - s(40),
    width = width + s(80), height = height + s(80),
    radius = s(28),
    color = "#ffffff0e",
    border_width = s(1),
    border_color = "#ffffff1c",
    visible = up,
    opacity = function() return here() and 1.0 or 0.0 end,
    scale = function() return here() and 1.0 or 0.94 end,
    translate_y = function() return shown:get() and -s(180) or 0 end,
      behavior = {
      opacity = { duration = 190, easing = "out_quad" },
      scale = { kind = "spring", mass = 1, damping = 19, stiffness = 170,
                epsilon = 0.002 },
      translate_y = { kind = "spring", mass = 1, damping = 20, stiffness = 190,
                      epsilon = 0.05 },
    },
  }
end

place(sheet(LIST_X, LIST_Y, LIST_W, LIST_H + s(56),
            function() return list_up:get() end,
            function() return not asking:get() end))
place(ui.Item(list_view))

--------------------------------------------------------------------------------
-- `.login-dialog-bottom-button-group`: 32px in from the corner, 16px apart.
--------------------------------------------------------------------------------

local BUTTON_SIZE = s(48)       -- 16px icon with `to_em(16px)` around it
local BUTTON_PAD = s(32)
local BUTTON_GAP = s(16)
-- Along the top. They used to sit in GDM's corner, bottom right — which is
-- exactly where the keyboard comes up, and a row of buttons under a keyboard is
-- a row of buttons nobody can reach.
local BUTTON_Y = BUTTON_PAD

local right = W - BUTTON_PAD
local function place_right(node)
  right = right - BUTTON_SIZE
  node.x = right
  node.y = BUTTON_Y
  place(node)
  right = right - BUTTON_GAP
end

-- Rightmost first, so they read left to right in the order written.
place_right(round_button("keyboard", "keyboard", BUTTON_SIZE, open_keyboard))
place_right(round_button("power", "power", BUTTON_SIZE, function() power("PowerOff") end))
place_right(round_button("restart", "restart", BUTTON_SIZE, function() power("Reboot") end))
place_right(round_button("suspend", "suspend", BUTTON_SIZE, function() power("Suspend") end))
if #available > 1 then
  place_right(round_button("options", "options", BUTTON_SIZE, function()
    if not working:get() then write(chosen_session, chosen_session:get() % #available + 1) end
  end))
end

-- Which session that button is currently on. GDM keeps this in a popup; a
-- greeter with no popup has to say it somewhere, and saying nothing at all
-- leaves the button meaningless.
place(ui.Text {
  x = s(32),
  y = BUTTON_Y,
  width = right - s(32),
  height = BUTTON_SIZE,
  text = function()
    local entry = available[chosen_session:get()]
    return entry and entry.name or "no sessions on this machine"
  end,
  font_size = s(12),
  font_weight = 700,
  vertical_alignment = "center",
  horizontal_alignment = "right",
  elide = "right",
  color = DIM,
})

place(sheet(PROMPT_X, PROMPT_Y, PROMPT_W, ROW_Y + ROW_H + s(60),
            function() return prompt_up:get() end,
            function() return asking:get() end))
place(ui.Item(prompt))
-- Letting go. Each is one shot: the signal is set on the event and dropped here,
-- and the springs above turn the drop into the movement.
kick_rest = ui.Timer {
  interval = 70, ["repeat"] = false, running = false,
  on_triggered = function() write(kick, 0) end,
}
shake_rest = ui.Timer {
  interval = 60, ["repeat"] = false, running = false,
  on_triggered = function() write(shake, 0) end,
}
unmount_rest = ui.Timer {
  interval = 460, ["repeat"] = false, running = false,
  on_triggered = function()
    write(list_up, not asking:get())
    write(prompt_up, asking:get())
  end,
}

--------------------------------------------------------------------------------
-- The keyboard.
--------------------------------------------------------------------------------

-- evdev, because that is what the board speaks — it hands over a key and
-- whether it was shifted, and what that means is this screen's business. On a
-- surface of its own the same call becomes a virtual-keyboard press; here it is
-- simply the same three functions a physical key already reaches, so the dots,
-- the ring and the avatar answer a tap exactly as they answer a keystroke.
local EVDEV_BACKSPACE, EVDEV_ENTER = 14, 28

local board_panel, board_height = board.build {
  width = W,
  key = function(code, _shifted, label)
    if working:get() then return end
    if code == EVDEV_BACKSPACE then
      backspace()
    elseif code == EVDEV_ENTER then
      if asking:get() then
        attempt()
      elseif #users > 0 then
        choose_user(chosen_user:get())
      end
    elseif label and label ~= "" then
      if not asking:get() then
        if #users == 0 then return end
        choose_user(chosen_user:get())
      end
      type_character(label)
    end
  end,
}

place(ui.Item {
  x = 0,
  y = H - board_height,
  width = W,
  height = board_height,
  visible = function() return shown:get() end,
  -- Comes up from under the edge of the screen, which is where a keyboard
  -- comes from.
  opacity = function() return shown:get() and 1.0 or 0.0 end,
  translate_y = function() return shown:get() and 0 or board_height end,
  behavior = {
    opacity = { duration = 160, easing = "out_quad" },
    translate_y = { kind = "spring", mass = 1, damping = 20, stiffness = 200,
                    epsilon = 0.05 },
  },
  board_panel,
})

place(kick_rest)
place(shake_rest)
place(unmount_rest)

place(face_swap)

ui.Item(tree)
