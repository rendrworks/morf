-- A component: a model, the messages that change it, and a view of it.
--
-- The shape is Elm's, on this engine's own terms. `init(args)` returns a
-- plain table, which becomes a `morf.state`: every field a signal. `view`
-- runs once and returns an ordinary tree whose properties are functions of
-- the model, so a field changing redraws what reads it and nothing else.
-- `update(model, msg, send)` is the one place the model changes, and a
-- handler sends a message rather than writing a signal. Because a handler
-- is one flush, an update that touches five fields is one pass.
--
--   local Counter = component.define {
--     init = function(args) return { count = args.start or 0 } end,
--     update = function(model, msg)
--       if msg == "up" then model.count = model.count + 1 end
--     end,
--     view = function(model, send)
--       return ui.MouseArea {
--         on_clicked = send("up"),
--         ui.Text { text = function() return tostring(model.count) end },
--       }
--     end,
--   }
--   local counter = Counter { start = 3 }   -- counter.root is the node
--
-- `send(msg)` returns a handler that delivers the message when called;
-- `send_with(fn)` returns one that builds the message from the handler's
-- arguments, for keys and pointer positions; `dispatch(msg)` delivers one
-- now, from code that is not a handler, such as a D-Bus callback. A message
-- that is a function is a command: it runs, and what it returns is the
-- message, or nothing.

local morf = require("morf")

local component = {}

--- Defines a component and returns its constructor.
function component.define(spec)
  assert(type(spec.init) == "function", "component needs init")
  assert(type(spec.update) == "function", "component needs update")
  assert(type(spec.view) == "function", "component needs view")
  return function(args)
    local instance = {}
    local model = morf.state(spec.init(args or {}))
    local queue, draining = {}, false

    local function dispatch(msg)
      queue[#queue + 1] = msg
      if draining then return end
      draining = true
      while #queue > 0 do
        local next = table.remove(queue, 1)
        if type(next) == "function" then next = next() end
        if next ~= nil then spec.update(model, next, instance.send) end
      end
      draining = false
    end

    function instance.send(msg)
      return function() dispatch(msg) end
    end
    function instance.send_with(build)
      return function(...) dispatch(build(...)) end
    end
    instance.dispatch = dispatch
    instance.model = model
    instance.root = spec.view(model, instance.send, instance)
    return instance
  end
end

return component
