-- Workspaces: every workspace on the focused monitor as a tile with the
-- windows on it, the active one marked; click to switch, and click a
-- window to focus it.

local morf = require("morf")
local ui = require("morf.ui")
local io = require("morf.io")
local config = require("config")
local theme = require("theme")
local hypr = require("hypr")

local S = theme.S
local C = theme.color

local page = {}

page.tiles = morf.list_model({})
local COLUMNS = 4

function page.width() return S(config.panelW * 1.5) end

--- Reads the windows and groups them by workspace.
function page.refresh()
  local monitor = hypr.state.monitor
  local reply = hypr.ask("j/clients", 120)
  if not reply then return end
  local ok, clients = pcall(io.json.decode, reply)
  if not ok or type(clients) ~= "table" then return end
  local by_workspace = {}
  for _, client in ipairs(clients) do
    local workspace = client.workspace or {}
    if client.mapped ~= false and type(workspace.id) == "number" and workspace.id > 0 then
      by_workspace[workspace.id] = by_workspace[workspace.id] or {}
      table.insert(by_workspace[workspace.id], {
        address = client.address, title = client.title or "", class = client.class or client.initialClass or "",
      })
    end
  end
  local tiles = {}
  for _, row in ipairs(hypr.rows) do
    local windows = by_workspace[row.id] or {}
    tiles[#tiles + 1] = {
      id = row.id, workspace = row.id, label = row.label, active = row.active,
      count = #windows, windows = windows,
    }
  end
  page.tiles:replace(tiles, "id")
end

function page.on_open()
  hypr.refresh()
  morf.timer(80, page.refresh, false)
end

local function glyph_for(class)
  local lower = class:lower()
  if lower:find("kitty") or lower:find("foot") or lower:find("alacritty") or lower:find("term") then return "󰆍" end
  if lower:find("firefox") or lower:find("zen") or lower:find("chrom") or lower:find("brave") then return "󰖟" end
  if lower:find("code") or lower:find("vim") or lower:find("zed") then return "󰨞" end
  if lower:find("spotify") then return "󰓇" end
  if lower:find("discord") or lower:find("telegram") or lower:find("slack") then return "󰭹" end
  if lower:find("file") or lower:find("nautilus") or lower:find("thunar") then return "󰉋" end
  return "󰖯"
end

function page.build(island)
  local W = page.width() - S(32)
  local TILE_W = math.floor((W - S(10) * (COLUMNS - 1)) / COLUMNS)
  local TILE_H = S(104)
  local function tile(item)
    local windows = {}
    for index, window in ipairs(item.windows) do
      if index > 3 then break end
      windows[#windows + 1] = ui.Item {
        width = TILE_W - S(24), height = S(18),
        ui.Row {
          gap = S(6), align = "center",
          theme.icon { text = glyph_for(window.class), size = config.iconSize - 5, color = C.muted },
          theme.text { text = window.title ~= "" and window.title or window.class, size = config.fontSize - 5,
            color = C.muted, width = TILE_W - S(50), elide = "right" },
        },
        ui.MouseArea {
          anchors = { fill = true }, cursor = "pointer",
          on_clicked = function()
            hypr.focus_window(window.address)
            island.close()
          end,
        },
      }
    end
    return theme.button {
      width = TILE_W, height = TILE_H,
      color = function() return item.active and C.on_tint or C.card end,
      hover_color = function() return item.active and C.on_tint or C.card_hover end,
      border_width = 1,
      border_color = item.active and C.on_edge or C.edge,
      on_click = function()
        hypr.go_to(item.workspace)
        island.close()
      end,
      ui.Column {
        gap = S(6),
        anchors = { left = true, top = true, margins = S(12) },
        ui.Item {
          width = TILE_W - S(24), height = S(18),
          theme.text { text = item.label, font_weight = 700, anchors = { left = true } },
          theme.text {
            text = item.count == 0 and "empty" or string.format("%d window%s", item.count, item.count == 1 and "" or "s"),
            size = config.fontSize - 5, color = C.muted, anchors = { right = true, top = true, top_margin = S(3) },
          },
        },
        table.unpack(windows),
      },
    }
  end
  return ui.Column {
    gap = S(10),
    theme.text { text = "Workspaces", font_weight = 700, size = config.fontSize + 1 },
    ui.Repeater { as = "grid", columns = COLUMNS, gap = S(10), model = page.tiles, delegate = tile },
    theme.text { text = "No workspaces reported", size = config.fontSize - 2, color = C.faint,
      visible = function() return page.tiles:len() == 0 end },
  }
end

return page
