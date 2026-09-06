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
local align = require("lib.align")
local tray_watcher = require("lib.tray_watcher")
local dbusmenu = require("lib.dbusmenu")

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

-- The first item's menu, read when asked. `morf ipc call menu` describes it
-- one entry per line -- label, id, and what kind of entry -- and
-- `morf ipc call click <id>` sends the click the application is waiting on.
local open_menu
local function describe(entries, depth)
  local lines = {}
  for _, entry in ipairs(entries) do
    local kind = entry.separator and "separator"
      or entry.children and "submenu"
      or entry.button_type and (entry.button_type .. ":" .. tostring(entry.checked))
      or "item"
    lines[#lines + 1] = string.rep("  ", depth) .. entry.text .. "(" .. entry.id .. ") " .. kind
    if entry.children then
      for _, line in ipairs(describe(entry.children, depth + 1)) do lines[#lines + 1] = line end
    end
  end
  return lines
end
morf.ipc.menu = function()
  if not addresses[1] then return "no items" end
  local menu, why = dbusmenu.for_item(addresses[1])
  if not menu then return tostring(why) end
  open_menu = menu
  return table.concat(describe(menu.entries(), 0), "\n")
end
morf.ipc.click = function(id)
  if not open_menu then return "open the menu first" end
  open_menu.click(tonumber(id))
  return "clicked " .. tostring(id)
end

-- The watcher's state at the left, the count at the right, the clock in
-- the middle: the bar layout, from `lib/align.lua`, filling the surface.
ui.Rect {
  color = "#101418",
  align.bar {
    anchors = { fill = true },
    gap = 12,
    ui.Text {
      color = "#8a94a0",
      font_size = 16,
      text = function() return status:get() end,
    },
    ui.Text {
      color = "#ffffff",
      font_size = 16,
      text = function() return morf.clock:get() end,
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
