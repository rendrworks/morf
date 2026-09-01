-- morf's build, as recipes. This replaced the Makefile; there is no other.
--
--   make            the recipes, with what each of them says it does
--   make build      the binary
--   make test       the suite
--   make verify     the whole local gate
--
-- At an oslo prompt in this directory `make` is enough; everywhere else it is `oslo make`.
-- CI has no oslo, so it calls the language's own tool -- nothing here is on the release path.

local make = oslo.make

-- Name and version live in PROJECT, one per line, so every tool reads them from one place.
local function project()
  local found = {}
  for line in (oslo.fs.read("PROJECT") or ""):gmatch("[^\n]+") do
    local value = line:match("^%s*([^#%[%s]%S*)%s*$")
    if value then found[#found + 1] = value end
  end
  return found[1] or "morf", found[2] or "0.1.0"
end

local NAME, VERSION = project()
local PREFIX = os.getenv("PREFIX") or (os.getenv("HOME") .. "/.local")

------------------------------------------------------------------ what was built

local function dim(text)
  return oslo.ui.style(text, { dim = true })
end

local function line(label, value)
  print(dim(oslo.ui.pad(label, 8)) .. value)
end

-- `1524720` -> `1,524,720`. A number this long is read in groups or not at all.
local function grouped(n)
  local text = tostring(math.floor(n))
  local out = text:sub(-3)
  local at = #text - 3
  while at > 0 do
    out = text:sub(math.max(1, at - 2), at) .. "," .. out
    at = at - 3
  end
  return out
end

-- Asked of the ELF, not assumed. `ldd` is not enough on its own: it prints "statically linked" for
-- a binary that still carries an INTERP and will not start.
local function linkage(path)
  local segments = oslo.run{ "readelf", "-l", path, capture = true }
  if not segments.ok then return nil end
  local dynamic = oslo.run{ "readelf", "-d", path, capture = true }
  if (segments.out or ""):find("program interpreter") or (dynamic.out or ""):find("NEEDED") then
    return "dynamic"
  end
  return "static"
end

-- What was built, how big it is, and whether it needs anything on the target machine. Silent when
-- the artifact is not there, so a recipe that builds nothing does not pretend it did.
local function report(path)
  local stat = oslo.fs.stat(path)
  if not stat then return end
  local megabytes = ("%.2f MB"):format(stat.size / 1048576)

  print("")
  print(oslo.ui.title(("%s %s   %s"):format(NAME, VERSION, megabytes)))
  line("binary", path)
  -- Bytes beside megabytes: `1.45 MB` cannot be subtracted from last week's `1.42 MB` to get one.
  line("size", megabytes .. dim("   " .. grouped(stat.size) .. " bytes"))

  local kind = linkage(path)
  if kind == "static" then
    line("linking", oslo.ui.style("✓ static", { fg = "green" }) ..
                    dim("   no runtime dependencies"))
  elseif kind == "dynamic" then
    line("linking", oslo.ui.style("dynamic", { fg = "yellow" }) ..
                    dim("   needs a matching libc on the target machine"))
  end
  print("")
end

-- The same, for artifacts whose exact path the build system decides. Walked with find rather than
-- globbed: oslo's `**` matches a single directory level, and build trees nest deeper than that.
local function report_found(root, pattern)
  local found = oslo.run{ "find", root, "-type", "f", "-name", pattern, capture = true }
  for path in (found.out or ""):gmatch("[^\n]+") do
    report(path)
    return
  end
end


make.recipe{ name = "version", desc = "what this checkout calls itself",
             run = function() print(("%s v%s"):format(NAME, VERSION)) end }

local function need(tool, why)
  assert(oslo.run{ "sh", "-c", "command -v " .. tool, capture = true }.ok, why)
end

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    need("git-rel", "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}

make.recipe{
  name = "changelog",
  desc = "regenerate CHANGELOG.md",
  run = function()
    need("git-cliff", "git-cliff is not installed; install it first")
    sh.git("cliff", "-o", "CHANGELOG.md")
  end,
}

---------------------------------------------------------------------------- rust

local EXAMPLE = os.getenv("EXAMPLE") or "examples/quickshell/init.lua"

make.recipe{ name = "build", desc = "the workspace",
             run = function()
               sh.cargo("build", "--workspace")
               report("target/debug/morf")
             end }
make.alias("b", "build")

make.recipe{
  name = "run",
  desc = "run a configuration: --example PATH",
  params = { { "--example", desc = "path to the Lua configuration", default = EXAMPLE } },
  run = function(a)
    local script = a.example or EXAMPLE
    sh.cargo("build", "--package", "morf-cli")
    local command = { "target/debug/morf", script }
    local wrapper = oslo.run{ "sh", "-c", "command -v nixVulkan", capture = true }
    if wrapper.ok then
      command = { (wrapper.out or ""):match("[^\n]+"), command[1], command[2] }
    end
    assert(oslo.run(command).ok, "morf exited with an error")
  end,
}
make.alias("r", "run")

make.recipe{ name = "test", desc = "the suite",
             run = function() sh.cargo("test", "--workspace", "--all-targets") end }
make.alias("t", "test")

make.recipe{
  name = "gpu-smoke",
  desc = "initialize the GPU and submit an SDF frame",
  run = function()
    sh.cargo("build", "--package", "morf-render", "--example", "gpu_smoke")
    local command = { "target/debug/examples/gpu_smoke" }
    local wrapper = oslo.run{ "sh", "-c", "command -v nixVulkan", capture = true }
    if wrapper.ok then
      command = { (wrapper.out or ""):match("[^\n]+"), command[1] }
    end
    assert(oslo.run(command).ok, "GPU smoke failed")
  end,
}

make.recipe{
  name = "config-smoke",
  desc = "build every example's shaders and draw a frame on the GPU",
  run = function()
    -- The CPU gates say a configuration loads, lays out and paints. None of
    -- them say the driver will accept the shaders it declared: a pipeline that
    -- fails validation looks identical from the CPU side, and the first sign
    -- of it is a black screen or a panic in front of whoever ran it. This
    -- builds the pipelines and draws, for every example there is.
    sh.cargo("build", "--package", "morf-cli", "--example", "frame_bench")
    local wrapper = oslo.run{ "sh", "-c", "command -v nixVulkan", capture = true }
    local prefix = wrapper.ok and (wrapper.out or ""):match("[^\n]+") or nil
    local listed = oslo.run{ "sh", "-c", "ls examples/*.lua", capture = true }
    assert(listed.ok, "no examples to check")
    local checked = 0
    for path in (listed.out or ""):gmatch("[^\n]+") do
      local command = { "target/debug/examples/frame_bench", path, "gpu" }
      if prefix then
        command = { prefix, command[1], command[2], command[3] }
      end
      assert(oslo.run(command).ok, path .. " does not render")
      checked = checked + 1
    end
    print(("%d configurations rendered"):format(checked))
  end,
}

make.recipe{
  name = "wayland-smoke",
  desc = "present a layer surface and receive its frame callback",
  run = function()
    sh.cargo("build", "--package", "morf-wayland", "--example", "layer_smoke")
    local command = { "target/debug/examples/layer_smoke" }
    local wrapper = oslo.run{ "sh", "-c", "command -v nixVulkan", capture = true }
    if wrapper.ok then
      command = { (wrapper.out or ""):match("[^\n]+"), command[1] }
    end
    assert(oslo.run(command).ok, "Wayland smoke failed")
  end,
}

make.recipe{
  name = "popup-smoke",
  desc = "present an xdg popup anchored to a layer-surface click",
  run = function()
    sh.cargo("build", "--package", "morf-wayland", "--example", "popup_smoke")
    local command = { "target/debug/examples/popup_smoke" }
    local wrapper = oslo.run{ "sh", "-c", "command -v nixVulkan", capture = true }
    if wrapper.ok then
      command = { (wrapper.out or ""):match("[^\n]+"), command[1] }
    end
    assert(oslo.run(command).ok, "popup smoke failed")
  end,
}

make.recipe{
  name = "io-smoke",
  desc = "exchange bytes through a Unix-domain socket",
  run = function()
    sh.cargo("run", "--package", "morf-io", "--example", "socket_smoke")
  end,
}

make.recipe{
  name = "dbus-smoke",
  desc = "call and introspect the session message bus",
  run = function()
    sh.cargo("run", "--package", "morf-io", "--example", "dbus_smoke")
    sh.cargo("run", "--package", "morf-lua", "--example", "dbus_smoke")
  end,
}

make.recipe{
  name = "pam-smoke",
  desc = "load PAM and reject invalid credentials",
  run = function()
    sh.cargo("run", "--package", "morf-services", "--example", "pam_smoke")
  end,
}

make.recipe{
  name = "pipewire-smoke",
  desc = "enumerate the native PipeWire graph and round-trip sink volume",
  run = function()
    sh.cargo("run", "--package", "morf-services", "--example", "pipewire_smoke")
    sh.cargo("run", "--package", "morf-lua", "--example", "pipewire_smoke")
  end,
}

make.recipe{
  name = "udev-smoke",
  desc = "open the native kernel uevent monitor",
  run = function()
    sh.cargo("run", "--package", "morf-services", "--example", "udev_smoke")
  end,
}

make.recipe{ name = "test-all", desc = "the suite, with every feature on",
             run = function()
               sh.cargo("test", "--workspace", "--all-targets", "--all-features")
             end }

make.recipe{ name = "check", desc = "type-check every target",
             run = function() sh.cargo("check", "--workspace", "--all-targets") end }

make.recipe{
  name = "quickshell-inventory",
  desc = "inventory the pinned Quickshell API reference",
  run = function()
    assert(oslo.run{ "sh", "tools/quickshell-api-inventory.sh" }.ok,
           "Quickshell API inventory failed")
  end,
}

make.recipe{ name = "check-all", desc = "type-check every target, every feature",
             run = function()
               sh.cargo("check", "--workspace", "--all-targets", "--all-features")
             end }

make.recipe{ name = "clippy", desc = "clippy, with warnings denied",
             run = function()
               sh.cargo("clippy", "--workspace", "--all-targets", "--all-features", "--",
                        "-Dwarnings")
             end }

make.recipe{
  name = "rustdoc",
  desc = "build the docs, with warnings denied",
  run = function()
    local built = oslo.run{ "env", "RUSTDOCFLAGS=-Dwarnings",
                            "cargo", "doc", "--workspace", "--all-features", "--no-deps" }
    assert(built.ok, "rustdoc failed")
  end,
}

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() sh.cargo("fmt", "--all") end }

make.recipe{ name = "fmt-check", desc = "fail if anything is unformatted",
             run = function() sh.cargo("fmt", "--all", "--", "--check") end }

make.recipe{
  name = "rust-loc-check",
  desc = "fail when a Rust source exceeds 500 lines",
  run = function()
    local listed = oslo.run{
      "git", "ls-files", "--cached", "--others", "--exclude-standard", "--", "*.rs",
      capture = true,
    }
    assert(listed.ok, "could not inventory Rust sources")
    local oversized = {}
    for path in (listed.out or ""):gmatch("[^\n]+") do
      if not path:match("^xtra/") then
        local source = oslo.fs.read(path) or ""
        local _, lines = source:gsub("\n", "")
        if #source > 0 and source:sub(-1) ~= "\n" then lines = lines + 1 end
        if lines > 500 then
          oversized[#oversized + 1] = ("%s: %d lines"):format(path, lines)
        end
      end
    end
    table.sort(oversized)
    assert(#oversized == 0,
           "Rust sources exceed 500 lines:\n" .. table.concat(oversized, "\n"))
  end,
}

make.recipe{
  name = "boundary-check",
  desc = "enforce the engine-only repository boundary",
  run = function()
    assert(not oslo.fs.stat("runtime"), "runtime/ must not contain engine implementations")
    assert(not oslo.fs.stat("crates/morf-widgets"), "widgets belong downstream")
    assert(not oslo.fs.stat("crates/patin"), "patin belongs downstream")
    local scan = oslo.run{
      "grep", "-RIl", "--exclude-dir=target", "-E", "patin|morf-widgets",
      "crates", "examples", capture = true,
    }
    assert(not scan.ok, "downstream widget or shell ownership leaked into morf")
  end,
}

make.recipe{ name = "clean", desc = "remove every build output",
             run = function() sh.cargo("clean") end }

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

make.recipe{
  name = "verify",
  desc = "the whole local gate",
  deps = { "boundary-check", "rust-loc-check", "fmt-check", "check", "test",
           "check-all", "test-all", "clippy", "rustdoc" },
}
make.alias("v", "verify")
