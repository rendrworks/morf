local mold = require("mold")
local core = require("mold.core")
local io = require("mold.io")
local ui = require("mold.ui")
local window = require("mold.window")

local palette = {
  background = "#111318",
  surface = "#1b1d23",
  raised = "#24272f",
  foreground = "#f1f1f4",
  muted = "#a9abb5",
  accent = "#89b4fa",
}

local home = core.env("HOME")
if home then
  local wal = io.file_view {
    path = home .. "/.cache/wal/colors.json",
    preload = true,
  }
  if wal:loaded() then
    local colors = io.json.decode(wal:text())
    if colors.special then
      palette.background = colors.special.background or palette.background
      palette.foreground = colors.special.foreground or palette.foreground
    end
    if colors.colors then
      palette.surface = colors.colors.color0 or palette.surface
      palette.raised = colors.colors.color8 or palette.raised
      palette.accent = colors.colors.color4 or palette.accent
    end
  end
end

local function label(text, x, y, size, color)
  return ui.Text {
    x = x,
    y = y,
    text = text,
    font_size = size,
    color = color or palette.foreground,
  }
end

local function card(x, y, width, height, title, children)
  local values = {
    x = x,
    y = y,
    width = width,
    height = height,
    radius = 12,
    color = palette.surface,
    border_width = 1,
    border_color = palette.raised,
    label(title, 18, 14, 14, palette.muted),
  }
  for _, child in ipairs(children or {}) do
    values[#values + 1] = child
  end
  return ui.Rect(values)
end

mold.variants(mold.screens, function(screen)
  local screen_width = screen.width or 1920
  local screen_height = screen.height or 1080
  local width = math.floor(screen_width * 0.576 + 0.5)
  local height = math.floor(screen_height * 0.544 + 0.5)
  local left = math.floor((screen_width - width) / 2)
  local top = math.floor((screen_height - height) / 2)

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
    radius = 14,
  }

  local gap = math.max(6, math.floor(math.min(width, height) * 0.012))
  local inset = gap
  local inner_width = width - inset * 2
  local inner_height = height - inset * 2
  local left_width = math.floor((inner_width - gap) * 0.22)
  local right_width = inner_width - gap - left_width
  local clock_height = math.floor((inner_height - gap) * 0.40)
  local system_height = inner_height - gap - clock_height
  local user_height = math.floor((inner_height - gap) * 0.25)
  local bottom_height = inner_height - gap - user_height
  local info_width = math.floor((right_width - gap) * 0.68)
  local logo_width = right_width - gap - info_width
  local calendar_width = math.floor((right_width - gap) * 0.56)
  local media_width = right_width - gap - calendar_width
  local right_x = inset + left_width + gap

  return ui.Rect {
    width = width,
    height = height,
    color = palette.background,
    opacity = 0.98,
    radius = 14,
    clip = true,

    card(inset, inset, left_width, clock_height, "MOLD", {
      ui.Icon {
        x = math.floor(left_width * 0.30),
        y = math.floor(clock_height * 0.25),
        width = math.floor(left_width * 0.40),
        height = math.floor(left_width * 0.40),
        name = "applications-graphics",
      },
      label("Rust rendering engine", 18, clock_height - 38, 13, palette.muted),
    }),

    card(inset, inset + clock_height + gap, left_width, system_height, "SYSTEM", {
      label(core.env("USER") or "user", 18, 48, 22),
      label(screen.name, 18, 82, 14, palette.accent),
      label("scale  " .. tostring(screen.scale), 18, 116, 14, palette.muted),
      label(core.env("XDG_SESSION_TYPE") or "linux", 18, 148, 14, palette.muted),
    }),

    card(right_x, inset, info_width, user_height, "SESSION", {
      label("General shell primitives", 18, 48, 22),
      label("files  json  processes  sockets", 18, 82, 14, palette.muted),
    }),

    card(right_x + info_width + gap, inset, logo_width, user_height, "CLOCK", {
      ui.Text {
        x = 12,
        y = math.floor(user_height * 0.42),
        width = logo_width - 24,
        text = function() return mold.clock:get() end,
        font_size = math.max(24, math.floor(logo_width * 0.16)),
        color = palette.foreground,
        horizontal_alignment = "center",
      },
    }),

    card(right_x, inset + user_height + gap, calendar_width, bottom_height, "CALENDAR", {
      label("MON   TUE   WED   THU   FRI   SAT   SUN", 18, 52, 13, palette.muted),
      label("  1      2      3      4      5      6      7", 18, 88, 15),
      label("  8      9     10     11     12     13     14", 18, 122, 15),
      label(" 15     16     17     18     19     20     21", 18, 156, 15),
      label(" 22     23     24     25     26     27     28", 18, 190, 15),
      label(" 29     30     31", 18, 224, 15),
    }),

    card(right_x + calendar_width + gap, inset + user_height + gap, media_width, bottom_height, "MEDIA", {
      ui.Rect {
        x = 18,
        y = 50,
        width = media_width - 36,
        height = math.max(80, math.floor(bottom_height * 0.42)),
        radius = 10,
        color = palette.raised,
        label("consumer-owned", 18, 20, 18, palette.accent),
        label("Connect a media plugin here", 18, 52, 13, palette.muted),
      },
      label("The engine provides composition,", 18, math.floor(bottom_height * 0.62), 13, palette.muted),
      label("not a built-in media widget.", 18, math.floor(bottom_height * 0.62) + 24, 13, palette.muted),
    }),
  }
end)
