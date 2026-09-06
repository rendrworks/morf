-- The kit: every shape a page may be made of, and the only ones. One row,
-- one card with a slider, one row with choices, one section label, one
-- full-width action, one round button, one empty line, one statistic.
-- A page is a column of these at one gap, and so every page is the same
-- page with different words in it.

local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

local S = theme.S
local C = theme.color
local motion = theme.motion

local kit = {}

kit.ROW = S(56)
kit.GAP = S(8)
kit.PAD = S(14)
kit.CIRCLE = S(36)
kit.RADIUS = S(config.cornerR)

local function call(value)
  if type(value) == "function" then return value() end
  return value
end

--- The row. `icon` at the left in a circle, `title` over `subtitle` (a
--- row always has two lines: an empty second line is a blank, so rows
--- line up), and `right` at the right end: a switch, a chevron, a figure,
--- a cross. `active()` lights it with the accent. `on_click` is the row's
--- own tap; `right` keeps its taps to itself when it has any.
function kit.row(values)
  local W = values.width
  local H = values.height or kit.ROW
  local active = values.active or function() return false end
  local accent = values.accent or C.on
  local tint = values.tint or C.on_tint
  local edge = values.edge or C.on_edge
  local room = W - kit.PAD - kit.CIRCLE - S(12) - (values.right_w or S(56))
  local circle
  if values.icon_node then
    circle = values.icon_node
  else
    circle = ui.Rect {
      width = kit.CIRCLE, height = kit.CIRCLE, radius = kit.CIRCLE / 2,
      color = function() return active() and accent or C.card_hover end,
      behavior = { color = motion.hover },
      theme.icon { text = values.icon, size = config.iconSize - 1, anchors = { center_in = true },
        color = function() return active() and C.bg or C.fg end },
    }
  end
  local title = values.title_node or theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1,
    width = room, elide = "right" }
  -- A row with no second line centres its title; one with a second line
  -- keeps it even when the line is empty for the moment, so rows in a
  -- list stay level.
  local words
  if values.subtitle == nil and not values.title_node then
    words = title
  else
    words = ui.Column {
      gap = S(2), title,
      theme.text {
        text = function()
          local text = call(values.subtitle)
          return (text == nil or text == "") and " " or tostring(text)
        end,
        size = config.fontSize - 4, color = C.muted, width = room, elide = "right",
      },
    }
  end
  local children = {
    ui.Row {
      gap = S(12), align = "center", height = H,
      anchors = { left = true, left_margin = kit.PAD },
      circle,
      words,
    },
  }
  if values.right then
    children[#children + 1] = ui.Item {
      width = values.right_w or S(56), height = H, z = 2,
      anchors = { right = true },
      values.right,
    }
  end
  if values.on_right_click then
    children[#children + 1] = ui.MouseArea {
      anchors = { fill = true }, accepted_buttons = "right", on_clicked = values.on_right_click,
    }
  end
  local rest = {
    width = W, height = H, radius = values.pill and H / 2 or kit.RADIUS,
    visible = values.visible,
    translate_y = values.translate_y, opacity = values.opacity,
    color = function() return active() and tint or C.card end,
    hover_color = function() return active() and tint or C.card_hover end,
    border_width = 1,
    border_color = function() return active() and edge or C.edge end,
    on_click = values.on_click,
    behavior = { color = motion.hover, scale = motion.snappy, border_color = motion.hover,
      translate_y = motion.move, opacity = motion.fade },
  }
  for _, child in ipairs(children) do rest[#rest + 1] = child end
  return theme.button(rest)
end

--- The chevron for a row that opens a page: centred in the right end,
--- split off by a hairline when the row's own tap does something else.
function kit.chevron(on_click, split)
  return ui.Item {
    width = S(56), height = kit.ROW,
    ui.Rect { width = 1, height = kit.ROW - S(24), y = S(12), color = C.edge, visible = split == true },
    theme.icon { text = "󰅂", size = config.iconSize - 2, anchors = { center_in = true }, color = C.muted },
    ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = on_click },
  }
end

--- A cross at the right end: forget, dismiss, remove.
function kit.cross(on_click, visible)
  return ui.Item {
    width = S(56), height = kit.ROW,
    visible = visible,
    theme.icon { text = "󰅖", size = config.iconSize - 3, anchors = { center_in = true }, color = C.muted },
    ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = on_click },
  }
end

--- A figure at the right end: a percentage, a key, a count.
function kit.figure(text, width)
  return ui.Item {
    width = width or S(56), height = kit.ROW,
    theme.text { text = text, size = config.fontSize - 3, color = C.muted, anchors = { center_in = true } },
  }
end

--- A row whose right end is a switch and whose tap flips it.
function kit.switch_row(values)
  values.right = ui.Item {
    width = S(56), height = kit.ROW,
    ui.Item { anchors = { center_in = true }, theme.toggle(values.on, values.set) },
  }
  values.active = values.active or values.on
  values.on_click = values.on_click or function() values.set(not values.on()) end
  return kit.row(values)
end

--- A row whose tap opens something; the chevron says so.
function kit.nav_row(values)
  values.right = kit.chevron(values.on_click, false)
  return kit.row(values)
end

--- A card with a title, a figure at the right, and a slider under them;
--- `icon` at the left of the slider, and `on_icon` its tap (mute).
function kit.slider_card(values)
  local W = values.width
  local H = S(72)
  return theme.card {
    width = W, height = H,
    border_width = 1, border_color = C.edge,
    theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1,
      width = W - S(120), elide = "right",
      anchors = { left = true, top = true, left_margin = kit.PAD, top_margin = S(10) } },
    theme.text { text = values.value, size = config.fontSize - 4, color = C.muted,
      anchors = { right = true, top = true, right_margin = kit.PAD, top_margin = S(12) } },
    ui.Row {
      gap = S(10), align = "center",
      anchors = { left = true, bottom = true, left_margin = kit.PAD, bottom_margin = S(10) },
      ui.Item {
        width = S(24), height = S(24),
        visible = values.icon ~= nil,
        theme.icon { text = values.icon or "", size = config.iconSize - 2, anchors = { center_in = true },
          color = values.icon_color or C.muted },
        values.on_icon and ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = values.on_icon } or nil,
      },
      theme.slider { width = W - kit.PAD * 2 - (values.icon and S(34) or 0), height = S(20), track = S(8), knob = S(14),
        color = C.on, fraction = values.fraction, set = values.set },
    },
  }
end

--- A slider on one line: an icon, the track, a figure. For the quick
--- settings, where two sliders sit under the system row.
function kit.slider_line(values)
  local W = values.width
  return ui.Row {
    gap = S(12), align = "center", height = S(28),
    visible = values.visible,
    ui.Item { width = S(6), height = 1 },
    ui.Item {
      width = S(24), height = S(24),
      theme.icon { text = values.icon, size = config.iconSize, anchors = { center_in = true },
        color = values.icon_color or C.fg },
      values.on_icon and ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = values.on_icon } or nil,
    },
    theme.slider { width = W - S(6) - S(24) - S(12) * 3 - S(44), height = S(24), track = S(10), knob = S(16),
      color = C.on, fraction = values.fraction, set = values.set },
    theme.text { text = function() return math.floor(values.fraction() * 100 + 0.5) .. "%" end,
      size = config.fontSize - 3, color = C.muted, width = S(44), horizontal_alignment = "right" },
  }
end

--- A row with a title at the left and choices at the right, one lit.
function kit.choice_row(values)
  local chips = theme.chips(values.options, values.current, values.choose)
  return theme.card {
    width = values.width, height = kit.ROW,
    border_width = 1, border_color = C.edge,
    theme.text { text = values.title, font_weight = 700, size = config.fontSize - 1,
      anchors = { left = true, top = true, left_margin = kit.PAD, top_margin = S(19) } },
    ui.Item { anchors = { right = true, top = true, right_margin = S(10), top_margin = S(13) }, chips },
  }
end

--- A small heading over a group of rows.
function kit.section(text, visible)
  return ui.Item {
    width = S(200), height = S(22),
    visible = visible,
    theme.label { text = text, anchors = { left = true, left_margin = S(4), bottom = true } },
  }
end

--- A line of words where a list is empty.
function kit.empty(text, visible)
  return ui.Item {
    width = S(400), height = S(28),
    visible = visible,
    theme.text { text = text, size = config.fontSize - 3, color = C.faint, anchors = { left = true, left_margin = S(4), top = true, top_margin = S(6) } },
  }
end

--- A full-width action: `kind` is "plain", "primary" or "danger".
function kit.action(values)
  local kind = values.kind or "plain"
  local tint = kind == "primary" and C.on_tint or kind == "danger" and C.crit_tint or C.card
  local edge = kind == "primary" and C.on_edge or kind == "danger" and C.crit:alpha(0.5) or C.edge
  local ink = kind == "danger" and C.crit or kind == "primary" and C.on or C.muted
  return theme.button {
    width = values.width, height = S(48), radius = S(24),
    visible = values.visible,
    color = tint, hover_color = kind == "plain" and C.card_hover or tint,
    border_width = 1, border_color = edge,
    on_click = values.on_click,
    ui.Row {
      gap = S(10), align = "center", anchors = { center_in = true },
      values.icon and theme.icon { text = values.icon, size = config.iconSize - 2, color = ink } or nil,
      theme.text { text = values.label, font_weight = 700, size = config.fontSize - 1 },
    },
  }
end

--- A round button with a glyph, lit while `lit()`.
function kit.icon_button(glyph, on_click, lit)
  local D = S(50)
  return theme.button {
    width = D, height = D, radius = D / 2,
    color = function() return lit and lit() and C.on_tint or C.card end,
    border_width = 1, border_color = function() return lit and lit() and C.on_edge or C.edge end,
    on_click = on_click,
    theme.icon { text = glyph, size = config.iconSize - 2, anchors = { center_in = true },
      color = function() return lit and lit() and C.on or C.muted end },
  }
end

--- A statistic: a small label over a figure, centred.
function kit.stat(width, label, value)
  return theme.card {
    width = width, height = kit.ROW,
    border_width = 1, border_color = C.edge,
    ui.Column {
      gap = S(2), anchors = { center_in = true },
      ui.Item { width = width - S(20), height = S(14),
        theme.text { text = label, size = config.fontSize - 5, color = C.muted, anchors = { center_in = true } } },
      ui.Item { width = width - S(20), height = S(18),
        theme.text { text = value, size = config.fontSize - 1, font_weight = 700, anchors = { center_in = true } } },
    },
  }
end

--- Nodes side by side in `columns`, wrapping, each the same width.
function kit.grid(width, columns, nodes)
  return ui.Flex { direction = "row", wrap = true, gap = kit.GAP, width = width, table.unpack(nodes) }
end

--- How wide one cell of a grid is.
function kit.cell(width, columns)
  return math.floor((width - kit.GAP * (columns - 1)) / columns)
end

--- A page: its parts in one column at one gap, hidden parts taking no room.
function kit.page(width, parts)
  local values = { direction = "column", width = width, gap = kit.GAP }
  for _, part in ipairs(parts) do values[#values + 1] = part end
  return ui.Flex(values)
end

return kit
