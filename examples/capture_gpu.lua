-- The same screen, captured twice: once through shared memory, once into a
-- dmabuf the compositor draws straight into.
--
-- The picture is the same either way; what differs is the trip. Through
-- shared memory the compositor copies the frame out of the GPU and this shell
-- uploads it back to draw a thumbnail. With `{ gpu = true }` the renderer
-- exports a texture, the compositor draws into it, and there is nothing to
-- copy: `frame.source` names the texture, `frame.pixels` is empty and
-- `frame.gpu` is true. Where the compositor or the GPU cannot do that, the
-- request is answered through shared memory and `frame.gpu` says so.
--
-- Started with `MORF_CAPTURE_READBACK=1` the engine reads the GPU capture
-- back once -- the one copy the path otherwise avoids -- so the two can be
-- compared here, pixel for pixel. `morf ipc call status` says how that went;
-- `morf ipc call again` takes both pictures again.

local morf = require("morf")
local ui = require("morf.ui")

morf.surface.height = 200
morf.surface.layer = "overlay"

local W, H = 320, 180

local status = morf.signal("capture.status", "capturing")
local frames = {}

local gpu_shot = ui.Image {
  x = 12, y = 10, width = W, height = H, fill_mode = "preserve_aspect_fit",
}
local shm_shot = ui.Image {
  x = 12 + W + 12, y = 10, width = W, height = H, fill_mode = "preserve_aspect_fit",
}
local gpu_label = ui.Text {
  x = 12, y = 10 + H - 18, text = "gpu", font_size = 12, color = "#ffffff",
}
local shm_label = ui.Text {
  x = 12 + W + 12, y = 10 + H - 18, text = "shm", font_size = 12, color = "#ffffff",
}
local line = ui.Text {
  x = 12 + 2 * (W + 12), y = 10, width = 600,
  text = function() return status:get() end,
  font_size = 13, color = "#e0e0e0",
}

--- Compares the two captures on a grid of sample points.
---
--- The blue, green and red bytes only: the fourth is padding in `xrgb8888`,
--- and the compositor may leave anything there.
local function compare()
  local a, b = frames.gpu, frames.shm
  if not a or not b then return end
  if not a.gpu then
    status:set("the gpu request was answered through shared memory")
    return
  end
  if #a.pixels == 0 then
    status:set("gpu capture is a texture; run with MORF_CAPTURE_READBACK=1 to compare pixels")
    return
  end
  if a.width ~= b.width or a.height ~= b.height then
    status:set(string.format("sizes differ: gpu %dx%d, shm %dx%d", a.width, a.height, b.width, b.height))
    return
  end
  -- A coarse grid, because a handler has a fuel budget and the point is not
  -- to compare every pixel but to catch a wrong layout, which wrecks all of
  -- them.
  local differing, total = 0, 0
  local step_x = math.max(1, math.floor(a.width / 24))
  local step_y = math.max(1, math.floor(a.height / 16))
  for y = 0, a.height - 1, step_y do
    for x = 0, a.width - 1, step_x do
      local ia = y * a.stride + x * 4 + 1
      local ib = y * b.stride + x * 4 + 1
      total = total + 1
      for c = 0, 2 do
        if math.abs(a.pixels:byte(ia + c) - b.pixels:byte(ib + c)) > 2 then
          differing = differing + 1
          break
        end
      end
    end
  end
  status:set(string.format("%dx%d: %d of %d sampled pixels differ between gpu and shm",
    a.width, a.height, differing, total))
end

local function shoot()
  frames = {}
  status:set("capturing")
  morf.screencopy.capture(false, function(frame, err)
    if err then
      status:set("gpu: " .. tostring(err))
      return
    end
    frames.gpu = frame
    gpu_shot.source = frame.source
    gpu_label.text = frame.gpu and "gpu  " .. frame.source or "shm (fallback)  " .. frame.source
    compare()
  end, { gpu = true })
  morf.screencopy.capture(false, function(frame, err)
    if err then
      status:set("shm: " .. tostring(err))
      return
    end
    frames.shm = frame
    shm_shot.source = frame.source
    shm_label.text = "shm  " .. frame.source
    compare()
  end)
end

morf.ipc.status = function() return status:get() end
morf.ipc.caps = function() return tostring(morf.capabilities.dmabuf_capture) end
morf.ipc.again = function()
  shoot()
  return "ok"
end

shoot()

ui.Rect {
  color = "#101418",
  gpu_shot, shm_shot, gpu_label, shm_label, line,
}
