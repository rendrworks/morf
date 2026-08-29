local mold = require("mold")
local ui = require("mold.ui")
local Bar = require("patin.shells.bar")
local Launcher = require("patin.shells.launcher")
local Osk = require("patin.shells.osk")
local Lock = require("patin.shells.lock")
local Notifications = require("patin.shells.notifications")
local NetworkSettings = require("patin.shells.network_settings")

return ui.component(function(props)
  local apps = props.apps or mold.list_model({ "Browser", "Messages", "Settings" })
  local notifications = props.notifications or mold.list_model({})
  return ui.Item {
    width = props.width or 720,
    height = props.height or 1280,
    Bar { width = props.width or 720, left = props.bar_left, right = props.bar_right },
    Launcher { x = 180, y = 100, model = apps, visible = props.launcher_visible },
    Notifications { x = 370, y = 52, model = notifications },
    NetworkSettings { x = 180, y = 100, wifi_enabled = props.wifi_enabled },
    Osk { x = 100, y = 1080, on_key = props.on_osk_key },
    Lock {
      width = props.width or 720,
      height = props.height or 1280,
      visible = props.locked or false,
      password = props.password,
      message = props.lock_message,
    },
  }
end)
