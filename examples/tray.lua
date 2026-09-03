-- A tray that brings its own watcher.
--
-- The watcher is `examples/lib/tray_watcher.lua`, served from this process;
-- the host is the engine's own `morf.status_notifier`, which then finds the
-- watcher beside it. Two halves that used to need two programs -- on a bare
-- session the host had nothing to register with and the tray stayed empty.
--
-- Run it, then start anything with a tray icon, and the count moves.
-- `morf ipc call tray` reports the addresses from a terminal.

local morf = require("morf")
local ui = require("morf.ui")
local tray_watcher = require("lib.tray_watcher")

morf.surface.height = 40

local watcher, why = tray_watcher.serve { replace = false }
local status = morf.signal("tray.status", watcher and "watching" or tostring(why))
local count = morf.signal("tray.count", 0)
local addresses = {}

-- The host, in the same process. It reads the watcher's list once and then
-- follows its signals, so an item that registers after this line still shows.
morf.status_notifier.subscribe(function(items)
  addresses = {}
  for index, item in ipairs(items) do
    addresses[index] = item.service .. item.path
  end
  count:set(#items)
end)

morf.ipc.tray = function()
  return table.concat(addresses, ",")
end
morf.ipc.watcher = function()
  return watcher and table.concat(watcher.list(), ",") or "no watcher"
end

ui.Rect {
  color = "#101418",
  ui.Row {
    spacing = 16,
    anchors = { left = true, margins = 12 },
    ui.Text {
      color = "#8a94a0",
      font_size = 16,
      text = function() return status:get() end,
    },
    ui.Text {
      color = "#ffffff",
      font_size = 16,
      text = function()
        local n = count:get()
        return n .. " tray item" .. (n == 1 and "" or "s")
      end,
    },
  },
}
