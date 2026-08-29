local mold = require("mold")

local Oxin = {}

function Oxin.new(path)
  local socket = mold.socket(path)
  return {
    request = function(_, bytes, maximum, timeout)
      socket:send(bytes)
      return socket:receive(maximum or 65536, timeout or 500)
    end,
  }
end

return Oxin
