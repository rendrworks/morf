-- A login screen, and a lock screen: the same file, told which.
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
-- Told `lock`, the same file asks morf for a session lock instead of a layer —
-- `morf.surface.session_lock = true`, and morf renders whichever surface a
-- file asks for — then asks only the person who is logged in, through PAM, and
-- the right password, or a finger on the reader, lifts the lock:
--
--     morf examples/greeter.lua -- lock
--
-- To look at the lock in a window, where nothing can be held, add `window`:
--
--     cage -- morf examples/greeter.lua -- lock window
--
-- Two things that bite. Under greetd it runs as user `greeter`, not as you —
-- every font, asset and path here must be readable by that user, which is the
-- usual way these fail. And `GREETD_SOCK` exists only in the environment
-- greetd provides, so run it nested and the password field will say so rather
-- than pretending.
--
-- ON THE LOOK. Black, white, one monospace face, one green. Everything in a
-- narrow column down the middle: the time, a rule, the accounts as rows with
-- a square face and a name, the chosen one in a one-pixel box; then one row
-- and a boxed field for the password with a caret in it. The controls are
-- small boxed labels in the top corners — KEYBOARD, SESSION, A11Y, POWER —
-- each also a function key. Nothing glows and nothing is rounded past a
-- corner or two. What GDM has learned to do is all here, set small.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local io = require("morf.io")
-- The keyboard. Not a keyboard drawn to look like it — the same file
-- `examples/keyboard.lua` puts in a surface of its own. It cannot have one here
-- because a kiosk compositor shows a single window, so a login screen's second
-- surface is never seen; drawn into this one it is the same board either way.
local board = require("lib.board")

-- One file, two doors, and the word after the file's `--` says which. `lock`
-- is the lock screen, and holds the session unless `window` follows, which
-- is how the lock is looked at in a window. Anything else is the greeter,
-- because a greeter is the door that cannot lock anybody out.
local LOCKING = morf.operands[1] == "lock"
local HELD = LOCKING and morf.operands[2] ~= "window"

local screen = morf.screens[1]
local W = (screen and screen.width) or 1920
local H = (screen and screen.height) or 1080

morf.surface.width = W
morf.surface.height = H
morf.surface.anchors = { top = true, left = true, right = true, bottom = true }
morf.surface.layer = "overlay"
-- The greeter is the only thing on screen and the password has to go somewhere.
morf.surface.keyboard_focus = "exclusive"
-- A lock is not a layer: one surface per output under the session-lock
-- protocol, held until this file says otherwise. morf renders whichever the
-- file asks for.
morf.surface.session_lock = HELD

-- One number, everything in proportion. A phone is not a small desktop, it is
-- a different shape: a narrow screen takes its scale from its width, because
-- width is what everything has to fit inside; a wide one from the smaller of
-- its two proportions against a 1080p desktop.
local NARROW = W < 700
local SCALE = NARROW and math.max(0.9, math.min(1.4, W / 400))
  or math.max(0.75, math.min(1.8, math.min(W / 1920, H / 1080)))
local function s(n) return math.floor(n * SCALE) end

-- One face, and one size for a line of it. `CH` is a line's height, and the
-- whole screen is laid out in lines.
local MONO = "Iosevka Nerd Font Mono"
local BODY = NARROW and s(14) or s(15)
local CH = math.floor(BODY * 1.6)
local MARGIN = NARROW and s(16) or s(32)
local EM = math.floor(BODY * 0.6)   -- a character cell, roughly
local COL_W = math.min(NARROW and (W - MARGIN * 2) or s(420), W - MARGIN * 2)
local COL_X = math.floor((W - COL_W) / 2)

--------------------------------------------------------------------------------
-- The theme.
--------------------------------------------------------------------------------

-- What the accessibility menu can switch on. `morf.prefers` says what the
-- desktop asked for; this is what somebody standing at this screen asks for,
-- and the theme reads both.
local a11y = morf.state { large_text = false, high_contrast = false }

-- Two colours per scheme and an accent. Everything else is derived on every
-- read from whatever it touches, so the scheme, the contrast and the accent
-- can change under it and every surface follows.
local theme = morf.theme({
  dark_ink = "#000000",
  dark_text = "#f2f2f2",
  light_ink = "#ffffff",
  light_text = "#111111",
  dark_alert = "#ff5d5d",
  light_alert = "#c81e1e",
  fallback_accent = "#3ddc84",
  theme = "dark",

  light = function(t)
    local scheme = morf.prefers.color_scheme
    if scheme == "light" or scheme == "dark" then return scheme == "light" end
    return t.theme == "light"
  end,
  contrast = function()
    return a11y.high_contrast or morf.prefers.contrast == "high"
  end,
  ink = function(t)
    if t.contrast then return morf.color(t.light and "white" or "black") end
    return t.light and t.light_ink or t.dark_ink
  end,
  text = function(t)
    if t.contrast then return morf.color(t.light and "black" or "white") end
    return t.light and t.light_text or t.dark_text
  end,
  -- Grey, and greyer: the text, thinned.
  dim = function(t) return t.text:alpha(t.contrast and 1.0 or 0.55) end,
  faint = function(t) return t.text:alpha(t.contrast and 0.8 or 0.3) end,
  -- The desktop's accent if it chose one, else the green.
  accent = function(t) return morf.prefers.accent_color or t.fallback_accent end,
  -- A fill for a hovered row, and the one-pixel lines everything is boxed in.
  glass = function(t) return t.text:alpha(t.contrast and 0.25 or 0.06) end,
  glass_hot = function(t) return t.text:alpha(t.contrast and 0.35 or 0.1) end,
  edge = function(t) return t.text:alpha(t.contrast and 1.0 or 0.18) end,
  edge_hot = function(t) return t.text:alpha(t.contrast and 1.0 or 0.5) end,
  alert = function(t) return t.light and t.light_alert or t.dark_alert end,
})

local function write(signal, value)
  local ok, error = signal:set(value)
  assert(ok, error)
end

--------------------------------------------------------------------------------
-- Accounts and sessions.
--------------------------------------------------------------------------------

--- The picture an account keeps of itself, if it keeps one: what
--- AccountsService holds for it, else the `.face` in its home, which is
--- where the desktops that predate AccountsService looked.
local function face_of(name)
  for _, path in ipairs {
    "/var/lib/AccountsService/icons/" .. name,
    "/home/" .. name .. "/.face",
    "/home/" .. name .. "/.face.icon",
  } do
    if io.file_view { path = path }:exists() then return path end
  end
  return nil
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
        found[#found + 1] = {
          name = name,
          label = label,
          initial = label:sub(1, 1):upper(),
          face = face_of(name),
        }
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
-- A lock asks one person: whoever is logged in. A name the list has not got
-- is still that person, made up on the spot, because a lock that offers other
-- accounts is a switch-user dialog, and this is not one.
if LOCKING then
  local me = core.env("USER") or core.env("LOGNAME") or ""
  local mine
  for _, user in ipairs(users) do
    if user.name == me then mine = user end
  end
  if me ~= "" and not mine then
    mine = { name = me, label = me, initial = me:sub(1, 1):upper(), face = face_of(me) }
  end
  if mine then users = { mine } end
end
-- A lock starts no session, so there is no list of them to read.
local available = LOCKING and {} or sessions()

-- Two PAM services, because a lock has two doors and PAM walks one stack at
-- a time. The password goes to the password stack — `morf-lock` when the
-- machine has written one, else `system-auth`, else `login` — and gets a
-- verdict in the time a hash takes. The machine's interactive stack, `login`,
-- is where it puts a fingerprint reader, a face, a key: that one is listened
-- to the whole time the lock is up, in a conversation of its own, and
-- whatever it says ("place your finger on the reader") is shown. Not one
-- stack for both, because a `login` stack puts `pam_fprintd.so` ahead of the
-- password and then waits for a finger before it will hear a word.
local PASSWORD_SERVICE = "login"
for _, service in ipairs({ "morf-lock", "system-auth" }) do
  if io.file_view { path = "/etc/pam.d/" .. service }:exists() then
    PASSWORD_SERVICE = service
    break
  end
end
local FINGER_SERVICE = PASSWORD_SERVICE ~= "login"
  and io.file_view { path = "/etc/pam.d/login" }:exists() and "login" or nil

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
local message = morf.signal("greeter.message", "")
-- A key struck, and a password refused. Both are set to one and dropped back to
-- zero a moment later; the springs on the properties they drive do the rest.
local kick = morf.signal("greeter.kick", 0)
local shake = morf.signal("greeter.shake", 0)
local kick_rest, shake_rest, unmount_rest
-- `asking` flips the instant a choice is made, so the fade has something to
-- run against; these say which of the two screens is still *mounted*.
local asking = morf.signal("greeter.asking", false)
local list_up = morf.signal("greeter.up.list", true)
local prompt_up = morf.signal("greeter.up.prompt", false)
-- Which of the corner menus is open: "", "session", "power" or "a11y".
local menu = morf.signal("greeter.menu", "")
-- The session starting; the screen having been left alone; and an account
-- that is not on the list, being asked for by name.
local launching = morf.signal("greeter.launching", false)
local dozing = morf.signal("greeter.dozing", false)
local breath = morf.signal("greeter.breath", 0)
local naming = morf.signal("greeter.naming", false)
local custom_name = morf.signal("greeter.custom", "")
local idle_rest
-- What the reader's stack last said; and the conversation with it, which
-- outlives any one password attempt.
local finger = morf.signal("greeter.finger", "")
local listen, stop_listening, unlocked
local listen_again, listening
-- The login under greetd, as a conversation that outlives a keystroke. It
-- starts when an account is chosen, so greetd's stack — and the reader in it
-- — is already listening when the prompt appears; the password is handed
-- over when greetd asks for it, or held until it does.
local begin_login, end_login
local toggle_menu
local login, login_again
local held_password
local wanting_secret = false
local started = false

--- Any touch of the screen wakes it and puts the idle clock back.
local function wake()
  if dozing:get() then write(dozing, false) end
  if idle_rest then idle_rest.running = true end
end

--- The account being asked for: one from the list, or the name typed in.
local function account_name()
  local user = users[chosen_user:get()]
  return user and user.name or custom_name:get()
end

local function say(text, bad)
  write(message, text)
  write(alarmed, bad or false)
end

local function clear_password()
  password = ""
  write(typed, 0)
end

local function close_menus() write(menu, "") end

--------------------------------------------------------------------------------
-- Authentication.
--------------------------------------------------------------------------------

--- Runs one login attempt.
local function attempt()
  if working:get() or account_name() == "" then return end
  if not LOCKING and #available == 0 then return end
  write(working, true)
  say("")

  if LOCKING then
    -- The password path: credentials in, verdict back, and on a yes the lock
    -- is lifted the way it was asked for, by this file.
    morf.pam.authenticate(PASSWORD_SERVICE, account_name(), password, function(ok, why)
      if ok then
        unlocked()
        return
      end
      say(why or "That did not work", true)
      clear_password()
      write(working, false)
      write(shake, 1)
      if shake_rest then shake_rest.running = true end
    end)
    return
  end

  -- greetd owns the conversation, and it is already under way: an account
  -- was chosen, so its stack has been asked, and whatever it wanted first —
  -- a finger, a word — has been on screen since. If it has asked for the
  -- password, here it is. If it has not, it is still with the reader, and the
  -- password waits for the question rather than the other way round.
  if not login then begin_login() end
  if not login then
    say("No greetd to ask — nested, so Return does nothing", true)
    write(working, false)
    write(shake, 1)
    if shake_rest then shake_rest.running = true end
    return
  end
  if wanting_secret then
    wanting_secret = false
    login:respond(password)
  else
    held_password = password
  end
end

--- Asks logind to suspend, reboot or power off.
---
--- The `false` is logind's `interactive` flag: a greeter has nobody to prompt
--- for a polkit password, so an interactive check would hang waiting for an
--- agent that is not running.
local function power(method)
  close_menus()
  local ok, manager = pcall(io.dbus.proxy, "system", "org.freedesktop.login1",
                            "/org/freedesktop/login1", "org.freedesktop.login1.Manager")
  if not (ok and manager) then
    say("Cannot reach logind", true)
    return
  end
  local called, err = pcall(manager.call_with, manager, method, false)
  if not called then say(tostring(err), true) end
end

--- The lock lifts. The face grows to fill the screen while the process ends,
--- the way a session arrives out of the account rather than after a cut.
unlocked = function()
  stop_listening()
  say("")
  write(launching, true)
  morf.surface.session_lock = false
end

stop_listening = function()
  if listening then
    listening:cancel()
    listening = nil
  end
end

--- Listens to the reader's stack for as long as the lock is up: a finger, a
--- face, a key — whatever the machine put in `login`. Its words go to the
--- screen; its yes lifts the lock; its no starts it listening again, because
--- a reader that gave up is not a reader that was refused.
listen = function()
  if not (LOCKING and FINGER_SERVICE) or listening or account_name() == "" then return end
  local session = morf.pam.session(FINGER_SERVICE, account_name())
  listening = session
  session:on_message(function(m)
    if listening ~= session then return end
    if m.kind == "info" or m.kind == "error" then
      write(finger, m.text)
      if asking:get() and not working:get() then say(m.text, m.kind == "error") end
    elseif m.kind == "prompt" then
      -- The reader gave up and the stack fell through to a password
      -- question. The password has a stack of its own; this conversation
      -- is over, and the next one starts at the reader again.
      session:cancel()
    elseif m.kind == "finished" then
      listening = nil
      if m.ok then
        unlocked()
      elseif not launching:get() then
        listen_again.running = true
      end
    end
  end)
end

listen_again = ui.Timer {
  interval = 1500, ["repeat"] = false, running = false,
  on_triggered = listen,
}

end_login = function()
  if login then
    login:cancel()
    login = nil
  end
  wanting_secret = false
  held_password = nil
  started = false
end

--- Opens the login with greetd for the account on the prompt. Everything
--- greetd's stack says comes through here: a module's words go on screen and
--- are acknowledged; a request for the password is answered from the one
--- typed, or waited on; a yes starts the chosen session; a no says why and
--- opens the login again, so the reader is listening for the next try.
begin_login = function()
  if LOCKING or login or account_name() == "" then return end
  local session = morf.greetd.converse(account_name())
  login = session
  wanting_secret = false
  started = false
  session:on_message(function(m)
    if login ~= session then return end
    if m.kind == "auth" then
      if m.auth_type == "secret" or m.auth_type == "visible" then
        if held_password then
          local answer = held_password
          held_password = nil
          session:respond(answer)
        else
          wanting_secret = true
        end
      else
        write(finger, m.text)
        if not working:get() then say(m.text, m.auth_type == "error") end
        session:respond(nil)
      end
    elseif m.kind == "success" then
      if started then
        -- greetd takes it from here: this process is about to be replaced by
        -- the session it just asked for. Until it is, the face grows to fill
        -- the screen, so the session arrives out of the account rather than
        -- after a cut to black.
        write(launching, true)
        return
      end
      local wanted = available[chosen_session:get()]
      if not wanted then
        say("No session to start", true)
        write(working, false)
        return
      end
      started = true
      say("Starting " .. wanted.name)
      session:start(wanted.command, wanted.environment)
    elseif m.kind == "error" then
      -- greetd is done with this login. Say why, and open the next.
      say(m.text ~= "" and m.text or "That did not work", true)
      session:cancel()
      login = nil
      held_password = nil
      wanting_secret = false
      clear_password()
      write(working, false)
      write(shake, 1)
      if shake_rest then shake_rest.running = true end
      if asking:get() then login_again.running = true end
    elseif m.kind == "failed" then
      -- Nested inside a session there is no `$GREETD_SOCK`, so there is
      -- nothing to authenticate against and Return can do nothing. The
      -- password is left where it is: nobody typed it in order to have it
      -- thrown away, and the screen is otherwise perfectly usable to look at
      -- and to type in.
      login = nil
      if held_password then
        held_password = nil
        say("No greetd to ask — nested, so Return does nothing", true)
        write(shake, 1)
        if shake_rest then shake_rest.running = true end
      end
      write(working, false)
    end
  end)
end

login_again = ui.Timer {
  interval = 800, ["repeat"] = false, running = false,
  on_triggered = begin_login,
}

--------------------------------------------------------------------------------
-- Moving between the two screens.
--------------------------------------------------------------------------------

--- Crosses from the list to the prompt or back: the arriving screen is mounted
--- at once so it has somewhere to fade in from, and the leaving one is taken
--- down only after it has finished leaving.
local function cross(to_prompt)
  write(asking, to_prompt)
  write(to_prompt and prompt_up or list_up, true)
  if unmount_rest then unmount_rest.running = true end
end

local function choose_user(index)
  if working:get() then return end
  close_menus()
  write(chosen_user, index)
  clear_password()
  write(naming, false)
  say("")
  cross(true)
  begin_login()
end

--- An account the list does not show: the prompt asks for a name first.
local function not_listed()
  if working:get() then return end
  close_menus()
  write(chosen_user, 0)
  write(custom_name, "")
  write(naming, true)
  clear_password()
  say("")
  cross(true)
end

--- Return on the prompt: a name typed goes on to its password; a password
--- typed is tried.
local function confirm()
  if naming:get() then
    if custom_name:get() ~= "" then
      write(naming, false)
      say("")
      begin_login()
    end
    return
  end
  attempt()
end

local function go_back()
  if working:get() then return end
  close_menus()
  end_login()
  clear_password()
  write(naming, false)
  if chosen_user:get() < 1 and #users > 0 then write(chosen_user, 1) end
  say("")
  cross(false)
end

--------------------------------------------------------------------------------
-- Typing, and the keyboard.
--------------------------------------------------------------------------------

--- Shows and hides the board.
local shown = morf.signal("greeter.board", false)
local function open_keyboard()
  write(shown, not shown:get())
end

-- `-- --numbers-only` puts up a keypad rather than a keyboard: three keys
-- across instead of ten, so the same width buys targets three times the size.
local numeric = morf.signal("greeter.numeric", core.options["numbers-only"] == true)

local CAPACITY = 64

local function type_character(character)
  if working:get() then return end
  if naming:get() then
    if #custom_name:get() < CAPACITY then write(custom_name, custom_name:get() .. character) end
    return
  end
  if #password >= CAPACITY then return end
  password = password .. character
  write(typed, #password)
  -- Every key nudges the pill, and the spring settles it. Written here rather
  -- than in the key handler so a character arriving from the on-screen keyboard
  -- looks the same as one from a real one.
  write(kick, 1)
  if kick_rest then kick_rest.running = true end
  if alarmed:get() then say("") end
end

local function backspace()
  if working:get() then return end
  if naming:get() then
    write(custom_name, custom_name:get():sub(1, -2))
    return
  end
  password = password:sub(1, -2)
  write(typed, #password)
end

--------------------------------------------------------------------------------
-- Pieces.
--------------------------------------------------------------------------------

--- An SVG outline as a field: exact at any size, and it composes.
local function icon(name, box, extra)
  local field = {
    width = box,
    height = box,
    fill_color = function() return theme.text end,
    ui.SdfShape {
      width = box,
      height = box,
      source = core.shell_path("assets/icon-" .. name .. ".svg"),
    },
  }
  for key, value in pairs(extra or {}) do field[key] = value end
  return ui.Sdf(field)
end

--- Large text is everything a fifth larger, about its own centre.
local function magnify() return a11y.large_text and 1.2 or 1.0 end

--- A line of text, in the face and size everything else is in.
local function line(extra)
  local node = {
    height = CH,
    font_family = MONO,
    font_size = BODY,
    vertical_alignment = "center",
    color = function() return theme.text end,
  }
  for key, value in pairs(extra or {}) do node[key] = value end
  return ui.Text(node)
end

--- A small boxed label that can be clicked: the corners' controls. `lit`
--- fills the box, which is how a thing that is on is shown here.
local function word(id, text, x, y, on_tap, lit, anchors)
  local hot = morf.signal("greeter.word." .. id, false)
  local width = #text * EM + EM * 2
  return ui.Item {
    x = x, y = y, width = width, height = CH,
    anchors = anchors,
    opacity = function() return dozing:get() and 0.0 or 1.0 end,
    behavior = { opacity = { duration = 600, easing = "in_out_quad" } },
    ui.Rect {
      anchors = { fill = true },
      radius = s(4),
      color = function()
        if lit and lit() then return theme.text end
        return hot:get() and theme.glass_hot or theme.glass:alpha(0)
      end,
      border_width = math.max(1, s(1)),
      border_color = function()
        if lit and lit() then return theme.text end
        return hot:get() and theme.edge_hot or theme.edge
      end,
      behavior = {
        color = { duration = 100, easing = "out_quad" },
        border_color = { duration = 100, easing = "out_quad" },
      },
    },
    line {
      x = EM, width = width - EM * 2,
      text = text,
      font_size = math.floor(BODY * 0.8),
      letter_spacing = s(1),
      horizontal_alignment = "center",
      color = function()
        if lit and lit() then return theme.ink end
        return hot:get() and theme.text or theme.dim
      end,
    },
    ui.MouseArea {
      cursor = "pointer",
      anchors = { fill = true },
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = function()
        wake()
        on_tap()
      end,
    },
  }
end

--- Large text is everything a fifth larger, about the top left.
local function magnify() return a11y.large_text and 1.2 or 1.0 end

--------------------------------------------------------------------------------
-- The tree.
--------------------------------------------------------------------------------

-- Children go into a list rather than inline: a `table.unpack` in the middle of
-- a table constructor keeps only its first value.
local tree = { anchors = { fill = true } }
local function place(node) tree[#tree + 1] = node end

-- The corner buttons, and where things sit. On a phone the top row is
-- thumb-sized and everything else keeps to the top half, because the bottom
-- half belongs to the keyboard and to the hand.
local BUTTON = math.max(s(44), NARROW and 44 or 0)
local PAD = NARROW and s(16) or s(28)
local GAP = s(12)
local TOP_BAR = PAD + BUTTON + s(16)

-- First, so it is at the bottom of the stack: a hit test returns the topmost
-- MouseArea, and this one covers the screen. Key handlers are collected by
-- walking the tree, so being underneath costs it nothing.
place(ui.MouseArea {
  anchors = { fill = true },
  on_clicked = function()
    wake()
    close_menus()
  end,
  on_key_pressed = function(keysym, text)
    local RETURN, KP_ENTER = 0xff0d, 0xff8d
    local BACKSPACE, ESCAPE = 0xff08, 0xff1b
    local LEFT, UP, RIGHT, DOWN, F1 = 0xff51, 0xff52, 0xff53, 0xff54, 0xffbe

    wake()
    if working:get() then return end

    local function step_user(by)
      if #users == 0 then return end
      local next_user = ((chosen_user:get() - 1 + by) % #users) + 1
      write(chosen_user, next_user)
      if asking:get() then clear_password() end
    end

    if keysym == RETURN or keysym == KP_ENTER then
      if asking:get() then
        confirm()
      elseif chosen_user:get() < 1 then
        not_listed()
      else
        choose_user(chosen_user:get())
      end
    elseif keysym == BACKSPACE then
      backspace()
    elseif keysym == ESCAPE then
      if menu:get() ~= "" then
        close_menus()
      elseif asking:get() then
        go_back()
      else
        clear_password()
      end
    elseif keysym == LEFT or keysym == UP then
      step_user(-1)
    elseif keysym == RIGHT or keysym == DOWN then
      step_user(1)
    elseif keysym == F1 then
      open_keyboard()
    elseif keysym == 0xffbf then
      if not LOCKING then toggle_menu("session") end
    elseif keysym == 0xffc0 then
      toggle_menu("a11y")
    elseif keysym == 0xffc1 then
      toggle_menu("power")
    elseif text and text ~= "" then
      -- A key struck on the list is not a key wasted: it picks the account
      -- that is already selected and becomes the first character of its
      -- password.
      if not asking:get() then
        if chosen_user:get() < 1 then not_listed() else choose_user(chosen_user:get()) end
      end
      type_character(text)
    end
  end,
  on_position_changed = function() wake() end,
})

--------------------------------------------------------------------------------
-- The room: the ink, and three lights moving in it.
--------------------------------------------------------------------------------

place(ui.Rect {
  anchors = { fill = true },
  color = function() return theme.ink end,
})

--------------------------------------------------------------------------------
-- The column: the clock, a rule, the accounts.
--------------------------------------------------------------------------------

local clock = core.system_clock { precision = "seconds" }
local hostname = (io.file("/etc/hostname"):read() or "localhost"):match("^%s*(%S*)")
if hostname == "" then hostname = "localhost" end

local TOP = CH + s(16)
local CLOCK_SIZE = NARROW and s(56) or s(76)
local CLOCK_Y = NARROW and TOP + s(24) or math.floor(H * 0.2)

-- Making room for the keyboard. When the board is up the clock shrinks
-- towards the top and everything under it moves up by what the board needs,
-- as far as the top allows; when it goes, everything comes back. The two
-- ends are filled in once the column is laid out, and the board's height
-- once it is built; a binding reads them when it is asked, not now.
local board_height = 0
local LIST_END, PROMPT_END = 0, 0
local CLOCK_SHRINK = math.floor(CLOCK_SIZE * 0.3)
local function room()
  if not shown:get() then return 0, 0 end
  local bottom = (asking:get() and PROMPT_END or LIST_END) + s(20)
  local needed = math.max(0, bottom - (H - board_height))
  local top_room = math.max(0, CLOCK_Y - TOP - s(8))
  local upper = math.min(math.max(0, needed - CLOCK_SHRINK), top_room)
  return upper, upper + CLOCK_SHRINK
end
local function upper_room() local upper = room() return -upper end
local function lower_room() local _, lower = room() return -lower end
local squeeze = { kind = "spring", mass = 1, damping = 20, stiffness = 170, epsilon = 0.05 }

local clock_morph = morf.signal("greeter.clock.morph", 0)
local shown_time = clock:format("%H:%M")
local clock_face
clock_face = ui.Text {
  x = COL_X, y = CLOCK_Y, width = COL_W,
  translate_y = upper_room,
  scale = function() return shown:get() and 0.7 or 1.0 end,
  transform_origin_y = 0,
  text = shown_time,
  morph_to = shown_time,
  morph_progress = function() return clock_morph:get() end,
  font_family = MONO,
  font_size = CLOCK_SIZE,
  font_weight = 500,
  letter_spacing = -math.floor(CLOCK_SIZE * 0.03),
  line_height = 1.0,
  horizontal_alignment = "center",
  color = function() return theme.text end,
  behavior = {
    translate_y = squeeze,
    scale = { kind = "spring", mass = 1, damping = 20, stiffness = 170, epsilon = 0.002 },
    morph_progress = {
      duration = 500, easing = "in_out_cubic",
      on_finished = function()
        if clock_morph:get() == 1 then
          clock_face.text = clock_face.morph_to
          write(clock_morph, 0)
        end
      end,
    },
  },
}
place(clock_face)
place(ui.Timer {
  interval = 250, ["repeat"] = true, running = true,
  on_triggered = function()
    local now = clock:format("%H:%M")
    if now ~= shown_time and clock_morph:get() == 0 then
      shown_time = now
      clock_face.morph_to = now
      write(clock_morph, 1)
    end
  end,
})
place(line {
  x = COL_X, y = CLOCK_Y + CLOCK_SIZE + s(4), width = COL_W,
  text = function() return clock:format("%A, %-d %B") .. "  ·  " .. hostname end,
  horizontal_alignment = "center",
  color = function() return theme.dim end,
  translate_y = lower_room,
  behavior = { translate_y = squeeze },
})

local RULE_Y = CLOCK_Y + CLOCK_SIZE + CH + s(24)
place(ui.Rect {
  x = COL_X, y = RULE_Y, width = COL_W, height = math.max(1, s(1)),
  color = function() return theme.edge end,
  opacity = function() return dozing:get() and 0.0 or 1.0 end,
  translate_y = lower_room,
  behavior = { opacity = { duration = 600, easing = "in_out_quad" }, translate_y = squeeze },
})

-- The caret's blink, shared by every caret on the screen.
local blink = morf.signal("greeter.blink", true)
place(ui.Timer {
  interval = 530, ["repeat"] = true, running = true,
  on_triggered = function() write(blink, not blink:get()) end,
})

--- A small square face: the account's picture, or its initial on a tile.
local TILE = s(32)
local function tile(user, extra)
  local node = {
    width = TILE, height = TILE,
    ui.Rect {
      anchors = { fill = true },
      radius = s(6),
      color = function() return theme.glass_hot end,
      border_width = math.max(1, s(1)),
      border_color = function() return theme.edge end,
    },
  }
  if user and user.face then
    node[#node + 1] = ui.ClipRect {
      anchors = { fill = true }, radius = s(6), color = "transparent",
      ui.Image { anchors = { fill = true }, source = user.face, fill_mode = "preserve_aspect_crop" },
    }
  else
    node[#node + 1] = ui.Text {
      anchors = { fill = true },
      text = user and user.initial or "?",
      font_family = MONO,
      font_size = math.floor(TILE * 0.5),
      font_weight = 500,
      horizontal_alignment = "center",
      vertical_alignment = "center",
      color = function() return theme.text end,
    }
  end
  for key, value in pairs(extra or {}) do node[key] = value end
  return ui.Item(node)
end

local LIST_Y = RULE_Y + s(24)
local ROW_H = s(52)
local COUNT = #users

local list = {
  anchors = { fill = true },
  visible = function() return list_up:get() end,
  opacity = function() return (asking:get() or dozing:get()) and 0.0 or 1.0 end,
  translate_y = function() return (asking:get() and -s(8) or 0) + lower_room() end,
  scale = magnify,
  transform_origin_y = LIST_Y / H,
  behavior = {
    opacity = { duration = 200, easing = "out_quad" },
    translate_y = squeeze,
  },
}
list[#list + 1] = line {
  x = COL_X, y = LIST_Y, width = COL_W,
  text = LOCKING and "LOCKED" or "ACCOUNTS",
  font_size = math.floor(BODY * 0.8),
  letter_spacing = s(2),
  color = function() return theme.faint end,
}
for index, user in ipairs(users) do
  local hot = morf.signal("greeter.row." .. index, false)
  local function live() return hot:get() or chosen_user:get() == index end
  local y = LIST_Y + CH + (index - 1) * (ROW_H + s(8))
  list[#list + 1] = ui.Rect {
    x = COL_X, y = y, width = COL_W, height = ROW_H,
    radius = s(6),
    color = function() return hot:get() and theme.glass or theme.glass:alpha(0) end,
    border_width = math.max(1, s(1)),
    border_color = function() return live() and theme.edge_hot or theme.edge end,
    behavior = {
      color = { duration = 100, easing = "out_quad" },
      border_color = { duration = 120, easing = "out_quad" },
    },
  }
  list[#list + 1] = tile(user, { x = COL_X + s(10), y = y + math.floor((ROW_H - TILE) / 2) })
  list[#list + 1] = line {
    x = COL_X + s(10) + TILE + s(14), y = y + math.floor((ROW_H - CH) / 2),
    width = COL_W - TILE - s(60),
    text = user.label,
    elide = "right",
    color = function() return live() and theme.text or theme.dim end,
    behavior = { color = { duration = 120, easing = "out_quad" } },
  }
  list[#list + 1] = line {
    x = COL_X + COL_W - s(60), y = y + math.floor((ROW_H - CH) / 2), width = s(48),
    text = "↵",
    horizontal_alignment = "right",
    color = function() return theme.faint end,
    opacity = function() return live() and 1.0 or 0.0 end,
    behavior = { opacity = { duration = 120, easing = "out_quad" } },
  }
  list[#list + 1] = ui.MouseArea {
    cursor = "pointer",
    x = COL_X, y = y, width = COL_W, height = ROW_H,
    on_entered = function()
      write(hot, true)
      if not working:get() then write(chosen_user, index) end
    end,
    on_exited = function() write(hot, false) end,
    on_clicked = function()
      wake()
      choose_user(index)
    end,
  }
end
local LIST_BOTTOM = LIST_Y + CH + math.max(COUNT, 1) * (ROW_H + s(8))
LIST_END = LIST_BOTTOM + s(4) + CH
if COUNT == 0 then
  list[#list + 1] = line {
    x = COL_X, y = LIST_Y + CH, width = COL_W, height = ROW_H,
    text = "No accounts on this machine",
    color = function() return theme.dim end,
  }
end
do
  local hot = morf.signal("greeter.notlisted", false)
  list[#list + 1] = line {
    x = COL_X, y = LIST_BOTTOM + s(4), width = COL_W,
    text = function()
      if LOCKING then return finger:get() end
      return "+ Not listed"
    end,
    horizontal_alignment = LOCKING and "center" or "left",
    color = function() return (hot:get() and not LOCKING) and theme.text or theme.faint end,
    behavior = { color = { duration = 120, easing = "out_quad" } },
  }
  if not LOCKING then
    list[#list + 1] = ui.MouseArea {
      cursor = "pointer",
      x = COL_X, y = LIST_BOTTOM + s(4), width = s(160), height = CH,
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = function()
        wake()
        not_listed()
      end,
    }
  end
end
place(ui.Item(list))

--------------------------------------------------------------------------------
-- The prompt: one row, one field.
--------------------------------------------------------------------------------

local PROMPT_Y = LIST_Y
local FIELD_Y = PROMPT_Y + CH + ROW_H + s(16)
local FIELD_H = s(48)
PROMPT_END = FIELD_Y + FIELD_H + s(10) + CH * 2 + s(8)

local function chosen() return users[chosen_user:get()] end

local prompt = {
  anchors = { fill = true },
  visible = function() return prompt_up:get() end,
  opacity = function() return (asking:get() and not dozing:get()) and 1.0 or 0.0 end,
  translate_y = function() return (asking:get() and 0 or s(8)) + lower_room() end,
  scale = magnify,
  transform_origin_y = PROMPT_Y / H,
  behavior = {
    opacity = { duration = 200, easing = "out_quad" },
    translate_y = squeeze,
  },
}
prompt[#prompt + 1] = line {
  x = COL_X, y = PROMPT_Y, width = COL_W,
  text = LOCKING and "LOCKED" or "SIGN IN",
  font_size = math.floor(BODY * 0.8),
  letter_spacing = s(2),
  color = function() return theme.faint end,
}
-- The chosen account's row, as it was in the list, without its box.
local ROW_Y = PROMPT_Y + CH
do
  local tiles = { x = COL_X + s(10), y = ROW_Y + math.floor((ROW_H - TILE) / 2), width = TILE, height = TILE,
    scale = function()
      if launching:get() then return math.max(W, H) * 3 / TILE end
      return 1.0
    end,
    behavior = { scale = { duration = 700, easing = "in_cubic" } },
  }
  for index, user in ipairs(users) do
    tiles[#tiles + 1] = tile(user, { visible = function() return chosen_user:get() == index end })
  end
  tiles[#tiles + 1] = tile(nil, { visible = function() return chosen_user:get() < 1 end })
  prompt[#prompt + 1] = ui.Item(tiles)
end
prompt[#prompt + 1] = line {
  x = COL_X + s(10) + TILE + s(14), y = ROW_Y + math.floor((ROW_H - CH) / 2),
  width = COL_W - TILE - s(40),
  opacity = function() return launching:get() and 0.0 or 1.0 end,
  text = function()
    local user = chosen()
    if user then return user.label .. "  " .. user.name .. "@" .. hostname end
    return naming:get() and "New user" or ""
  end,
  elide = "right",
  color = function() return theme.text end,
}

-- The field: a box, a caret. A key thickens the edge and a spring lets it
-- go; checking, it breathes in the accent; a refusal turns it the alert
-- colour and shakes it.
prompt[#prompt + 1] = ui.Rect {
  x = COL_X, y = FIELD_Y, width = COL_W, height = FIELD_H,
  radius = s(6),
  color = function() return theme.glass:alpha(theme.contrast and 0.2 or 0.03) end,
  border_width = function()
    return math.max(1, s(1)) + kick:get() * s(1) + (working:get() and breath:get() * s(1) or 0)
  end,
  border_color = function()
    if shake:get() > 0 then return theme.alert end
    if working:get() then return breath:get() > 0 and theme.accent or theme.edge_hot end
    return kick:get() > 0 and theme.accent or theme.edge_hot
  end,
  opacity = function() return launching:get() and 0.0 or 1.0 end,
  translate_x = function() return shake:get() * s(12) end,
  behavior = {
    border_width = { kind = "spring", mass = 1, damping = 12, stiffness = 420, epsilon = 0.01 },
    border_color = { duration = 220, easing = "out_quad", space = "oklch" },
    opacity = { duration = 200, easing = "out_quad" },
    translate_x = { kind = "spring", mass = 1, damping = 5.5, stiffness = 560, epsilon = 0.05 },
  },
}
prompt[#prompt + 1] = line {
  x = COL_X + s(16), y = FIELD_Y + math.floor((FIELD_H - CH) / 2), width = COL_W - s(32),
  opacity = function() return launching:get() and 0.0 or 1.0 end,
  translate_x = function() return shake:get() * s(12) end,
  behavior = {
    translate_x = { kind = "spring", mass = 1, damping = 5.5, stiffness = 560, epsilon = 0.05 },
  },
  text = function()
    local caret = (blink:get() and not working:get()) and "▍" or " "
    if naming:get() then
      local typed_name = custom_name:get()
      if typed_name == "" then return caret end
      return typed_name .. caret
    end
    if typed:get() == 0 then return caret end
    return ("•"):rep(typed:get()) .. caret
  end,
  color = function() return theme.text end,
}
prompt[#prompt + 1] = line {
  x = COL_X + s(16) + EM, y = FIELD_Y + math.floor((FIELD_H - CH) / 2), width = COL_W - s(32),
  text = function()
    if naming:get() then return custom_name:get() == "" and "user name" or "" end
    return typed:get() == 0 and "password" or ""
  end,
  color = function() return theme.faint end,
  opacity = function() return launching:get() and 0.0 or 1.0 end,
}
prompt[#prompt + 1] = ui.MouseArea {
  cursor = "text",
  x = COL_X, y = FIELD_Y, width = COL_W, height = FIELD_H,
  on_clicked = function()
    wake()
    confirm()
  end,
}
-- What there is to say, under the field; and the way back beside it.
prompt[#prompt + 1] = line {
  x = COL_X, y = FIELD_Y + FIELD_H + s(10), width = COL_W,
  text = function()
    if working:get() then return "checking…" end
    if message:get() ~= "" then return message:get() end
    if naming:get() then return "type a user name, then enter" end
    if LOCKING then return finger:get() end
    return ""
  end,
  elide = "right",
  color = function() return alarmed:get() and theme.alert or theme.dim end,
  behavior = { color = { duration = 150, easing = "out_quad" } },
}
if not LOCKING then
  prompt[#prompt + 1] = word("back", "ESC  BACK", COL_X, FIELD_Y + FIELD_H + s(10) + CH + s(8), go_back)
end
place(ui.Item(prompt))

--------------------------------------------------------------------------------
-- The corners, and their menus.
--------------------------------------------------------------------------------

toggle_menu = function(id)
  write(menu, menu:get() == id and "" or id)
end

local BAR_Y = s(12)
-- Where each menu opens: under its label. The labels are anchored, so the
-- menus are anchored the same way.
local menu_anchor = {}
local function label(id, side, order, text, on_tap, lit)
  -- Later labels on a side go further along by the widths of the earlier ones.
  local along = MARGIN
  for _, earlier in ipairs(menu_anchor[side] or {}) do along = along + earlier + s(8) end
  local width = #text * EM + EM * 2
  menu_anchor[side] = menu_anchor[side] or {}
  menu_anchor[side][#menu_anchor[side] + 1] = width
  menu_anchor[id] = { side = side, along = along }
  place(word(id, text, 0, 0, on_tap, lit,
             { top = true, top_margin = BAR_Y, [side] = true, [side .. "_margin"] = along }))
end
label("keyboard", "left", 1, "F1 KEYBOARD", open_keyboard, function() return shown:get() end)
if not LOCKING then
  label("session", "left", 2, "F2 SESSION", function() toggle_menu("session") end,
        function() return menu:get() == "session" end)
end
label("power", "right", 1, "F4 POWER", function() toggle_menu("power") end,
      function() return menu:get() == "power" end)
label("a11y", "right", 2, "F3 A11Y", function() toggle_menu("a11y") end,
      function() return menu:get() == "a11y" end)

--- A menu under its label: a box of rows, a dot on the one that is on.
local MENU_ROW = s(40)
local function menu_panel(id, rows)
  local widest = 0
  for _, row in ipairs(rows) do widest = math.max(widest, #row.label) end
  local width = math.max(s(200), (widest + 6) * EM)
  local height = s(8) * 2 + #rows * MENU_ROW
  local at = menu_anchor[id] or { side = "left", along = MARGIN }
  local panel = {
    width = width, height = height,
    anchors = { top = true, top_margin = BAR_Y + CH + s(8), [at.side] = true, [at.side .. "_margin"] = at.along },
    z = 10,
    visible = function() return menu:get() == id end,
    ui.Rect {
      anchors = { fill = true },
      radius = s(8),
      color = function() return theme.ink end,
      border_width = math.max(1, s(1)),
      border_color = function() return theme.edge_hot end,
    },
  }
  for index, row in ipairs(rows) do
    local hot = morf.signal("greeter.menu." .. id .. "." .. index, false)
    local y = s(8) + (index - 1) * MENU_ROW
    panel[#panel + 1] = ui.Rect {
      x = s(8), y = y, width = width - s(16), height = MENU_ROW,
      radius = s(5),
      color = function() return hot:get() and theme.glass_hot or theme.glass:alpha(0) end,
      behavior = { color = { duration = 80, easing = "out_quad" } },
    }
    panel[#panel + 1] = line {
      x = s(20), y = y + math.floor((MENU_ROW - CH) / 2), width = width - s(60),
      text = row.label,
      elide = "right",
      color = function() return theme.text end,
    }
    if row.on then
      local mark = s(6)
      panel[#panel + 1] = ui.Sdf {
        x = width - s(20) - mark, y = y + math.floor((MENU_ROW - mark) / 2),
        width = mark, height = mark,
        fill_color = function() return row.on() and theme.accent or theme.edge end,
        behavior = { fill_color = { duration = 120, easing = "out_quad", space = "oklch" } },
        ui.SdfShape { width = mark, height = mark, shape = "circle" },
      }
    end
    panel[#panel + 1] = ui.MouseArea {
      cursor = "pointer",
      x = s(8), y = y, width = width - s(16), height = MENU_ROW,
      on_entered = function() write(hot, true) end,
      on_exited = function() write(hot, false) end,
      on_clicked = function()
        wake()
        row.action()
      end,
    }
  end
  return ui.Item(panel)
end

local session_rows = {}
for index, entry in ipairs(available) do
  session_rows[#session_rows + 1] = {
    label = entry.name,
    on = function() return chosen_session:get() == index end,
    action = function()
      if not working:get() then write(chosen_session, index) end
      close_menus()
    end,
  }
end
if #session_rows == 0 then
  session_rows[1] = { label = "No sessions on this machine", action = close_menus }
end

if not LOCKING then place(menu_panel("session", session_rows)) end
place(menu_panel("power", {
  { label = "Suspend", action = function() power("Suspend") end },
  { label = "Restart", action = function() power("Reboot") end },
  { label = "Power off", action = function() power("PowerOff") end },
}))
place(menu_panel("a11y", {
  { label = "Large text", on = function() return a11y.large_text end,
    action = function() a11y.large_text = not a11y.large_text end },
  { label = "High contrast", on = function() return a11y.high_contrast end,
    action = function() a11y.high_contrast = not a11y.high_contrast end },
}))

--------------------------------------------------------------------------------
-- Timers.
--------------------------------------------------------------------------------

-- The pill's breath while a password is being checked: one beat in, one out.
place(ui.Timer {
  interval = 700, ["repeat"] = true,
  running = function() return working:get() end,
  on_triggered = function() write(breath, breath:get() > 0 and 0 or 1) end,
})
-- Half a minute untouched and the screen dozes: back to the accounts if it
-- was asking, then everything but the clock fades. Any key or move of the
-- pointer wakes it.
idle_rest = ui.Timer {
  interval = 30000, ["repeat"] = false, running = true,
  on_triggered = function()
    if asking:get() then go_back() end
    write(dozing, true)
  end,
}
place(idle_rest)

kick_rest = ui.Timer {
  interval = 70, ["repeat"] = false, running = false,
  on_triggered = function() write(kick, 0) end,
}
shake_rest = ui.Timer {
  interval = 60, ["repeat"] = false, running = false,
  on_triggered = function() write(shake, 0) end,
}
unmount_rest = ui.Timer {
  interval = 360, ["repeat"] = false, running = false,
  on_triggered = function()
    write(list_up, not asking:get())
    write(prompt_up, asking:get())
  end,
}
place(kick_rest)
place(shake_rest)
place(unmount_rest)
place(listen_again)
place(login_again)

--------------------------------------------------------------------------------
-- The keyboard.
--------------------------------------------------------------------------------

-- evdev, because that is what the board speaks — it hands over a key and
-- whether it was shifted, and what that means is this screen's business.
local EVDEV_BACKSPACE, EVDEV_ENTER = 14, 28

local function board_key(code, _shifted, label)
  wake()
  if working:get() then return end
  if code == EVDEV_BACKSPACE then
    backspace()
  elseif code == EVDEV_ENTER then
    if asking:get() then
      confirm()
    elseif chosen_user:get() < 1 then
      not_listed()
    else
      choose_user(chosen_user:get())
    end
  elseif label and label ~= "" then
    if not asking:get() then
      if chosen_user:get() < 1 then not_listed() else choose_user(chosen_user:get()) end
    end
    type_character(label)
  end
end

-- Both boards are built, and whichever is wanted is the one that is shown.
-- In this screen's own language: outlined keys on the ink, the face in the
-- text colour, the green for a shift that is on.
local look = {
  panel = function() return theme.ink end,
  keyface = function() return theme.ink:mix(theme.text, theme.contrast and 0.3 or 0.09) end,
  edge = function() return theme.edge_hot end,
  live = function() return theme.accent end,
  label = function() return theme.text end,
  dim = function() return theme.dim end,
  down = function() return theme.text end,
  font = MONO,
}
local keypad_panel, keypad_height = board.build {
  width = W,
  keypad = true,
  prefix = "greeter.keypad",
  key = board_key,
  on_full = function() write(numeric, false) end,
  look = look,
}
local full_panel, full_height = board.build {
  width = W,
  prefix = "greeter.keys",
  key = board_key,
  look = look,
}
board_height = math.max(keypad_height, full_height)

place(ui.Item {
  anchors = { left = true, right = true, bottom = true },
  height = board_height,
  visible = function() return shown:get() end,
  opacity = function() return shown:get() and 1.0 or 0.0 end,
  translate_y = function() return shown:get() and 0 or board_height end,
  behavior = {
    opacity = { duration = 160, easing = "out_quad" },
    translate_y = { kind = "spring", mass = 1, damping = 20, stiffness = 200, epsilon = 0.05 },
  },
  ui.Item {
    y = board_height - keypad_height, width = W, height = keypad_height,
    visible = function() return numeric:get() end,
    keypad_panel,
  },
  ui.Item {
    y = board_height - full_height, width = W, height = full_height,
    visible = function() return not numeric:get() end,
    full_panel,
  },
})

-- A lock listens to the reader from its first frame.
listen()

-- A lock's root has to be an opaque Rect: a compositor shows nothing behind a
-- lock surface, and the runtime refuses a root that could let it. The greeter
-- keeps the plain Item, which is what a layer surface is measured from.
if LOCKING then
  tree.color = function() return theme.ink end
  ui.Rect(tree)
else
  ui.Item(tree)
end
