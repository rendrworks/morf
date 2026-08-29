local mold = require("mold")

local Brightness = {}

function Brightness.new(value_path, maximum_path)
  local value = mold.file(value_path)
  local maximum = mold.file(maximum_path)
  return {
    level = function()
      return tonumber(value:read()) / tonumber(maximum:read())
    end,
    set_level = function(_, level)
      local target = math.floor(math.max(0, math.min(1, level)) * tonumber(maximum:read()))
      value:write(tostring(target))
    end,
  }
end

return Brightness
