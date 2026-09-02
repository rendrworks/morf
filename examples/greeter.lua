-- A login screen.
--
-- Not a display manager. `greetd` is the daemon that owns authentication and
-- starts sessions; a greeter is an ordinary Wayland client it runs, and this is
-- one. greetd draws nothing itself — it launches a compositor and the greeter
-- inside it, so morf needs no DRM, no root and no rendering it does not already
-- do. The same binary, in a different host.
--
--     systemd
--      └─ greetd.service         root, no graphics, owns the VT
--          ├─ cage               a Wayland compositor, for the login only
--          │   └─ morf greeter.lua
--          │        └── $GREETD_SOCK ──► greetd
--          └─ on success: kills cage, starts the session
--
-- Try it *nested* first, inside a session you are already logged into:
--
--     cage -s -- morf examples/greeter.lua
--
-- A greeter you can only test by logging out is a greeter you will eventually
-- be locked out by. Needs `greetd` and `cage` installed.
--
-- Then, when it behaves, `/etc/greetd/config.toml`:
--
--     [terminal]
--     vt = 1
--
--     [default_session]
--     command = "cage -s -- morf /etc/morf/greeter.lua"
--     user = "greeter"
--
-- Two things that bite. It runs as user `greeter`, not as you — every font,
-- asset and path here must be readable by that user, which is the usual way
-- these fail. And `GREETD_SOCK` is only set in the environment greetd provides,
-- so run nested and the password field will say so rather than pretending.

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

local INK = "#0b0e14"
local PANEL = "#141a24"
local TEXT = "#e8ecf4"
local MUTED = "#7f8899"
local ACCENT = "#7fb7c9"
local ALERT = "#e8735a"

--------------------------------------------------------------------------------
-- Who can log in.
--------------------------------------------------------------------------------

--- Ordinary human accounts, read straight out of `/etc/passwd`.
---
--- No D-Bus and no AccountsService: the file is world-readable, it is the
--- authority, and a login screen that cannot list users because a daemon is not
--- running is worse than one that reads a file. The range is the convention
--- every distribution follows — below 1000 is the system's, and 65534 is
--- `nobody`.
local function accounts()
  local users = {}
  local text = io.file("/etc/passwd"):read()
  if not text then return users end
  for line in text:gmatch("[^\n]+") do
    local name, uid, gecos, shell = line:match("^([^:]+):[^:]*:(%d+):[^:]*:([^:]*):[^:]*:([^:]*)$")
    local id = tonumber(uid or "")
    if name and id and id >= 1000 and id < 65534 then
      -- A shell that cannot be logged into is an account that cannot log in.
      if not (shell:match("nologin$") or shell:match("/false$")) then
        users[#users + 1] = {
          name = name,
          label = (gecos ~= "" and gecos:match("^[^,]*") or name),
        }
      end
    end
  end
  table.sort(users, function(a, b) return a.name < b.name end)
  return users
end

--- The sessions this machine can start.
---
--- Ordinary desktop entries in the two session directories. `Exec` is the
--- command greetd runs once the password is accepted.
local function sessions()
  local found = {}
  local entries = core.desktop_entries(core.session_paths())
  for _, entry in ipairs(entries:applications()) do
    if entry.exec and entry.exec ~= "" then
      found[#found + 1] = { name = entry.name, exec = entry.exec }
    end
  end
  return found
end

local users = accounts()
local available = sessions()
local user_index = 1
local session_index = 1
local password = ""
local status = "enter password"
local busy = false

--------------------------------------------------------------------------------
-- Talking to greetd.
--------------------------------------------------------------------------------

--- Runs one login attempt to completion.
---
--- greetd's protocol is a conversation: a session is created, it answers with
--- whatever PAM wants to ask, and each answer may produce another question.
--- This handles the common shape — one password prompt — and cancels on
--- anything it does not understand rather than guessing, because a greeter that
--- guesses at an authentication prompt is answering a question it did not read.
local function attempt()
  if busy or #users == 0 or #available == 0 then return end
  busy = true
  status = "checking"

  local ok, session = pcall(morf.greetd.connect)
  if not (ok and session) then
    status = "no greetd (run me under greetd, or nested with cage)"
    busy = false
    password = ""
    return
  end

  local reply = session:create_session(users[user_index].name)
  while reply and reply.type == "auth_message" do
    if reply.auth_message_type == "secret" or reply.auth_message_type == "visible" then
      reply = session:respond(password)
    else
      -- An informational or error message carries no answer.
      reply = session:respond(nil)
    end
  end

  if reply and reply.type == "success" then
    local chosen = available[session_index]
    local started = session:start_session({ chosen.exec }, {})
    if started and started.type == "success" then
      status = "starting " .. chosen.name
      -- greetd takes it from here: this process is about to be replaced by the
      -- session it just asked for.
      return
    end
    status = (started and started.description) or "could not start the session"
    session:cancel_session()
  else
    status = (reply and reply.description) or "authentication failed"
    session:cancel_session()
  end

  password = ""
  busy = false
end

--------------------------------------------------------------------------------
-- Turning the machine off.
--------------------------------------------------------------------------------

--- Asks logind to suspend, reboot or power off.
---
--- Over D-Bus rather than by running `systemctl`, because the greeter is
--- already able to talk to the system bus and this is one call rather than a
--- process. The `false` is logind's `interactive` flag: a greeter has nobody to
--- prompt for a polkit password, so asking for an interactive check would hang
--- waiting for an agent that is not running.
---
--- Nothing here asks whether it is *allowed*. `CanPowerOff` exists and would
--- let a button be greyed out, but a greeter that hides an action it could have
--- offered is worse than one that tries and reports the refusal.
local function power(method)
  local ok, manager = pcall(io.dbus.proxy, "system", "org.freedesktop.login1",
                            "/org/freedesktop/login1", "org.freedesktop.login1.Manager")
  if not (ok and manager) then
    status = "cannot reach logind"
    return
  end
  local called, err = pcall(manager.call_with, manager, method, false)
  if not called then
    status = tostring(err)
  end
end

--------------------------------------------------------------------------------
-- What it looks like.
--------------------------------------------------------------------------------

local CARD_W = 460
local CARD_H = 300
local CARD_X = math.floor((W - CARD_W) / 2)
local CARD_Y = math.floor((H - CARD_H) / 2)

local clock = ui.Text {
  x = 0, y = CARD_Y - 130, width = W,
  text = "", font_size = 64, color = TEXT, horizontal_alignment = "center",
}
local who = ui.Text {
  x = CARD_X + 28, y = CARD_Y + 26, width = CARD_W - 56,
  text = "", font_size = 20, color = TEXT,
}
local session_line = ui.Text {
  x = CARD_X + 28, y = CARD_Y + 58, width = CARD_W - 56,
  text = "", font_size = 12, color = MUTED,
}
local dots = ui.Text {
  x = CARD_X + 28, y = CARD_Y + 122, width = CARD_W - 56,
  text = "", font_size = 22, color = ACCENT,
}
local note = ui.Text {
  x = CARD_X + 28, y = CARD_Y + 186, width = CARD_W - 56,
  text = status, font_size = 12, color = MUTED,
}
local help = ui.Text {
  x = CARD_X + 28, y = CARD_Y + 244, width = CARD_W - 56,
  text = "tab: user   ctrl+tab: session   enter: log in   F1/F2/F12: suspend, reboot, off",
  font_size = 11, color = MUTED,
}

local function redraw()
  local user = users[user_index]
  who.text = user and (user.label ~= "" and user.label or user.name) or "no accounts found"
  local chosen = available[session_index]
  session_line.text = chosen and chosen.name or "no sessions found"
  dots.text = string.rep("•", #password)
  note.text = status
  note.color = status:match("fail") or status:match("no greetd") and ALERT or MUTED
  clock.text = core.system_clock():format("%H:%M")
end

ui.Item {
  width = W, height = H,
  ui.Rect { width = W, height = H, color = INK },
  clock,
  ui.Rect {
    x = CARD_X, y = CARD_Y, width = CARD_W, height = CARD_H,
    radius = 16, color = PANEL,
  },
  who, session_line, dots, note, help,

  ui.Timer {
    interval = 1000, ["repeat"] = true, running = true,
    on_triggered = redraw,
  },

  -- Keyboard focus is exclusive, so every key arrives here.
  ui.MouseArea {
    width = W, height = H,
    on_key_pressed = function(key, modifiers, text)
      if busy then return end
      if key == "Return" or key == "KP_Enter" then
        attempt()
      elseif key == "BackSpace" then
        password = password:sub(1, -2)
      elseif key == "Tab" and modifiers and modifiers.control then
        if #available > 0 then session_index = session_index % #available + 1 end
      elseif key == "Tab" then
        if #users > 0 then user_index = user_index % #users + 1 end
        password = ""
      elseif key == "Escape" then
        password = ""
      elseif key == "F1" then
        power("Suspend")
      elseif key == "F2" then
        power("Reboot")
      elseif key == "F12" then
        power("PowerOff")
      elseif text and text ~= "" and #password < 256 then
        password = password .. text
      end
      redraw()
    end,
  },
}

redraw()
