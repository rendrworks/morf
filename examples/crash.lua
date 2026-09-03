-- What the person sees when the shell died.
--
-- A crash report on disk is for whoever debugs it; this is for whoever was
-- using the shell, who otherwise sees a bare compositor and no idea why. The
-- crash handler starts it when `MORF_CRASH_SCREEN` names this file:
--
--   MORF_CRASH_SCREEN=/path/to/crash.lua morf shell.lua
--
-- and hands it the report's path as its one argument. It shows where the
-- fault was and where the report went, and goes away on a click.

local morf = require("morf")
local ui = require("morf.ui")

morf.surface.height = 120
morf.surface.layer = "overlay"

local report = morf.operands[1] or "(no report)"
local head = {}
local ok, text = pcall(function() return morf.file(report):read() end)
if ok and type(text) == "string" then
  -- The first lines say what and where; the backtrace below them is for the
  -- file, not the screen.
  local count = 0
  for line in text:gmatch("[^\n]+") do
    if line == "" or count >= 4 then break end
    head[#head + 1] = line
    count = count + 1
  end
else
  head[1] = "the report could not be read: " .. tostring(text)
end

ui.MouseArea {
  on_clicked = function() morf.quit() end,
  ui.Rect {
    color = "#3a1414",
    ui.Column {
      gap = 4,
      anchors = { left = true, top = true, margins = 12 },
      ui.Text { text = "morf stopped", color = "#ffffff", font_size = 20, font_weight = 700 },
      ui.Repeater {
        model = morf.list_model(head),
        delegate = function(line)
          return ui.Text { text = line, color = "#f0d0d0", font_size = 14 }
        end,
      },
      ui.Text { text = "report: " .. report .. "  (click to dismiss)", color = "#c09090", font_size = 13 },
    },
  },
}
