-- The island: one black capsule at the top edge that is the whole shell.
--
-- Collapsed it is a strip with the day, the clock, the workspace, the
-- keyboard layout and the battery. Asked for a page it grows downwards
-- into that page, and the strip's pieces do not fade: they are the same
-- nodes throughout. The clock's letters walk into the page's title, the
-- day's into its subtitle, the battery's glyph into the page's icon, each
-- travelling on a spring from where it sat in the strip to where the page
-- keeps it, and back when the page closes. The capsule hugs the screen
-- edge with two concave corners, like a hardware notch.
--
-- The island lives on the shell's own surface, collapsed or open, so a
-- page is one spring away rather than a surface away. The keyboard comes
-- from a second surface the size of a pixel with exclusive focus, opened
-- while a page is open: every page closes with Escape, which that surface
-- hears, or a click outside, which the shell's surface hears.

local morf = require("morf")
local ui = require("morf.ui")
local core = require("morf.core")
local config = require("config")
local theme = require("theme")
local system = require("system")
local hypr = require("hypr")
local bar = require("bar_glyphs")

local S = theme.S
local C = theme.color
local motion = theme.motion
local state = system.state

local island = {}

local PILL_H = S(config.pillH)
local PHONE = theme.phone
local FLARE = PHONE and 0 or S(config.notchFlare)
local RADIUS = S(config.cornerR)
local NOTCH = config.notchMode and not PHONE
local PAD = S(16)
-- Where a page's content starts: under the title and subtitle.
island.HEADER_H = S(50)
island.BIG_HEADER_H = S(58)

-- Which page is open on the expanded surface; "" is collapsed.
island.page = morf.signal("panacea.page", "")
-- The pages this one was reached through, for the back button.
island.history = {}
island.depth = morf.signal("panacea.depth", 0)
island.expanded = morf.signal("panacea.expanded", false)
-- What the recorder says, for the collapsed strip.
island.recording = morf.signal("panacea.recording", "")

-- Pages register here: name -> { build = fn(host) -> node, title, subtitle,
-- icon, big (the clock stays a clock, large), width = fn?, on_key?,
-- on_open?, on_close?, slots }.
--
-- `slots` is where a page keeps a piece of the strip: `battery_text = {
-- node, size, weight }` says the strip's "100%" lands on that node, at that
-- size, and the page draws nothing of its own there. A page that rebuilds
-- a slot bumps `island.slots_changed`.
island.pages = {}
island.slots_changed = morf.signal("panacea.slots.changed", 0)

function island.register(name, page)
  island.pages[name] = page
end

local function page_width(name)
  local page = island.pages[name]
  if page and page.width then return page.width() end
  return theme.panel_w()
end

--- A page's title or subtitle right now: a string, or a function of none.
local function line_of(page, key)
  if not page then return "" end
  local value = page[key]
  if type(value) == "function" then value = value() end
  return tostring(value or "")
end

-- ------------------------------------------------------------------ pieces --

local clock_format = config.clock12 and "%I:%M" or "%H:%M"
if config.clockSeconds then clock_format = clock_format .. ":%S" end

local battery_glyph = bar.battery_glyph

local function battery_color()
  if state.battery.charging then return C.ok end
  if state.battery.percent <= 15 then return C.crit end
  return C.fg
end

--- What each strip piece says while collapsed.
local function strip_words()
  return {
    day = theme.clock:format("%a"),
    clock = theme.clock:format(clock_format),
    ws = hypr.state.active_label,
    layout = hypr.state.keymap ~= "" and hypr.state.keymap or "US",
    battery = state.battery.percent .. "%",
  }
end

-- ------------------------------------------------------------------- body --

--- The capsule and its flares, sized by `width` and `height` functions,
--- around `content`.
local function capsule(width, height, content)
  local function open_now() return island.page:get() ~= "" end
  local body = ui.Rect {
    -- Open, the capsule warms very slightly toward the accent. No edge:
    -- a hairline never meets the notch's flares cleanly.
    color = function() return open_now() and C.bg:mix(C.on, 0.07) or C.bg end,
    width = width,
    height = height,
    top_left_radius = (NOTCH or PHONE) and 0 or RADIUS,
    top_right_radius = (NOTCH or PHONE) and 0 or RADIUS,
    -- A phone's status bar is square; the sheet under it is not.
    bottom_left_radius = function() return (PHONE and island.page:get() == "") and 0 or RADIUS end,
    bottom_right_radius = function() return (PHONE and island.page:get() == "") and 0 or RADIUS end,
    behavior = config.bench and {} or { width = motion.move, height = motion.move, color = motion.fade },
    -- Clipped to the body, so a page still growing does not show past the
    -- capsule. A square clip: a rounded one would render through an
    -- offscreen layer every frame, and nothing reaches the corners anyway.
    ui.ClipRect {
      anchors = { fill = true },
      color = "transparent",
      content,
    },
  }
  local flares = {}
  if NOTCH then
    flares[1] = ui.Image {
      source = core.shell_path("assets/flare-left.svg"),
      width = FLARE, height = FLARE,
      anchors = { left = true, top = true, left_margin = -FLARE },
    }
    flares[2] = ui.Image {
      source = core.shell_path("assets/flare-right.svg"),
      width = FLARE, height = FLARE,
      anchors = { right = true, top = true, right_margin = -FLARE },
    }
  end
  return ui.Item {
    width = width,
    height = height,
    behavior = config.bench and {} or { width = motion.move, height = motion.move },
    body,
    flares[1],
    flares[2],
  }
end

-- ---------------------------------------------------------------- expanded --

local page_nodes = {}

--- A text that walks its letters into whatever it is told to say next.
---
--- `to(word)` starts the walk; the node's `text` becomes the word when it
--- lands, and a walk asked for mid-walk lands the current one first.
local function morpher(values)
  local progress = morf.signal("panacea.morph." .. tostring(values.key), 0)
  local node
  local target = values.text
  local pending = nil
  values.key = nil
  values.morph_to = values.text
  values.morph_progress = function() return progress:get() end
  values.behavior = values.behavior or {}
  values.behavior.morph_progress = {
    duration = config.reduceMotion and 1 or (config.animMove or 230) * 1.6,
    easing = "in_out_cubic",
    on_finished = function()
      if progress:get() >= 1 then
        node.text = node.morph_to
        progress:set(0)
        if pending then
          local next_word = pending
          pending = nil
          morf.timer(16, function()
            node.morph_to = next_word
            target = next_word
            progress:set(1)
          end, false)
        end
      end
    end,
  }
  node = theme.text(values)
  local function to(word)
    word = tostring(word or "")
    if word == target then return end
    if progress:get() > 0 then
      pending = word
      return
    end
    target = word
    node.morph_to = word
    progress:set(1)
  end
  return node, to
end

--- A glyph in a field, which morphs into another glyph.
local function glyph_morpher(size, color, values)
  local progress = morf.signal("panacea.glyph.morph", 0)
  local shape = ui.SdfShape {
    anchors = { fill = true },
    shape = "glyph",
    glyph = "󰁹",
    glyph_morph_to = "󰁹",
  }
  values = values or {}
  values.width, values.height = size, size
  values.fill_color = color
  values.morph_progress = function() return progress:get() end
  values.behavior = values.behavior or {}
  values.behavior.morph_progress = {
    duration = config.reduceMotion and 1 or (config.animMove or 230) * 1.6,
    easing = "in_out_cubic",
    on_finished = function()
      if progress:get() >= 1 then
        shape.glyph = shape.glyph_morph_to
        progress:set(0)
      end
    end,
  }
  values[#values + 1] = shape
  local field = ui.Sdf(values)
  local current = "󰁹"
  local function to(glyph)
    if glyph == current or glyph == "" then return end
    current = glyph
    shape.glyph_morph_to = glyph
    progress:set(1)
  end
  return field, to
end

--- Every registered page, built once.
local function build_pages()
  local nodes = {}
  for name, page in pairs(island.pages) do
    local shown = morf.signal("panacea.page.shown." .. name, false)
    local content = page.build(island)
    local top = PAD + (page.big and island.BIG_HEADER_H or island.HEADER_H)
    -- The content grows out of the strip rather than fading in: it starts
    -- small under the title and springs to size.
    local mounted = theme.mounted(shown)
    -- A page taller than the island scrolls: the content slides up inside
    -- a clip on a spring, by the wheel, as far as its last row.
    local scroll = morf.signal("panacea.page.scroll." .. name, 0)
    local function room() return S(config.expandedH) - top - PAD end
    local function visible_h() return math.min(content.layout_height or 0, room()) end
    local node = ui.Item {
      anchors = { left = true, top = true, right = true, left_margin = PAD, right_margin = PAD, top_margin = top },
      height = visible_h,
      visible = function() return shown:get() or mounted:get() end,
      opacity = function() return shown:get() and 1 or 0 end,
      translate_y = function() return shown:get() and 0 or -S(40) end,
      scale = function() return shown:get() and 1 or 0.7 end,
      transform_origin_y = 0,
      behavior = { opacity = { duration = 90 }, translate_y = motion.move, scale = motion.move },
      ui.ClipRect {
        anchors = { fill = true }, color = "transparent",
        ui.Item {
          anchors = { left = true, top = true, right = true },
          translate_y = function() return -scroll:get() end,
          behavior = { translate_y = motion.move },
          content,
        },
        ui.MouseArea {
          anchors = { fill = true }, z = -1,
          on_wheel = function(sx, sy, px, py, steps_x, steps_y) theme.wheel(sx, sy, px, py, steps_x, steps_y) end,
          on_pressed = function() theme.drag_begin() end,
          on_released = function() theme.drag_end() end,
          on_dragged = function(_, _, dx, dy) theme.drag(dx or 0, dy or 0) end,
        },
      },
    }
    page_nodes[name] = { node = node, shown = shown, content = content, top = top, scroll = scroll, room = room }
    nodes[#nodes + 1] = node
  end
  return nodes
end

--- The island with its pages. Its size follows the page: the strip's when
--- none is open, the page's otherwise.
function island.build()
  local pages = build_pages()
  local spring = motion.move

  local function current_page()
    local name = island.page:get()
    return name, island.pages[name]
  end

  -- The five pieces of the strip, as free nodes. Each is set in type once,
  -- at the largest size it ever takes, and scaled down from there: a scale
  -- is a transform the GPU applies for nothing, where a font size that
  -- moves re-shapes and re-rasterises the letters every frame.
  local day, clock, ws, layout_text, glyph, battery_text, status_row
  local ICON = S(config.iconSize - 1)
  local GAP = S(12)
  local BASE = {
    day = config.fontSize,
    clock = 26,
    ws = config.fontSize,
    layout = config.fontSize,
    battery = config.fontSize + 4,
    status = 1,
  }
  -- Flipped once every piece exists, so every binding that read a piece
  -- through the guard runs again and takes it up.
  local ready = morf.signal("panacea.pieces.ready", false)
  local function open() return island.page:get() ~= "" end
  local function big()
    local _, page = current_page()
    return page ~= nil and page.big == true
  end
  local function has_icon()
    local _, page = current_page()
    return page ~= nil and page.icon ~= nil and not page.big
  end

  -- Where the page keeps a piece, if it does: the slot's laid-out place,
  -- in the island's own coordinates, and the size it wants there.
  local body
  local function slot(name)
    local _ = island.slots_changed:get()
    local _, page = current_page()
    if not open() or not page or not page.slots then return nil end
    local entry = page.slots[name]
    if not entry or not entry.node then return nil end
    return entry
  end
  local function slot_x(name)
    local entry = slot(name)
    if not entry or not ready:get() then return nil end
    return (entry.node.layout_x or 0) - (body and body.layout_x or 0)
  end
  local function slot_y(name)
    local entry = slot(name)
    if not entry or not ready:get() then return nil end
    return (entry.node.layout_y or 0) - (body and body.layout_y or 0)
  end

  -- The scale each piece is drawn at, now: its size in the strip, the
  -- page's title or subtitle size, or a slot's, over its set size.
  local function scale_of(name)
    if not open() then
      if name == "clock" then return (config.fontSize + 1) / BASE.clock end
      if name == "battery" then return config.fontSize / BASE.battery end
      return 1
    end
    local entry = slot(name == "battery" and "battery_text" or name)
    if entry then return (entry.size or config.fontSize) / BASE[name] end
    if name == "day" then return (config.fontSize - 3) / BASE.day end
    if name == "clock" then return (big() and 26 or config.fontSize + 1) / BASE.clock end
    return 0.4
  end
  -- Laid-out sizes, as drawn.
  local function w(node, name)
    if not ready:get() or not node then return 0 end
    return (node.layout_width or 0) * scale_of(name)
  end
  local function h(node, name)
    if not ready:get() or not node then return PILL_H end
    return (node.layout_height or PILL_H) * scale_of(name)
  end
  -- The status icons take room only while one of them shows.
  local function status_w()
    local width = w(status_row, "status")
    return width > 0 and width + GAP or 0
  end
  local function strip_total()
    return w(day, "day") + GAP + w(clock, "clock") + GAP + w(ws, "ws") + GAP
      + w(layout_text, "layout") + GAP + status_w() + ICON + S(4) + w(battery_text, "battery")
  end
  local function island_width()
    if PHONE then return theme.WIDTH end
    local name = island.page:get()
    if name == "" then return math.max(S(config.collapsedW), strip_total() + PAD * 3) end
    return page_width(name)
  end
  -- Centred in the pill; from the left edge on a phone, where the battery
  -- keeps the right edge like a status bar.
  local function start_x() return PHONE and PAD or (island_width() - strip_total()) / 2 end
  local function centred_y(node, name) return (PILL_H - h(node, name)) / 2 end
  -- The title moves right to make room for the back button when there is
  -- one, and the icon sits between.
  local function back_w() return (open() and island.depth:get() > 0) and S(36) or 0 end
  local function title_x() return PAD + back_w() + (has_icon() and (ICON + S(8)) or 0) end
  local function title_h() return S((big() and 26 or config.fontSize + 1) * 1.25) end
  local function gathered_x() return title_x() + w(clock, "clock") + S(8) end
  local function gathered_y() return PAD + S(4) end

  local function day_x() return open() and title_x() or start_x() end
  local function clock_x() return open() and title_x() or start_x() + w(day, "day") + GAP end
  local function ws_x()
    if open() then return slot_x("ws") or gathered_x() end
    return clock_x() + w(clock, "clock") + GAP
  end
  local function layout_x() return open() and gathered_x() or ws_x() + w(ws, "ws") + GAP end
  local status_x, glyph_x, battery_x
  if PHONE then
    battery_x = function()
      if open() then return slot_x("battery_text") or gathered_x() end
      return island_width() - PAD - w(battery_text, "battery")
    end
    glyph_x = function()
      if open() then return slot_x("battery_glyph") or (has_icon() and (PAD + back_w()) or gathered_x()) end
      return battery_x() - S(4) - ICON
    end
    status_x = function()
      if open() then return gathered_x() end
      return glyph_x() - status_w()
    end
  else
    status_x = function() return open() and gathered_x() or layout_x() + w(layout_text, "layout") + GAP end
    glyph_x = function()
      if open() then return slot_x("battery_glyph") or (has_icon() and (PAD + back_w()) or gathered_x()) end
      return status_x() + status_w()
    end
    battery_x = function()
      if open() then return slot_x("battery_text") or gathered_x() end
      return glyph_x() + ICON + S(4)
    end
  end
  local function shown_when_kept(name)
    return function()
      if not open() then return 1 end
      return slot(name) and 1 or 0
    end
  end

  local day_to, clock_to, ws_to, layout_to, battery_to, glyph_to
  day, day_to = morpher {
    key = "day", text = strip_words().day, color = C.muted, size = BASE.day,
    transform_origin_x = 0, transform_origin_y = 0,
    x = day_x,
    y = function()
      if open() then return PAD + title_h() + S(2) end
      return centred_y(day, "day")
    end,
    scale = function() return scale_of("day") end,
    opacity = function() return (open() or config.clockWeekday) and 1 or 0 end,
    behavior = { x = spring, y = spring, scale = spring, opacity = motion.fade },
  }
  clock, clock_to = morpher {
    key = "clock", text = strip_words().clock, font_weight = 700, size = BASE.clock,
    transform_origin_x = 0, transform_origin_y = 0,
    x = clock_x,
    y = function() return open() and PAD or centred_y(clock, "clock") end,
    scale = function() return scale_of("clock") end,
    behavior = { x = spring, y = spring, scale = spring },
  }
  ws, ws_to = morpher {
    key = "ws", text = strip_words().ws, font_weight = 700, size = BASE.ws,
    transform_origin_x = 0, transform_origin_y = 0,
    x = ws_x,
    y = function()
      if open() then return slot_y("ws") or gathered_y() end
      return centred_y(ws, "ws")
    end,
    scale = function() return scale_of("ws") end,
    opacity = shown_when_kept("ws"),
    behavior = { x = spring, y = spring, scale = spring, opacity = motion.fade },
  }
  layout_text, layout_to = morpher {
    key = "layout", text = strip_words().layout, size = BASE.layout,
    color = function() return hypr.state.keymap == "RU" and C.on or C.muted end,
    transform_origin_x = 0, transform_origin_y = 0,
    x = layout_x,
    y = function() return open() and gathered_y() or centred_y(layout_text, "layout") end,
    scale = function() return scale_of("layout") end,
    opacity = shown_when_kept("layout"),
    behavior = { x = spring, y = spring, scale = spring, opacity = motion.fade },
  }
  -- Ink, not the state's colour, once it sits on a page's circle that
  -- already wears that colour: a green bolt on a green disc is no bolt.
  local function glyph_color()
    if open() and slot("battery_glyph") and battery_color() ~= C.fg then return C.bg end
    return battery_color()
  end
  glyph, glyph_to = glyph_morpher(ICON, glyph_color, {
    x = glyph_x,
    y = function()
      if open() then return slot_y("battery_glyph") or (PAD + S(3)) end
      return (PILL_H - ICON) / 2
    end,
    opacity = function() return (not open() or has_icon() or slot("battery_glyph")) and 1 or 0 end,
    scale = function()
      if not open() then return 1 end
      local entry = slot("battery_glyph")
      if entry then return (entry.size or config.iconSize - 1) / (config.iconSize - 1) end
      return has_icon() and 1 or 0.4
    end,
    transform_origin_x = 0, transform_origin_y = 0,
    behavior = { x = spring, y = spring, opacity = motion.fade, scale = spring },
  })
  -- The phone's status icons, between the layout and the battery; they
  -- gather into the title with the rest when a page opens.
  status_row = require("status").build(ICON - S(2), GAP, {
    transform_origin_x = 0, transform_origin_y = 0,
    x = status_x,
    y = function() return open() and gathered_y() or centred_y(status_row, "status") end,
    scale = function() return scale_of("status") end,
    opacity = function() return open() and 0 or 1 end,
    behavior = { x = spring, y = spring, scale = spring, opacity = motion.fade },
  })
  battery_text, battery_to = morpher {
    key = "battery", text = strip_words().battery, size = BASE.battery,
    color = function() return slot("battery_text") and C.fg or C.muted end,
    font_weight = function()
      local entry = slot("battery_text")
      return entry and (entry.weight or 700) or 400
    end,
    transform_origin_x = 0, transform_origin_y = 0,
    x = battery_x,
    y = function()
      if open() then return slot_y("battery_text") or gathered_y() end
      return centred_y(battery_text, "battery")
    end,
    scale = function() return scale_of("battery") end,
    opacity = shown_when_kept("battery_text"),
    behavior = { x = spring, y = spring, scale = spring, opacity = motion.fade, color = motion.fade },
  }

  -- The words follow the machine while collapsed, and the page while open.
  local function want()
    local name, page = current_page()
    local words = strip_words()
    if name == "" then
      return words.day, words.clock, words.ws, words.layout, words.battery, battery_glyph()
    end
    local title = page.big and words.clock or line_of(page, "title")
    return line_of(page, "subtitle"), title, words.ws, words.layout, words.battery, page.icon or battery_glyph()
  end
  local function retarget()
    local a, b, c, d, e, g = want()
    day_to(a) clock_to(b) ws_to(c) layout_to(d) battery_to(e) glyph_to(g)
  end
  island.retarget = retarget
  morf.timer(100, retarget, true)

  -- The recorder's dot and time, only while it runs.
  local recording = ui.Row {
    gap = S(6), align = "center",
    visible = function() return island.recording:get() ~= "" and not open() end,
    x = function() return start_x() - S(70) end,
    y = function() return centred_y(clock, "clock") end,
    ui.Rect { width = S(8), height = S(8), radius = S(4), color = C.crit },
    theme.text { text = function() return island.recording:get() end, color = C.crit, font_weight = 700 },
  }

  local height = function()
    local name = island.page:get()
    if name == "" then return PILL_H end
    local entry = page_nodes[name]
    local wanted = entry and (entry.content.layout_height or 0) or 0
    if wanted <= 0 then return PILL_H end
    return math.min(S(config.expandedH), wanted + entry.top + PAD)
  end

  -- With `pillHover`, hovering the strip opens it too: the player if
  -- something plays, quick settings otherwise, after a moment, and it goes
  -- again once the pointer has left the island for a while. A page opened
  -- by a click or a key stays. Off by default.
  local hover_clock = morf.elapsed_timer()
  local hovering = false
  local opened_by_hover = false
  if config.pillHover then morf.timer(80, function()
    if hovering and not open() and hover_clock:elapsed_ms() > 260 then
      local media = require("media")
      opened_by_hover = true
      island.open(media.state.status == "Playing" and "media" or "main")
    elseif not hovering and open() and opened_by_hover and hover_clock:elapsed_ms() > 700 then
      opened_by_hover = false
      island.close()
    end
  end, true) end
  island.keep = function() opened_by_hover = false end
  -- Back, at the top left where a phone keeps it, when the page was
  -- reached from another; the title steps aside for it.
  local back = theme.button {
    width = S(30), height = S(30), radius = 15,
    color = "transparent",
    anchors = { left = true, top = true, left_margin = PAD - S(4), top_margin = PAD - S(2) },
    visible = function() return open() and island.depth:get() > 0 end,
    on_click = function() island.back() end,
    theme.icon { text = "󰅁", size = config.iconSize, anchors = { center_in = true }, color = C.muted },
  }
  local content = ui.Item {
    anchors = { fill = true },
    -- Collapsed, a click on the strip opens quick settings; on a phone
    -- the shade, and a finger pulled down the strip opens it too.
    ui.MouseArea {
      anchors = { fill = true },
      cursor = "pointer",
      visible = function() return not open() end,
      on_entered = function() hovering = true hover_clock:restart() end,
      on_exited = function() hovering = false hover_clock:restart() end,
      on_clicked = function() island.keep() island.open(island.first_page()) end,
      on_pressed = function() island.pull = 0 end,
      on_dragged = function(_, _, _, dy)
        if not island.pull then return end
        island.pull = island.pull + (dy or 0)
        if island.pull > S(36) then
          island.pull = nil
          island.keep()
          island.open(island.first_page())
        end
      end,
      on_released = function() island.pull = nil end,
    },
    -- Open, the pointer over the island keeps it, and a finger anywhere on
    -- the sheet pulls or sweeps it.
    ui.MouseArea {
      anchors = { fill = true },
      z = -1,
      visible = function() return open() end,
      on_pressed = function() theme.drag_begin() end,
      on_released = function() theme.drag_end() end,
      on_dragged = function(_, _, dx, dy) theme.drag(dx or 0, dy or 0) end,
      on_entered = function() hovering = true hover_clock:restart() end,
      on_exited = function() hovering = false hover_clock:restart() end,
    },
    ui.Item { anchors = { fill = true }, table.unpack(pages) },
    recording, day, clock, ws, layout_text, status_row, glyph, battery_text,
    back,
  }
  body = capsule(island_width, height, content)
  ready:set(true)
  return ui.Flex {
    direction = "row",
    justify = "center",
    align = "start",
    anchors = { left = true, right = true, top = true, top_margin = (NOTCH or PHONE) and 0 or S(config.islandGap) },
    body,
  }
end

-- ------------------------------------------------------------------ verbs --

-- Filled by init: opens and closes the expanded surface.
island.surface = { open = function() end, close = function() end }

--- Scrolls the open page by `steps` notches of the wheel.
function island.scroll_by(steps)
  local entry = page_nodes[island.page:get()]
  if not entry or steps == 0 then return end
  local most = math.max(0, (entry.content.layout_height or 0) - entry.room())
  entry.scroll:set(math.max(0, math.min(most, entry.scroll:get() + steps * S(64))))
end
theme.wheel = function(_, _, _, _, _, steps_y) island.scroll_by(steps_y) end

--- The page a tap or a pull on the strip opens: the shade on a phone --
--- notifications and a few tiles, as the first pull of a phone's shade
--- shows -- and everything at once on a desk.
function island.first_page()
  return (PHONE and island.pages.shade) and "shade" or "main"
end

--- A finger across the page: up and down scrolls it, following the
--- finger; sideways, over the tiles, turns their page once per gesture.
--- On a phone the shade is pulled further open, or swept shut: a pull
--- down past the top of the shade opens the whole of quick settings, and a
--- sweep up from the top of any page closes it.
local drag = { x = 0, y = 0, turned = false, axis = nil, done = false }
theme.drag_begin = function() drag.x, drag.y, drag.turned, drag.axis, drag.done = 0, 0, false, nil, false end
theme.drag_end = function() drag.axis = nil end
theme.drag = function(dx, dy)
  local name = island.page:get()
  local entry = page_nodes[name]
  if not entry or drag.done then return end
  drag.x, drag.y = drag.x + dx, drag.y + dy
  if not drag.axis and (math.abs(drag.x) > S(8) or math.abs(drag.y) > S(8)) then
    drag.axis = math.abs(drag.x) > math.abs(drag.y) and "x" or "y"
  end
  if drag.axis == "y" then
    local most = math.max(0, (entry.content.layout_height or 0) - entry.room())
    local at_top = entry.scroll:get() <= 0
    if PHONE and at_top and drag.y > S(70) then
      drag.done = true
      if name == "shade" then island.open("main") end
      return
    end
    if PHONE and at_top and drag.y < -S(70) and most == 0 then
      drag.done = true
      island.close()
      return
    end
    entry.scroll:set(math.max(0, math.min(most, entry.scroll:get() - dy)))
  elseif drag.axis == "x" and not drag.turned and math.abs(drag.x) > S(60) then
    drag.turned = true
    local tiles = require("tiles")
    if tiles.turn then tiles.turn(drag.x < 0 and 1 or -1) end
  end
end

function island.open(name, going_back)
  if not island.pages[name] then return false end
  local current = island.page:get()
  if current == name then
    island.close()
    return false
  end
  if current ~= "" then
    local leaving = page_nodes[current]
    if leaving then leaving.shown:set(false) end
    local page = island.pages[current]
    if page and page.on_close then page.on_close() end
    if not going_back then island.history[#island.history + 1] = current end
  end
  if not going_back and current == "" then island.history = {} end
  island.depth:set(#island.history)
  local entry = page_nodes[name]
  if not entry then return false end
  entry.scroll:set(0)
  island.expanded:set(true)
  island.surface.open()
  island.page:set(name)
  entry.shown:set(true)
  local page = island.pages[name]
  if page.on_open then page.on_open() end
  if island.retarget then island.retarget() end
  return true
end

function island.close()
  local current = island.page:get()
  if current == "" then return end
  local entry = page_nodes[current]
  if entry then entry.shown:set(false) end
  local page = island.pages[current]
  if page and page.on_close then page.on_close() end
  island.page:set("")
  island.history = {}
  island.depth:set(0)
  island.expanded:set(false)
  island.surface.close()
  if island.retarget then island.retarget() end
end

--- The page before this one, if it was reached from one.
function island.back()
  local previous = table.remove(island.history)
  if not previous then
    island.close()
    return
  end
  island.open(previous, true)
  island.depth:set(#island.history)
end

function island.toggle(name)
  if island.keep then island.keep() end
  if island.page:get() == name then island.close() else island.open(name) end
  return island.page:get()
end

--- A key on the expanded surface: Escape closes, the rest goes to the page.
function island.handle_key(keysym, text)
  local name = island.page:get()
  local page = island.pages[name]
  if page and page.on_key and page.on_key(keysym, text) then return end
  if keysym == 0xff1b then island.close() end
end

return island
