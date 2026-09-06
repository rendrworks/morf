-- Settings: the island, the clock, the look, motion and notifications,
-- written back to settings.json. A change that reshapes the island --
-- its sizes, its font, its colours -- takes effect at the next start;
-- the ones that are read live change at once.

local morf = require("morf")
local ui = require("morf.ui")
local config = require("config")
local theme = require("theme")

local S = theme.S
local C = theme.color

local page = {}

page.title = "Settings"
page.icon = "󰒓"

page.section = morf.signal("panacea.settings.section", "island")
page.saved = morf.signal("panacea.settings.saved", "")

function page.width() return theme.panel_w(1.4) end

local SECTIONS = {
  { id = "island", glyph = "󰍹", label = "Bar & Island" },
  { id = "clock", glyph = "󰥔", label = "Clock & Date" },
  { id = "look", glyph = "󰏘", label = "Appearance" },
  { id = "motion", glyph = "󰑮", label = "Motion" },
  { id = "notif", glyph = "󰂚", label = "Notifications" },
  { id = "system", glyph = "󰒓", label = "System" },
}

-- The values being edited, as a state so every control follows.
local draft = morf.state {
  pillH = config.pillH, panelW = config.panelW, cornerR = config.cornerR, notchFlare = config.notchFlare,
  collapsedW = config.collapsedW, notchMode = config.notchMode, pillOverlay = config.pillOverlay,
  clock12 = config.clock12, clockSeconds = config.clockSeconds, clockWeekday = config.clockWeekday,
  fontSize = config.fontSize, iconSize = config.iconSize, colOn = config.colOn, colFg = config.colFg,
  mutedAlpha = config.mutedAlpha, themeId = config.themeId,
  animMove = config.animMove, animBounce = config.animBounce, reduceMotion = config.reduceMotion,
  notifTimeout = config.notifTimeout, notifDnd = config.notifDnd,
  terminal = config.terminal,
}

local function apply()
  local changes = {}
  for _, key in ipairs { "pillH", "panelW", "cornerR", "notchFlare", "collapsedW", "notchMode", "pillOverlay",
    "clock12", "clockSeconds", "clockWeekday", "fontSize", "iconSize", "colOn", "colFg", "mutedAlpha", "themeId",
    "animMove", "animBounce", "reduceMotion", "notifTimeout", "notifDnd", "terminal" } do
    changes[key] = draft[key]
  end
  changes.animMs = changes.animMove
  local ok = config.save(changes)
  require("notify").silent:set(draft.notifDnd)
  page.saved:set(ok and "Saved. Sizes, fonts and colours apply at the next start." or "Could not write settings.json")
  morf.timer(4000, function() page.saved:set("") end, false)
end

local function reset()
  draft.pillH, draft.panelW, draft.cornerR, draft.notchFlare, draft.collapsedW = 38, 540, 14, 12, 260
  draft.notchMode, draft.pillOverlay = true, true
  draft.clock12, draft.clockSeconds, draft.clockWeekday = false, false, true
  draft.fontSize, draft.iconSize, draft.colOn, draft.colFg, draft.mutedAlpha, draft.themeId = 15, 17, "#3b82f6", "#ffffff", 0.45, "default"
  draft.animMove, draft.animBounce, draft.reduceMotion = 230, 79, false
  draft.notifTimeout, draft.notifDnd = 5000, false
end

page.subtitle = function()
  for _, section in ipairs(SECTIONS) do
    if section.id == page.section:get() then return section.label end
  end
  return ""
end

function page.build(island)
  local W = page.width() - S(32)
  local SIDE = S(56)
  local BODY = W - SIDE - S(16)

  local function slider_setting(label, key, min, max, unit, step)
    return ui.Row {
      gap = S(12), align = "center",
      ui.Item { width = S(120), height = S(22), theme.text { text = label, size = config.fontSize - 2, anchors = { left = true, top = true, top_margin = S(2) } } },
      theme.slider {
        width = BODY - S(200), height = S(22), track = S(5), knob = S(14),
        fraction = function() return (draft[key] - min) / (max - min) end,
        set = function(fraction)
          local value = min + fraction * (max - min)
          if step then value = math.floor(value / step + 0.5) * step end
          draft[key] = value
        end,
      },
      ui.Item { width = S(60), height = S(22),
        theme.text { text = function()
          local value = draft[key]
          if step and step < 1 then return string.format("%.2f%s", value, unit) end
          return string.format("%d%s", value, unit)
        end, size = config.fontSize - 3, color = C.muted, anchors = { right = true, top = true, top_margin = S(2) } } },
    }
  end
  local function toggle_setting(label, key, hint)
    return ui.Item {
      width = BODY, height = hint and S(44) or S(30),
      ui.Column {
        gap = S(2), anchors = { left = true, top = true, top_margin = S(3) },
        theme.text { text = label, size = config.fontSize - 2 },
        hint and theme.text { text = hint, size = config.fontSize - 5, color = C.muted } or nil,
      },
      ui.Item { anchors = { right = true, top = true, top_margin = S(3) },
        theme.toggle(function() return draft[key] end, function(on) draft[key] = on end) },
    }
  end
  local function pick_setting(label, key, options)
    return ui.Row {
      gap = S(12), align = "center",
      ui.Item { width = S(120), height = S(30), theme.text { text = label, size = config.fontSize - 2, anchors = { left = true, top = true, top_margin = S(6) } } },
      theme.chips(options, function() return draft[key] end, function(v) draft[key] = v end),
    }
  end
  local function swatches(label, key, colours)
    local dots = {}
    for _, colour in ipairs(colours) do
      dots[#dots + 1] = ui.Rect {
        width = S(24), height = S(24), radius = S(12), color = colour,
        border_width = function() return draft[key] == colour and S(3) or 0 end,
        border_color = C.fg:alpha(0.7),
        ui.MouseArea { anchors = { fill = true }, cursor = "pointer", on_clicked = function() draft[key] = colour end },
      }
    end
    return ui.Row {
      gap = S(12), align = "center",
      ui.Item { width = S(120), height = S(24), theme.text { text = label, size = config.fontSize - 2, anchors = { left = true, top = true, top_margin = S(3) } } },
      ui.Row { gap = S(8), table.unpack(dots) },
    }
  end

  local sections = {
    island = ui.Column {
      gap = S(12),
      theme.label { text = "Sizes" },
      slider_setting("Pill height", "pillH", 28, 56, "px", 1),
      slider_setting("Panel width", "panelW", 400, 900, "px", 10),
      slider_setting("Collapsed width", "collapsedW", 180, 520, "px", 10),
      slider_setting("Corner radius", "cornerR", 0, 28, "px", 1),
      slider_setting("Notch flare", "notchFlare", 0, 24, "px", 1),
      theme.label { text = "Edge" },
      toggle_setting("Notch mode", "notchMode", "Hugs the screen edge with concave corners; off is a capsule with a gap."),
      toggle_setting("Float over windows", "pillOverlay", "Off reserves the pill's strip so windows stay below it."),
    },
    clock = ui.Column {
      gap = S(12),
      theme.label { text = "Clock" },
      pick_setting("Format", "clock12", { { label = "24-hour", value = false }, { label = "12-hour", value = true } }),
      toggle_setting("Seconds", "clockSeconds"),
      toggle_setting("Weekday on the pill", "clockWeekday"),
    },
    look = ui.Column {
      gap = S(12),
      theme.label { text = "Theme" },
      pick_setting("Theme", "themeId", { { label = "Default", value = "default" }, { label = "Nothing", value = "nothing" } }),
      theme.label { text = "Font" },
      slider_setting("Text size", "fontSize", 11, 20, "", 1),
      slider_setting("Icon size", "iconSize", 12, 24, "", 1),
      theme.label { text = "Colours" },
      swatches("Text", "colFg", { "#ffffff", "#d4d4d8", "#fde68a", "#86efac", "#93c5fd", "#f9a8d4" }),
      swatches("Accent", "colOn", { "#3b82f6", "#22c55e", "#f59e0b", "#ef4444", "#a855f7", "#14b8a6" }),
      slider_setting("Dimming", "mutedAlpha", 0.2, 0.8, "", 0.05),
    },
    motion = ui.Column {
      gap = S(12),
      theme.label { text = "Motion" },
      slider_setting("Speed", "animMove", 80, 600, "ms", 10),
      slider_setting("Bounce", "animBounce", 0, 100, "%", 1),
      toggle_setting("Reduce motion", "reduceMotion", "Every move lands at once."),
    },
    notif = ui.Column {
      gap = S(12),
      theme.label { text = "Notifications" },
      slider_setting("Shown for", "notifTimeout", 1000, 15000, "ms", 500),
      toggle_setting("Do not disturb", "notifDnd"),
    },
    system = ui.Column {
      gap = S(12),
      theme.label { text = "Programs" },
      pick_setting("Terminal", "terminal", { "foot", "footclient", "kitty", "alacritty", "wezterm" }),
      theme.label { text = "About" },
      theme.text { text = "Panacea on morf. Settings live in " .. config.path, size = config.fontSize - 4, color = C.muted,
        width = BODY, wrap = true, max_lines = 2 },
    },
  }

  local side = {}
  for _, section in ipairs(SECTIONS) do
    side[#side + 1] = theme.button {
      width = S(40), height = S(40), radius = 12,
      color = function() return page.section:get() == section.id and C.on_tint or morf.color("transparent") end,
      on_click = function() page.section:set(section.id) end,
      theme.icon { text = section.glyph, size = config.iconSize - 1, anchors = { center_in = true },
        color = function() return page.section:get() == section.id and C.fg or C.muted end },
    }
  end

  local bodies = {}
  for _, section in ipairs(SECTIONS) do
    local node = sections[section.id]
    bodies[#bodies + 1] = ui.Item(theme.reveal(
      (function()
        local shown = morf.signal("panacea.settings.shown." .. section.id, page.section:get() == section.id)
        theme.tick(function() shown:set(page.section:get() == section.id) end)
        return shown
      end)(),
      { anchors = { left = true, top = true }, from_y = S(8), node }))
  end

  return ui.Row {
    gap = S(16), align = "start",
    ui.Column { gap = S(4), table.unpack(side) },
    ui.Column {
      gap = S(12),
      ui.Item {
        width = BODY, height = S(30),
        ui.Row {
          gap = S(8), anchors = { right = true },
          theme.button { width = S(80), height = S(30), radius = 10, color = C.on_tint, on_click = apply,
            theme.text { text = "Apply", size = config.fontSize - 3, font_weight = 700, anchors = { center_in = true } } },
          theme.button { width = S(80), height = S(30), radius = 10, on_click = reset,
            theme.text { text = "Reset", size = config.fontSize - 3, anchors = { center_in = true } } },
        },
      },
      theme.text { text = function() return page.saved:get() end, size = config.fontSize - 4, color = C.muted,
        visible = function() return page.saved:get() ~= "" end },
      ui.Item { width = BODY, height = S(300), table.unpack(bodies) },
    },
  }
end

return page
