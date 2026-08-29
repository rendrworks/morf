local function matches(kind, value)
  if type(value) == "function" or value == nil or kind == nil or kind == "any" then return true end
  if kind == "boolean" then return type(value) == "boolean" end
  if kind == "color" then return type(value) == "string" or type(value) == "table" end
  return type(value) == kind
end

return function(definition)
  if type(definition) == "function" then return definition end
  assert(type(definition) == "table", "component definition must be a function or table")
  assert(type(definition.build) == "function", "component definition requires build")
  local properties = definition.properties or {}
  local signal_names = {}
  for _, name in ipairs(definition.signals or {}) do signal_names[name] = true end
  local component_name = definition.name or "component"

  return function(input)
    input = input or {}
    local values = {}
    for name, schema in pairs(properties) do
      local value = input[name]
      if value == nil then value = schema.default end
      assert(matches(schema.type, value), component_name .. " property `" .. name .. "` expects " .. schema.type)
      values[name] = value
    end
    for key in pairs(input) do
      if type(key) == "string" and properties[key] == nil then
        local signal = string.match(key, "^on_(.+)$")
        assert(signal and signal_names[signal], "unknown " .. component_name .. " property `" .. key .. "`")
      end
    end

    local slot = definition.default_slot
    if slot then
      local children = {}
      for index, child in ipairs(input) do children[index] = child end
      if input[slot] then
        for _, child in ipairs(input[slot]) do children[#children + 1] = child end
      end
      values[slot] = children
    end

    local self = {}
    function self:emit(name, ...)
      assert(signal_names[name], "unknown " .. component_name .. " signal `" .. name .. "`")
      local handler = input["on_" .. name]
      if handler then return handler(...) end
    end
    function self:binding(name)
      assert(properties[name], "unknown " .. component_name .. " property `" .. name .. "`")
      return function() return self[name] end
    end
    setmetatable(self, {
      __index = function(_, name)
        local value = values[name]
        if type(value) == "function" then return value() end
        return value
      end,
      __newindex = function(_, name, value)
        assert(properties[name], "unknown " .. component_name .. " property `" .. name .. "`")
        assert(matches(properties[name].type, value), component_name .. " property `" .. name .. "` has invalid type")
        values[name] = value
      end,
    })
    return definition.build(self)
  end
end
