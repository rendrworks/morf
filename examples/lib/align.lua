-- The bar layout: something at the left, something at the right, and
-- something in the middle.
--
-- A `ui.Layout` container with the two functions every custom layout is:
-- `measure` says how big it wants to be given its children, `place` says
-- where each child goes in the box it got. The children have been measured
-- already, once, and arrive as `{ width, height }`; this returns one
-- `{ x, y }` per child. Ten lines that every example used to write with a
-- cursor variable and a subtraction.
--
--   align.bar { left, middle, right, height = 40 }
--
-- Any of the three may be nil; the middle is centred in the whole bar, not
-- in what the sides leave, which is how a clock stays put when the tray
-- grows. `gap` keeps the sides off the edges.

local ui = require("morf.ui")

local align = {}

--- Builds the bar. `options` is the `ui.Layout` table minus the functions:
--- put the three children in it, in order, and any size or anchors.
function align.bar(options)
  local gap = options.gap or 0
  options.gap = nil
  local count = #options
  options.measure = function(available, children)
    local width, height = 0, 0
    for _, child in ipairs(children) do
      width = width + child.width
      height = math.max(height, child.height)
    end
    if available.width < math.huge then width = available.width end
    return width + gap * 2, height
  end
  options.place = function(bounds, children)
    local placements = {}
    for index, child in ipairs(children) do
      local x
      if index == 1 and count > 1 then
        x = gap
      elseif index == count then
        x = bounds.width - child.width - gap
      else
        x = (bounds.width - child.width) / 2
      end
      placements[index] = { x = x, y = (bounds.height - child.height) / 2 }
    end
    return placements
  end
  return ui.Layout(options)
end

return align
