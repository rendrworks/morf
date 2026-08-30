local mold = require("mold")
local core = require("mold.core")
local io = require("mold.io")
local ui = require("mold.ui")
local window = require("mold.window")

local font = "IosevkaTerm Nerd Font Mono"
local font_source = core.shell_path("assets/fonts")
local clock = core.system_clock { precision = "seconds" }
local calendar_clock = core.system_clock { precision = "hours" }

local theme = {
  color0 = "#000000",
  color1 = "#ffffff",
  color236 = "#1e1e1e",
  color238 = "#2a2a2a",
  color240 = "#303030",
  color244 = "#555555",
}

local home = core.env("HOME") or ""
local wal = io.file_view {
  path = home .. "/.cache/wal/colors.json",
  preload = true,
}
if wal:loaded() then
  local values = io.json.decode(wal:text())
  local colors = values.colors or {}
  for _, name in ipairs { "color0", "color1", "color236", "color238", "color240", "color244" } do
    theme[name] = colors[name] or theme[name]
  end
end

local function alpha(color, value)
  return color .. string.format("%02x", math.floor(value * 255 + 0.5))
end

local function text(values)
  values.font_family = font
  values.font_source = font_source
  values.font_weight = values.font_weight or 400
  return ui.Text(values)
end

local function card(x, y, width, height, radius, border_width, children)
  local values = {
    x = x,
    y = y,
    width = width,
    height = height,
    radius = radius,
    color = theme.color236,
    border_width = border_width,
    border_color = alpha(theme.color244, 0.08),
  }
  for _, child in ipairs(children or {}) do values[#values + 1] = child end
  return ui.Rect(values)
end

local function centered_label(value, width, height, size, color, weight)
  return text {
    width = width,
    height = height,
    text = value,
    font_size = size,
    font_weight = weight or 400,
    color = color,
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
end

local function progress_bar(x, y, width, line_height, value)
  local indicator_width = line_height * 0.3
  local indicator_gap = line_height * 0.8
  local indicator_position = function() return width * value:get() end
  return ui.Item {
    x = x,
    y = y,
    width = width,
    height = line_height * 6,
    ui.Rect {
      y = line_height * 2.5,
      height = line_height,
      width = function() return math.max(0, indicator_position() - indicator_gap) end,
      radius = line_height * 0.2,
      color = theme.color1,
      behavior = { width = { duration = 200, easing = "out_cubic" } },
    },
    ui.Rect {
      x = function() return indicator_position() - indicator_width * 0.5 end,
      y = line_height * 1.75,
      width = indicator_width,
      height = line_height * 2.5,
      radius = indicator_width * 0.5,
      color = theme.color1,
      behavior = { x = { duration = 200, easing = "out_cubic" } },
    },
    ui.Rect {
      x = function() return indicator_position() + indicator_gap end,
      y = line_height * 2.5,
      width = function() return math.max(0, width - indicator_position() - indicator_gap) end,
      height = line_height,
      radius = line_height * 0.2,
      color = alpha(theme.color244, 0.15),
      behavior = { x = { duration = 200, easing = "out_cubic" } },
    },
  }
end

local function logo_card(x, y, width, height, radius, border_width, line_height, small_radius)
  local logo_size = math.min(width, height) * 0.8
  local bar_width = width * 0.7
  local battery = mold.signal("board.battery", 0)
  local content_height = logo_size + line_height * 6
  local content_y = (height - content_height) * 0.5 + height * 0.05
  return card(x, y, width, height, radius, border_width, {
    ui.Image {
      x = (width - logo_size) * 0.5,
      y = content_y,
      width = logo_size,
      height = logo_size,
      source = "file://" .. home .. "/.config/bresilla.svg",
      fill_mode = "preserve_aspect_fit",
    },
    ui.Item {
      x = width * 0.15,
      y = content_y + logo_size,
      width = bar_width,
      height = line_height * 6,
      ui.Rect {
        y = line_height * 2.5,
        height = line_height,
        width = function() return math.max(0, bar_width * battery:get() / 100) end,
        radius = small_radius,
        color = function() return battery:get() > 20 and theme.color1 or "#ff5555" end,
        behavior = { width = { duration = 300, easing = "in_out_quad" } },
      },
      ui.Rect {
        x = function() return bar_width * battery:get() / 100 end,
        y = line_height * 2.5,
        width = function() return math.max(0, bar_width * (1 - battery:get() / 100)) end,
        height = line_height,
        radius = small_radius,
        color = alpha(theme.color244, 0.15),
        behavior = { x = { duration = 300, easing = "in_out_quad" } },
      },
    },
  })
end

local function system_card(x, y, width, height, radius, border_width, font_size)
  return card(x, y, width, height, radius, border_width, {
    centered_label("System", width, height, font_size, theme.color1),
  })
end

local function user_card(x, y, width, height, radius, border_width, line_height)
  local spacing = height * 0.08
  local item_width = (width - spacing * 2) * 0.5 - spacing * 1.5
  local item_height = height - spacing * 2
  local right_x = spacing + item_width + spacing * 1.5
  local title_size = height * 0.24
  local small_size = height * 0.13
  local icon_size = height * 0.34
  local left_height = title_size * 1.2 + small_size * 1.2 + spacing * 0.75
  local left_y = spacing + (item_height - left_height) * 0.5
  local controls_width = item_width * 0.8
  local controls_height = icon_size * 2 + spacing
  local controls_x = right_x + (item_width - controls_width) * 0.5
  local controls_y = spacing + (item_height - controls_height) * 0.5
  local bar_width = controls_width - icon_size - spacing
  local brightness = mold.signal("board.brightness", 0)

  local function icon_row(row_y, glyph, with_bar)
    local children = {
      ui.Rect {
        x = controls_x,
        y = row_y,
        width = icon_size,
        height = icon_size,
        radius = icon_size * 0.5,
        color = alpha(theme.color1, 0.15),
        centered_label(glyph, icon_size, icon_size, icon_size * 0.6, theme.color1),
      },
    }
    if with_bar then
      children[#children + 1] = progress_bar(
        controls_x + icon_size + spacing,
        row_y + (icon_size - line_height * 6) * 0.5,
        bar_width,
        line_height,
        brightness
      )
    end
    return children
  end

  local children = {
    text {
      x = spacing,
      y = left_y,
      width = item_width,
      height = title_size * 1.2,
      text = core.env("USER") or "User",
      font_size = title_size,
      font_weight = 500,
      color = theme.color1,
      elide = "right",
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    text {
      x = spacing,
      y = left_y + title_size * 1.2 + spacing * 0.75,
      width = item_width,
      height = small_size * 1.2,
      text = "Loading...",
      font_size = small_size,
      color = alpha(theme.color1, 0.7),
      elide = "right",
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
  }
  for _, child in ipairs(icon_row(controls_y, "󰕾", false)) do children[#children + 1] = child end
  for _, child in ipairs(icon_row(controls_y + icon_size + spacing, "󰃠", true)) do
    children[#children + 1] = child
  end
  return card(x, y, width, height, radius, border_width, children)
end

local function clock_card(x, y, width, height, radius, border_width)
  local digit_size = math.min(width, height) * 0.35
  local digit_width = digit_size * 0.58
  local row_spacing = digit_width * 0.15
  local colon_width = digit_width * 0.4
  local row_width = digit_width * 4 + colon_width + row_spacing * 4
  local date_size = digit_size * 0.375
  local column_spacing = height * 0.02
  local column_height = digit_size * 1.2 + column_spacing + date_size * 1.2
  local column_y = (height - column_height) * 0.5
  local cursor = (width - row_width) * 0.5
  local colon_visible = mold.signal("board.clock.colon", true)
  local children = {}

  local function digit(value, node_width)
    children[#children + 1] = text {
      x = cursor,
      y = column_y,
      width = node_width,
      height = digit_size * 1.2,
      text = value,
      font_size = digit_size,
      font_weight = 500,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    }
    cursor = cursor + node_width + row_spacing
  end

  digit(function() return clock:format("%H"):sub(1, 1) end, digit_width)
  digit(function() return clock:format("%H"):sub(2, 2) end, digit_width)
  children[#children + 1] = text {
    x = cursor,
    y = column_y,
    width = colon_width,
    height = digit_size * 1.2,
    text = ":",
    font_size = digit_size,
    font_weight = 500,
    color = theme.color1,
    opacity = function() return colon_visible:get() and 1 or 0 end,
    behavior = { opacity = { duration = 100, easing = "in_out_quad" } },
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
  cursor = cursor + colon_width + row_spacing
  digit(function() return clock:format("%M"):sub(1, 1) end, digit_width)
  digit(function() return clock:format("%M"):sub(2, 2) end, digit_width)
  children[#children + 1] = text {
    y = column_y + digit_size * 1.2 + column_spacing,
    width = width,
    height = date_size * 1.2,
    text = function() return clock:format("%b %d") end,
    font_size = date_size,
    color = alpha(theme.color1, 0.7),
    horizontal_alignment = "center",
    vertical_alignment = "center",
  }
  children[#children + 1] = ui.Timer {
    interval = 500,
    ["repeat"] = true,
    running = true,
    on_triggered = function() colon_visible:set(not colon_visible:get()) end,
  }
  return card(x, y, width, height, radius, border_width, children)
end

local function days_in_month(year, month)
  local days = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }
  if month == 2 and year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0) then return 29 end
  return days[month]
end

local function calendar_card(x, y, width, height, radius, border_width)
  local now = calendar_clock:snapshot()
  local scaled = math.min(width, height)
  local font_size = scaled * 0.04
  local spacing = scaled * 0.02
  local margin = spacing * 2
  local content_width = width - margin * 2
  local content_height = height - margin * 2
  local header_height = font_size * 2
  local weekdays_height = font_size * 1.5
  local weekdays_y = margin + header_height + spacing * 2
  local grid_y = weekdays_y + weekdays_height + spacing * 2
  local grid_height = content_height - header_height - weekdays_height - spacing * 4
  local cell_width = content_width / 7
  local cell_height = grid_height / 5
  local first_weekday = ((now.weekday - ((now.day - 1) % 7) - 1) % 7) + 1
  local previous_month = now.month == 1 and 12 or now.month - 1
  local previous_year = now.month == 1 and now.year - 1 or now.year
  local previous_days = days_in_month(previous_year, previous_month)
  local current_days = days_in_month(now.year, now.month)
  local children = {
    text {
      x = margin,
      y = margin,
      width = font_size * 2,
      height = header_height,
      text = "<",
      font_size = font_size,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    text {
      x = margin + font_size * 2,
      y = margin,
      width = content_width - font_size * 4,
      height = header_height,
      text = function() return calendar_clock:format("%B %Y") end,
      font_size = font_size * 1.3,
      font_weight = 500,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    text {
      x = margin + content_width - font_size * 2,
      y = margin,
      width = font_size * 2,
      height = header_height,
      text = ">",
      font_size = font_size,
      color = theme.color1,
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
  }

  local weekday_names = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }
  for index, name in ipairs(weekday_names) do
    children[#children + 1] = text {
      x = margin + (index - 1) * cell_width,
      y = weekdays_y,
      width = cell_width,
      height = weekdays_height,
      text = name,
      font_size = font_size,
      font_weight = 500,
      color = alpha(theme.color1, 0.6),
      horizontal_alignment = "center",
      vertical_alignment = "center",
    }
  end

  for index = 0, 34 do
    local offset = index - first_weekday + 2
    local day
    local current = true
    if offset < 1 then
      day = previous_days + offset
      current = false
    elseif offset > current_days then
      day = offset - current_days
      current = false
    else
      day = offset
    end
    local column = index % 7
    local row = math.floor(index / 7)
    local circle_size = math.min(cell_width - 4, cell_height - 4)
    local circle_x = margin + column * cell_width + (cell_width - circle_size) * 0.5
    local circle_y = grid_y + row * cell_height + (cell_height - circle_size) * 0.5
    local today = current and day == now.day
    children[#children + 1] = ui.Rect {
      x = circle_x,
      y = circle_y,
      width = circle_size,
      height = circle_size,
      radius = circle_size * 0.5,
      color = today and alpha(theme.color1, 0.12) or "transparent",
      centered_label(
        tostring(day),
        circle_size,
        circle_size,
        font_size,
        current and theme.color1 or alpha(theme.color1, 0.4),
        today and 500 or 400
      ),
    }
  end
  return card(x, y, width, height, radius, border_width, children)
end

local function media_card(x, y, width, height, radius, border_width)
  local scaled = math.min(width, height)
  local icon_size = scaled * 0.15
  local font_size = scaled * 0.05
  local spacing = scaled * 0.02
  local content_height = icon_size * 1.2 + spacing + font_size * 1.2
  local content_y = (height - content_height) * 0.5
  return card(x, y, width, height, radius, border_width, {
    text {
      y = content_y,
      width = width,
      height = icon_size * 1.2,
      text = "󰝚",
      font_size = icon_size,
      color = alpha(theme.color1, 0.5),
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
    text {
      y = content_y + icon_size * 1.2 + spacing,
      width = width,
      height = font_size * 1.2,
      text = "No Media",
      font_size = font_size,
      color = alpha(theme.color1, 0.7),
      horizontal_alignment = "center",
      vertical_alignment = "center",
    },
  })
end

mold.variants(mold.screens, function(screen)
  local screen_width = screen.width or 1920
  local screen_height = screen.height or 1080
  local short_side = math.min(screen_width, screen_height)
  local width = math.floor(screen_width * 0.576 + 0.5)
  local height = math.floor(screen_height * 0.544 + 0.5)
  local left = math.floor((screen_width - width) / 2 + 0.5)
  local top = math.floor((screen_height - height) / 2 + 0.5)
  local spacing = short_side * 12 / 2160
  local gap = spacing * 0.5
  local inset = gap
  local radius = short_side * 10 / 2160
  local border_width = short_side / 2160
  local line_height = short_side * 10 / 2160
  local small_radius = short_side * 2 / 2160
  local inner_width = width - inset * 2
  local inner_height = height - inset * 2
  local left_width = (inner_width - gap) * 0.22
  local right_width = (inner_width - gap) * 0.78
  local clock_height = (inner_height - gap) * 0.4
  local system_height = (inner_height - gap) * 0.6
  local user_height = (inner_height - gap) * 0.25
  local bottom_height = (inner_height - gap) * 0.75
  local info_width = (right_width - gap) * 0.68
  local logo_width = (right_width - gap) * 0.32
  local calendar_width = (right_width - gap) * 0.56
  local media_width = (right_width - gap) * 0.44
  local right_x = inset + left_width + gap

  mold.surface.namespace = "mold-board"
  mold.surface.width = width
  mold.surface.height = height
  mold.surface.exclusive_zone = 0
  mold.surface.anchors = { top = true, left = true }
  mold.surface.margin_left = left
  mold.surface.margin_top = top
  mold.surface.layer = "top"
  mold.surface.keyboard_focus = "none"
  mold.surface.mask = window.region {
    width = width,
    height = height,
    radius = math.floor(radius + 0.5),
  }

  return ui.ClipRect {
    width = width,
    height = height,
    color = theme.color238,
    opacity = 0.98,
    radius = radius,
    logo_card(inset, inset, left_width, clock_height, radius, border_width, line_height, small_radius),
    system_card(
      inset,
      inset + clock_height + gap,
      left_width,
      system_height,
      radius,
      border_width,
      short_side * 28 / 2160
    ),
    user_card(right_x, inset, info_width, user_height, radius, border_width, line_height),
    clock_card(right_x + info_width + gap, inset, logo_width, user_height, radius, border_width),
    calendar_card(
      right_x,
      inset + user_height + gap,
      calendar_width,
      bottom_height,
      radius,
      border_width
    ),
    media_card(
      right_x + calendar_width + gap,
      inset + user_height + gap,
      media_width,
      bottom_height,
      radius,
      border_width
    ),
  }
end)
