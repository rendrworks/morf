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
--
-- The card is a component (`examples/lib/component.lua`): one model with
-- the fields the card shows, one `update` where every change to it lives,
-- and a view that is functions of the model. What Escape does is one
-- branch of one function, not four handlers.

local morf = require("morf")
local ui = require("morf.ui")
local polkit_agent = require("lib.polkit_agent")
local component = require("lib.component")

local W = 520
local IDLE_HEIGHT, OPEN_HEIGHT = 1, 196

morf.surface.width = W
morf.surface.height = IDLE_HEIGHT
morf.surface.anchors = { top = true }
morf.surface.layer = "overlay"
morf.surface.keyboard_focus = "none"

local BG, CARD, TEXT, MUTED, FIELD, ACCENT, BAD =
  "#101418", "#1b2128", "#f2f4f6", "#9aa5b1", "#0c1014", "#5fa8ff", "#ff6b6b"

local RETURN, KP_ENTER, BACKSPACE, ESCAPE = 0xff0d, 0xff8d, 0xff08, 0xff1b

--- Opens or closes the card: size and keyboard focus follow, and the
--- compositor is told without a reconnect.
local function show(opening)
  morf.surface.height = opening and OPEN_HEIGHT or IDLE_HEIGHT
  morf.surface.keyboard_focus = opening and "exclusive" or "none"
end

-- The password is a plain local, never a field: fields are signals, and a
-- secret wants to be neither named nor observable. The model only knows
-- how many characters there are.
local password = ""
local current

local Card = component.define {
  init = function()
    return {
      open = false, title = "", detail = "", info = "", prompt = "",
      verdict = "", typed = 0, refused = false,
    }
  end,

  update = function(model, msg)
    if msg.type == "request" then
      local request = msg.request
      current = request
      model.title = request.message or request.action_id or "Authentication required"
      model.detail = request.action_id .. "  ·  as " .. tostring(request.user)
      model.info = request.info or ""
      model.prompt = request.prompt or ""
      model.verdict, model.refused = "", false
      if not model.open then
        password, model.typed = "", 0
        model.open = true
        show(true)
      end
    elseif msg.type == "done" then
      model.verdict = msg.ok and "authorised" or ("refused: " .. tostring(msg.reason))
      model.refused = not msg.ok
      current = nil
      password, model.typed = "", 0
      -- Long enough to read the verdict, short enough not to be in the way.
      morf.timer(1200, function()
        if not current then
          model.open = false
          show(false)
        end
      end)
    elseif msg.type == "key" and current then
      local keysym, text = msg.keysym, msg.text
      if keysym == RETURN or keysym == KP_ENTER then
        current.answer(password)
        password, model.typed = "", 0
        model.info = "checking"
      elseif keysym == BACKSPACE then
        password = password:sub(1, -2)
        model.typed = #password
      elseif keysym == ESCAPE then
        current.cancel()
      elseif text and text ~= "" and #password < 128 then
        password = password .. text
        model.typed = #password
      end
    elseif msg.type == "cancel" and current then
      current.cancel()
    end
  end,

  view = function(model, send, self)
    local function dots()
      local n = model.typed
      return n == 0 and "" or string.rep("●", math.min(n, 40))
    end
    local function tone() return model.refused and BAD or MUTED end
    return ui.MouseArea {
      width = W,
      height = OPEN_HEIGHT,
      on_key_pressed = self.send_with(function(keysym, text)
        return { type = "key", keysym = keysym, text = text }
      end),
      ui.Rect {
        width = W, height = OPEN_HEIGHT, color = BG,
        visible = function() return model.open end,
        ui.Rect {
          x = 8, y = 8, width = W - 16, height = OPEN_HEIGHT - 16, radius = 12, color = CARD,
          ui.Column {
            x = 18, y = 14, width = W - 52, gap = 6,
            ui.Text {
              text = function() return model.title end,
              color = TEXT, font_size = 15, font_weight = 600, width = W - 52, wrap = true,
              max_lines = 2,
            },
            ui.Text { text = function() return model.detail end, color = MUTED, font_size = 11 },
            ui.Text {
              text = function() return model.info end,
              color = tone, font_size = 12, width = W - 52, wrap = true, max_lines = 2,
            },
            ui.Row {
              gap = 10,
              -- The label sits on the field's centre line, not its top edge.
              align = "center",
              ui.Text {
                text = function() return model.prompt ~= "" and model.prompt or "Password:" end,
                color = TEXT, font_size = 13, width = 90,
              },
              ui.MouseArea {
                width = W - 52 - 100, height = 30,
                cursor = "text",
                ui.Rect {
                  width = W - 52 - 100, height = 30, radius = 6, color = FIELD,
                  border_color = ACCENT, border_width = 1,
                  ui.Text { x = 10, y = 6, text = dots, color = TEXT, font_size = 14 },
                },
              },
            },
            ui.Text {
              text = function()
                if model.verdict ~= "" then return model.verdict end
                return "Return to authenticate  ·  Escape to cancel"
              end,
              color = tone, font_size = 11,
            },
          },
        },
      },
    }
  end,
}

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

-- The card's root is a node with no parent, which is what a surface draws.
local card = Card {}
local agent, why
if not hosts_agent() then
  why = "another output hosts the agent"
else
  agent, why = polkit_agent.serve {
    subject = "process",
    on_request = function(request) card.dispatch { type = "request", request = request } end,
    on_done = function(_, ok, reason) card.dispatch { type = "done", ok = ok, reason = reason } end,
  }
end

morf.ipc.request = function()
  if not agent then return "no agent: " .. tostring(why) end
  local model = card.model
  if not model.open then return "no request" end
  return model.title .. " | " .. model.detail .. " | info: " .. model.info
    .. " | prompt: " .. model.prompt .. " | typed: " .. model.typed
    .. (model.verdict ~= "" and (" | " .. model.verdict) or "")
end
morf.ipc.cancel = function()
  card.dispatch { type = "cancel" }
  return "cancelled"
end
