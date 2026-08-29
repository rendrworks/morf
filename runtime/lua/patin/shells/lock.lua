local mold = require("mold")
local ui = require("mold.ui")
local theme = require("patin.theme")
local TextField = require("patin.widgets.text_field")

return ui.component(function(props)
  local password = mold.signal("patin.lock.password", props.password or "")
  local message = mold.signal("patin.lock.message", props.message or "")
  local busy = false
  local function authenticate()
    if busy then return end
    busy = true
    message:set("Checking…")
    mold.pam.authenticate_unlock(
      props.pam_service or "login",
      props.username or "",
      password:get(),
      function(ok, error)
        busy = false
        if not ok then
          password:set("")
          message:set(error or "Authentication failed")
        end
      end
    )
  end
  local function handle_key(keysym, text)
    if keysym == 65293 then authenticate(); return end
    if keysym == 65288 then
      password:set(string.sub(password:get(), 1, -2))
      return
    end
    if text and text ~= "" then password:set(password:get() .. text) end
  end
  return ui.Rect {
    width = props.width or 720,
    height = props.height or 1280,
    color = theme.colors.background,
    ui.Column {
      x = props.form_x or 220,
      y = props.form_y or 460,
      spacing = theme.spacing.lg,
      ui.Text {
        text = props.clock or "Locked",
        font_size = 48,
        color = theme.colors.text,
      },
      TextField {
        text = function() return string.rep("•", #password:get()) end,
        placeholder = "Password",
        on_key_pressed = props.on_key_pressed or handle_key,
      },
      ui.Text {
        text = function() return message:get() end,
        color = theme.colors.muted,
      },
    },
  }
end)
