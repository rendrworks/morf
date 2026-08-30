-- A port of `~/.config/quickshell` onto mold primitives.
--
-- The original is four Quickshell processes — `border`, `line`, `osd`, and the
-- root shell — each opening its own layer surfaces, twelve for the border
-- alone. mold binds one IPC socket per Wayland display and hosts one layer
-- surface per process, so the whole thing composes into a single fullscreen
-- overlay here, with the input region trimmed to the parts that take a click.
--
-- Everything below is a composition of Item, Rect, Shape, Text, and MouseArea.
-- The engine supplies the primitives, the frame clock, and the process and
-- timer services; nothing about a "ribbon" or a "workspace pill" is known to it.

local mold = require("mold")
local ui = require("mold.ui")
local core = require("mold.core")
local theme = require("theme")
local hypr = require("hypr")
local border = require("border")
local line = require("line")
local osd = require("osd")
local settings = require("settings")

local WIDTH, HEIGHT = theme.reference()

-- The original runs one ribbon per screen, each following its own monitor.
-- mold draws a single scene onto every output, so this follows one: whichever
-- `MOLD_MONITOR` names, else the focused one.
hypr.set_monitor(core.env("MOLD_MONITOR"))

mold.surface.namespace = "mold-quickshell"
mold.surface.width = WIDTH
mold.surface.height = HEIGHT
mold.surface.anchors = { top = true, left = true, right = true, bottom = true }
mold.surface.layer = "top"
mold.surface.keyboard_focus = "none"
-- The frame itself ignores exclusion, exactly as Border.qml's Edge and Corner
-- windows do — it is drawn over whatever is beneath it.
mold.surface.exclusive_zone = -1

-- The reservation is separate, and is what actually moves windows. Border.qml
-- does it with four zero-size `Reserve` windows, one per edge, each claiming an
-- exclusive zone; `mold.surface.reserve` opens the same four surfaces so the
-- compositor shrinks the tiling area and no window sits under the frame.
local reserved = border.reserved(WIDTH, HEIGHT)
mold.surface.reserve = {
  top = reserved,
  right = reserved,
  bottom = reserved,
  left = reserved,
}

-- The settings panels hang off the ribbon, so they are placed with the same
-- track geometry `line` computes for its pills.
local SHORT_SIDE = math.min(WIDTH, HEIGHT)
local bar_width = line.bar_width()
local track_height = HEIGHT * 0.5
local pill_gap = SHORT_SIDE * (10 / 2160)
local item_height = (track_height - pill_gap * 9) / 10
local track_top = (HEIGHT - track_height) / 2

-- Only the parts that must take a click do. Everything else — the frame, the
-- badge as it slides over a window, a panel while it is hidden — stays
-- transparent to the pointer, so the desktop underneath keeps working.
local regions = {}
for _, region in ipairs(line.regions()) do regions[#regions + 1] = region end
for _, region in ipairs(osd.regions()) do regions[#regions + 1] = region end
regions[#regions + 1] = settings.input_region(bar_width, track_top, item_height, pill_gap)
mold.surface.mask = { x = 0, y = 0, width = 0, height = 0, regions = regions }

ui.Item {
  width = WIDTH,
  height = HEIGHT,
  border.build(WIDTH, HEIGHT),
  line.build(),
  settings.build(bar_width, track_top, item_height, pill_gap),
  osd.build(),
}
