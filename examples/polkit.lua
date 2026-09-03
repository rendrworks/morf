-- A polkit dialog, drawn by the shell.
--
-- The agent is `examples/lib/polkit_agent.lua`; this is the dialog a shell
-- puts over it. Nothing is on screen until polkit asks; then a card drops
-- from the top of the output with what is being asked, whatever PAM has to
-- say about it, and a field for the password. Return sends it, Escape gives
-- up, and the card goes away with the verdict.
--
-- Registered for this process only, so it can be tried while the session's
-- own agent keeps its job:
--
--   pkcheck --action-id org.freedesktop.policykit.exec --process <pid> -u
--
-- `morf ipc call request` says what the card is showing, for a test that
-- cannot look at the screen. The password is typed into the card and
-- nowhere else.

local morf = require("morf")
local ui = require("morf.ui")
local polkit_agent = require("lib.polkit_agent")

local W = 520
local IDLE_HEIGHT, OPEN_HEIGHT = 1, 196

morf.surface.width = W
morf.surface.height = IDLE_HEIGHT
morf.surface.anchors = { top = true }
morf.surface.layer = "overlay"
morf.surface.keyboard_focus = "none"

local BG, CARD, TEXT, MUTED, FIELD, ACCENT, BAD =
  "#101418", "#1b2128", "#f2f4f6", "#9aa5b1", "#0c1014", "#5fa8ff", "#ff6b6b"

-- What the card shows. The password itself is a plain local, never a
-- signal: signals are named and observable, and a secret wants neither. The
-- screen only needs to know how many characters there are.
local open = morf.signal("polkit.open", false)
local title = morf.signal("polkit.title", "")
local detail = morf.signal("polkit.detail", "")
local info = morf.signal("polkit.info", "")
local prompt = morf.signal("polkit.prompt", "")
local verdict = morf.signal("polkit.verdict", "")
local typed = morf.signal("polkit.typed", 0)
local password = ""
local current

local function clear_password()
  password = ""
  typed:set(0)
end

--- Opens or closes the card: size and keyboard focus follow, and the
--- compositor is told without a reconnect.
local function show(opening)
  open:set(opening)
  morf.surface.height = opening and OPEN_HEIGHT or IDLE_HEIGHT
  morf.surface.keyboard_focus = opening and "exclusive" or "none"
end

-- One agent per session, and a session with three outputs runs three copies
-- of this file. The output whose name sorts first hosts it -- which is also
-- the one `morf ipc call` reaches, so the answer lands where the request is.
local function hosts_agent()
  local mine = morf.screens[1].name
  for _, screen in ipairs(morf.screens) do
    if screen.name < mine then return false end
  end
  return true
end

local agent, why
if not hosts_agent() then
  why = "another output hosts the agent"
else
  agent, why = polkit_agent.serve {
    subject = "process",
    on_request = function(request)
      current = request
      title:set(request.message or request.action_id or "Authentication required")
      detail:set(request.action_id .. "  ·  as " .. tostring(request.user))
      info:set(request.info or "")
      prompt:set(request.prompt or "")
      verdict:set("")
      if not open:get() then
        clear_password()
        show(true)
      end
    end,
    on_done = function(request, ok, reason)
      verdict:set(ok and "authorised" or ("refused: " .. tostring(reason)))
      current = nil
      clear_password()
      -- Long enough to read the verdict, short enough not to be in the way.
      morf.timer(1200, function()
        if not current then show(false) end
      end)
    end,
  }
end

local function submit()
  if not current then return end
  current.answer(password)
  clear_password()
  info:set("checking")
end

local function cancel()
  if not current then return end
  current.cancel()
end

morf.ipc.request = function()
  if not agent then return "no agent: " .. tostring(why) end
  if not open:get() then return "no request" end
  return title:get() .. " | " .. detail:get() .. " | info: " .. info:get()
    .. " | prompt: " .. prompt:get() .. " | typed: " .. typed:get()
    .. (verdict:get() ~= "" and (" | " .. verdict:get()) or "")
end
morf.ipc.cancel = function()
  cancel()
  return "cancelled"
end

local function dots()
  local n = typed:get()
  if n == 0 then return prompt:get() ~= "" and "" or "" end
  return string.rep("●", math.min(n, 40))
end

ui.MouseArea {
  width = W,
  height = OPEN_HEIGHT,
  on_key_pressed = function(keysym, text)
    local RETURN, KP_ENTER, BACKSPACE, ESCAPE = 0xff0d, 0xff8d, 0xff08, 0xff1b
    if not current then return end
    if keysym == RETURN or keysym == KP_ENTER then
      submit()
    elseif keysym == BACKSPACE then
      password = password:sub(1, -2)
      typed:set(#password)
    elseif keysym == ESCAPE then
      cancel()
    elseif text and text ~= "" and #password < 128 then
      password = password .. text
      typed:set(#password)
    end
  end,
  ui.Rect {
    width = W, height = OPEN_HEIGHT, color = BG,
    visible = function() return open:get() end,
    ui.Rect {
      x = 8, y = 8, width = W - 16, height = OPEN_HEIGHT - 16, radius = 12, color = CARD,
      ui.Column {
        x = 18, y = 14, width = W - 52, spacing = 6,
        ui.Text {
          text = function() return title:get() end,
          color = TEXT, font_size = 15, font_weight = 600, width = W - 52, wrap = true,
        },
        ui.Text { text = function() return detail:get() end, color = MUTED, font_size = 11 },
        ui.Text {
          text = function() return info:get() end,
          color = function() return verdict:get():find("refused", 1, true) and BAD or MUTED end,
          font_size = 12, width = W - 52, wrap = true,
        },
        ui.Row {
          spacing = 10,
          -- The label sits on the field's centre line, not its top edge.
          alignment = "center",
          ui.Text {
            text = function() return prompt:get() ~= "" and prompt:get() or "Password:" end,
            color = TEXT, font_size = 13, width = 90,
          },
          ui.Rect {
            width = W - 52 - 100, height = 30, radius = 6, color = FIELD,
            border_color = ACCENT, border_width = 1,
            ui.Text {
              x = 10, y = 6,
              text = dots, color = TEXT, font_size = 14,
            },
          },
        },
        ui.Text {
          text = function()
            local v = verdict:get()
            if v ~= "" then return v end
            return "Return to authenticate  ·  Escape to cancel"
          end,
          color = function() return verdict:get():find("refused", 1, true) and BAD or MUTED end,
          font_size = 11,
        },
      },
    },
  },
}
