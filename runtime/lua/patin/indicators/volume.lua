local mold = require("mold")
local ui = require("mold.ui")
local Volume = require("patin.services.volume")

return ui.component(function(props)
  local volume = props.service or Volume.new()
  local text = mold.signal("patin.volume.text", "mute")
  local function refresh()
    local muted, is_muted = pcall(volume.muted)
    if not muted or is_muted then text:set("mute"); return end
    local ok, level = pcall(volume.level)
    if ok then text:set(string.format("%.0f%%", level * 100)) end
  end
  refresh()
  mold.timer(props.interval or 1000, refresh)
  return ui.Text {
    color = props.color or "#eceff4",
    text = function() return text:get() end,
  }
end)
