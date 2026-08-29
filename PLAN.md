# mold — plan

**mold is a Wayland rendering and shell engine implemented in Rust.**

It provides native primitives for scene construction, layout, rendering, input,
surfaces, models, IO, and platform integration. Lua is the user-facing
configuration and extension interface to those Rust primitives.

**mold is not a widget toolkit and is not a desktop or phone shell.**

Buttons, sliders, bars, launchers, settings panels, notification centers, lock
screen presentation, themes, and complete shells belong to downstream projects.
patin is one such consumer. It is not part of mold and mold does not depend on
it, package it, or implement it.

---

## 1. Non-negotiable boundary

### mold owns the engine

The mold repository owns only reusable engine and platform facilities:

- process startup, shutdown, scheduling, and the Wayland event loop;
- the compositor frame clock and every per-frame operation;
- the reactive graph, models, properties, and native actions;
- the scene graph and primitive node types;
- layout, animation, hit testing, input routing, and focus;
- rendering, text, images, paths, effects, GPU resources, and damage;
- Wayland surfaces, outputs, seats, and supported protocols;
- process, file, socket, timer, D-Bus, and IPC primitives;
- low-level native service integrations that expose typed data and operations;
- Lua hosting, bindings, configuration loading, and plugin loading;
- native resource ownership, cleanup, recovery, and hot reload.

All of this is implemented in Rust.

### mold does not own widgets or shells

The mold repository must not contain:

- Button, Slider, Toggle, TextField, Card, Menu, or similar widgets;
- a widget library under any name;
- Bar, Launcher, Osk, LockScreen, NotificationCenter, NetworkSettings, or a
  complete shell composition;
- visual indicators such as Battery, Wifi, Clock, Volume, or Brightness;
- theme policy, product styling, navigation policy, or phone-shell state;
- a patin crate, patin package, or embedded patin Lua tree;
- downstream application behavior disguised as an engine primitive.

If an API has a product opinion, visual identity, or reusable control policy, it
belongs outside mold.

### Primitive versus widget

A primitive exists to let downstream code construct behavior without mold
choosing the product behavior.

Native mold primitives include:

- Item, Rectangle, Text, Image, Icon, Shape, and transform/clip nodes;
- pointer, touch, keyboard, focus, gesture, and input-region primitives;
- Row, Column, Grid, anchors, constraints, and implicit sizing;
- ListModel, Repeater, ListView, GridView, Flickable, and Loader;
- animation, state, transition, easing, spring, and timer primitives;
- panel, floating, popup, and session-lock surface primitives;
- typed IO, service, model, signal, and action handles.

A Button is not a primitive. It is a downstream composition of visual, input,
focus, state, accessibility, and action primitives. The same rule applies to
every higher-level control and shell component.

### Lua is the interface, not the runtime implementation

Lua exists so users and downstream projects can:

- assign engine settings;
- instantiate and compose Rust-exported primitives;
- configure native platform and service objects;
- bind keys and IPC names;
- register handlers at explicit extension points;
- implement optional downstream libraries and plugins;
- build a shell without modifying the engine.

The runtime underneath the interface remains Rust. mold must not ship its engine
or built-in primitives as a pile of Lua modules.

### Forcing tests

1. mold builds and its native primitive tests pass with no Lua source tree
   installed beside the binary.
2. require("mold") and require("mold.ui") resolve to module tables preloaded by
   the Rust host, not files under runtime/lua.
3. Deleting runtime/lua removes no engine primitive.
4. No downstream widget or patin module is required by a mold example, fixture,
   test, binary, or package.
5. A downstream project can build widgets and a shell using only the public
   primitive API, without mold knowing their names.

### Terminology

- runtime means the Rust mold process and its live native state;
- primitive means a reusable mechanism without product policy;
- widget means a downstream control composed from primitives;
- shell means a downstream product composed from primitives and widgets;
- Lua host means the Rust mold-lua crate embedding Luna;
- config means user Lua loaded through the public API;
- plugin means optional user or third-party Lua using the same API;
- $XDG_RUNTIME_DIR is OS storage for sockets and ephemeral process state;
- runtimepath is only an ordered Lua config/plugin search path;
- runtime/ is not a source directory for mold implementations.

---

## 2. Ownership and dependency direction

| layer | owner | mold relationship |
|---|---|---|
| Wayland process and event loop | mold Rust | engine |
| scene, layout, rendering, input | mold Rust | engine |
| models, IO, services, IPC | mold Rust | engine primitives |
| primitive Lua bindings | mold Rust | public interface |
| user configuration and plugins | user or downstream | consumer |
| widgets and component libraries | downstream projects | consumer |
| patin | separate downstream project | consumer |
| complete shell | downstream project | consumer |

Dependencies point one way:

    downstream shell or library
              |
              | Rust API and/or Lua primitive API
              v
    mold Rust engine and Lua host
              |
              v
    Wayland / GPU / operating system

mold never imports a downstream widget or shell package.

---

## 3. Repository layout

    mold/
    ├── crates/
    │   ├── mold-reactive/   native signals, derived values, batching
    │   ├── mold-scene/      native nodes, properties, models, views, animation
    │   ├── mold-layout/     native anchors, positioners, layouts, sizing
    │   ├── mold-text/       native shaping, metrics, glyph caching
    │   ├── mold-image/      native decode, SVG, icon themes, caches
    │   ├── mold-render/     native wgpu rendering, layers, effects, damage
    │   ├── mold-wayland/    native surfaces, protocols, outputs, input
    │   ├── mold-io/         native process, files, sockets, timers, D-Bus, IPC
    │   ├── mold-services/   low-level native platform integrations
    │   ├── mold-lua/        Rust host and primitive bindings for Luna
    │   └── mold-cli/        executable and command-line interface
    ├── examples/            primitive API examples
    └── tests/               engine, boundary, integration, and smoke tests

There is no:

- crates/mold-widgets;
- crates/patin;
- runtime/lua/patin;
- built-in Lua implementation tree.

The mold and mold.ui module tables are created and inserted into package.loaded
by mold-lua before config runs. require does not search the filesystem for
built-in engine modules.

---

## 4. Core Rust subsystems

### 4.1 Reactive core

mold-reactive provides:

- typed source signals and derived signals;
- dependency capture and topological recomputation;
- batched writes and glitch-free propagation;
- cycle detection with a named dependency chain;
- weak subscriptions and deterministic cleanup;
- diagnostics for fan-out, recomputation, and cost.

It knows nothing about widgets or shell policy.

### 4.2 Scene and properties

mold-scene provides:

- a generational node arena with stable handles;
- parent/child ownership and reparenting;
- typed properties with target and rendered values;
- dirty propagation for transform, layout, paint, and semantics;
- primitive elements and tree mutation;
- native models, repeaters, virtualized views, and loaders;
- native state, transition, and action machinery.

The scene crate exposes mechanisms from which downstream controls are built. It
does not expose Button or any other control.

### 4.3 Layout

mold-layout provides:

- implicit sizing;
- anchors and margins;
- row, column, grid, and stack positioners;
- min, preferred, and max constraints;
- hypothetical layout for transitions;
- fractional-scale-correct geometry.

Layout never calls Lua and never encodes widget styling or behavior.

### 4.4 Rendering, text, and images

mold-render, mold-text, and mold-image provide:

- wgpu Vulkan/GLES rendering;
- rectangles, rounded corners, borders, gradients, and paths;
- text shaping, metrics, and persistent glyph atlases;
- raster images, SVG, XDG icon themes, and caches;
- subtree layers, rounded clips, blur, shadows, opacity, and masks;
- draw-list diffing, damage tracking, scaling, and submission.

These crates render primitives. They do not know whether a tree represents a
button, bar, launcher, or anything else.

### 4.5 Input

mold-wayland and mold-scene provide:

- pointer, touch, tablet, and keyboard discovery;
- hit testing, grabs, drag thresholds, and contact identity;
- focus chains, key routing, text input, and accessibility semantics;
- low-level click, long-press, drag, swipe, and scrolling mechanics;
- input regions for click-through surfaces.

Input primitives report state and events. Downstream code decides what those
events mean for a widget or shell.

### 4.6 Surfaces and Wayland

mold-wayland provides:

- layer-shell panels;
- xdg toplevels;
- xdg popups;
- ext-session-lock surfaces;
- output selectors and native instances;
- output hotplug, transforms, fractional scale, and viewporter;
- virtual keyboard, input method, text input, screencopy, clipboard, and output
  power management where supported.

Session-lock surfaces are an engine primitive. A branded lock screen and its
presentation are downstream. Security-critical protocol and authentication
mechanisms stay native Rust.

### 4.7 IO and low-level services

mold-io and mold-services provide typed native mechanisms:

- process execution and streaming;
- file IO and inotify;
- Unix sockets, timers, D-Bus, and IPC;
- PipeWire graph and volume operations;
- PAM and greetd mechanisms;
- StatusNotifierItem protocol hosting;
- udev, xkb, logind, idle, and related platform integration.

These are data and operation primitives, not visual indicators or settings
panels. Downstream code chooses representation and policy.

---

## 5. Lua configuration and extension API

Lua is value-dominant configuration with a behavior edge. It follows the
lua-config-api contract: assign settings, register behavior, return nothing.

### 5.1 Native module preload

mold-lua is Rust and:

- creates the mold and mold.ui tables;
- inserts both into package.loaded before config executes;
- binds native constructors, values, models, signals, actions, and registrars;
- never loads built-in engine functionality from runtime/lua;
- contains no patin-specific names or behavior.

### 5.2 Settings are assigned

    local mold = require("mold")

    mold.render = {
      scale_policy = "fractional",
      damage_tracking = true,
    }

    mold.animation = {
      reduced_motion = false,
    }

Missing means unset and does not overwrite an environment or CLI choice. Nested
data remains nested.

### 5.3 Primitives are composed

    local mold = require("mold")
    local ui = require("mold.ui")

    mold.surface {
      role = "layer",
      anchors = { top = true, left = true, right = true },
      content = ui.Rectangle {
        height = 32,
        color = "#1f2430",
        ui.Text {
          x = 12,
          text = "configured downstream",
        },
      },
    }

Rectangle and Text are native Rust primitives. This example deliberately does
not use a widget or a shell package.

### 5.4 Behavior is registered

List registrars accumulate handlers in order:

    mold.on.output_added(function(output)
      mold.log.info("output added", output.name)
    end)

Keyed registrars replace by identity:

    mold.keys["Super+Space"] = function()
      user_shell.toggle_launcher()
    end

    mold.ipc["user-shell.toggle"] = user_shell.toggle_launcher

Handlers are optional user or downstream extensions. They are not the
implementation of mold primitives.

### 5.5 Config returns nothing

init.lua is assignments and registrations. It has no setup wrapper and returns
nothing. Load-time errors are fatal with file and line. Reload keeps the previous
valid configuration if the candidate fails.

### 5.6 Tables are arguments

Themes, primitive descriptions, presets, and plugin specs may be tables, but are
assigned or passed to a constructor or registrar. User config does not manually
merge untyped fragments.

### 5.7 Handler contracts

Each registrar documents exactly one return contract:

- side-effect handler: return value ignored;
- filter handler: nil means not handled and a typed value replaces;
- producer handler: typed return value is the result.

Runtime handlers execute one at a time under protected calls. One failure is
reported with its registration context and later handlers still run.

### 5.8 Host responsibilities

The Rust host:

- owns the VM, registry, limits, scheduling, and cleanup;
- stores list handlers in order and keyed handlers by identity;
- distinguishes missing from zero, false, or empty;
- discards the config chunk result;
- names load errors by file and line;
- protects and fuel-limits each runtime handler;
- builds an event value once and reuses it across handlers;
- keeps Lua off frame, layout, paint, damage, raw input routing, and
  security-critical paths.

---

## 6. Optional Lua plugins

Plugins are downstream configuration written by somebody else. They use the same
primitive API as init.lua and are never required by mold.

Follow the neovim path layout:

    <root>/
        plugin/**/*.lua       sourced automatically
        lua/**/*.lua          available to require, never auto-run
        after/plugin/**/*.lua sourced last

Rules:

- runtimepath is an ordered list of roots, not one hard-coded directory;
- files are sorted within directories and roots retain path order;
- plugin/ runs, lua/ is required, and after/ is the override seam;
- source is loaded as text, never unverified bytecode;
- plugin failures are isolated and remaining plugins continue;
- each plugin loads once;
- keyed registrations override;
- reload does not duplicate list registrations;
- clean and no-plugin startup modes exist;
- no plugin supplies a missing mold primitive.

mold ships no first-party Lua widget library on this path.

---

## 7. Runtime execution

### 7.1 Frame and event pipeline

    Wayland / timer / IO / service event
      |
      +--> Rust model update
      +--> Rust signal propagation and state transition
      +--> Rust native action dispatch
      +--> optional bounded downstream handler at an extension point
      +--> Rust animation tick
      +--> Rust layout
      +--> Rust scene-to-draw-list conversion
      +--> Rust damage calculation
      +--> wgpu submit + wl_surface commit

Lua never runs from the frame clock, layout, paint, damage, GPU submission, raw
input routing, authentication, or lock supervision.

### 7.2 Load and reload

1. Rust creates a fresh fuel-limited Luna VM.
2. Rust preloads native mold and mold.ui modules.
3. Rust loads init.lua and optional plugins as source.
4. Rust reads settings, primitive descriptions, and registrations.
5. Rust validates a candidate configuration.
6. Rust applies it atomically or keeps the previous configuration.
7. Rust retains the VM only when downstream handlers require it.
8. Rust drops the previous VM and its registrations.

### 7.3 IPC and CLI

The native socket lives at:

    $XDG_RUNTIME_DIR/mold/$WAYLAND_DISPLAY.sock

That is OS process state, not a Lua source directory.

- mold runs the Rust engine;
- mold -c NAME selects a configuration;
- mold ipc call TARGET.VERB ARGS invokes an allowlisted native or registered verb;
- mold ipc verbs lists the exposed surface from the first version;
- mold log and mold kill inspect or control the process.

The IPC server is Rust. Requests, replies, connections, timeouts, and encode
depth are bounded. Peer identity comes from SO_PEERCRED. There is no general eval
verb.

---

## 8. Security

- Wayland session-lock protocol handling is native Rust.
- PAM and greetd mechanisms are native Rust.
- Lua cannot replace authentication flow or protocol invariants.
- Plugin handlers cannot run on security-critical paths.
- PAM work stays off the event-loop thread and secrets have bounded lifetimes.
- Lua execution is fuel-metered, memory-bounded, protected, and interruptible.
- Native resources are released by Rust ownership, not Lua garbage collection.
- IPC exposes a small named surface and never arbitrary code execution.

Downstream code owns lock-screen presentation, but mold owns the correctness of
the primitives it exposes.

---

## 9. Corrective migration

The current runtime/lua/patin tree violates this plan. It is downstream code
embedded inside the engine repository.

### 9.1 Remove downstream ownership

- do not migrate patin widgets into a mold-widgets crate;
- do not migrate the patin shell into a Rust crate inside mold;
- do not add native Button, Slider, Bar, Launcher, indicator, or shell APIs;
- remove patin names and module registration from mold-lua;
- remove patin-specific fixtures, examples, and tests from mold;
- move any still-needed patin implementation to its own downstream repository.

### 9.2 Make built-in primitive modules native

- replace runtime/lua/mold/component.lua with an engine primitive implemented by
  Rust only if a generic component mechanism belongs in mold;
- otherwise remove it and let downstream code provide component helpers;
- create mold and mold.ui directly in Rust;
- set package.loaded entries before loading user config;
- keep filesystem require only for user and plugin modules.

### 9.3 Delete the implementation tree

- delete runtime/lua/patin;
- delete runtime/lua/mold after native preload is complete;
- delete runtime/;
- remove include_bytes references to runtime Lua;
- remove engine packaging of share/mold/runtime/lua;
- remove default module roots pointing at the repository runtime directory.

### 9.4 Preserve user extensibility

Removing built-in Lua does not remove Lua:

- user init.lua remains supported;
- downstream Lua libraries remain loadable;
- optional plugins remain supported;
- settings, primitive composition, registrations, and handlers remain public;
- runtimepath contains user and package roots, not engine implementation.

---

## 10. Milestones

| milestone | scope | done when |
|---|---|---|
| **R0 — boundary reset** | remove widget and shell ownership from mold architecture | no plan, crate, or public API claims mold owns widgets or patin |
| **R1 — native module preload** | build mold and mold.ui module tables entirely in Rust | require resolves both without reading built-in Lua files |
| **R2 — downstream removal** | remove embedded patin modules, fixtures, examples, and tests | mold source and tests contain no patin or widget implementation |
| **R3 — runtime directory removal** | delete runtime/ and built-in Lua packaging | mold builds and primitive tests pass with no runtime source directory |
| **R4 — primitive audit** | classify every exported type as an engine primitive | product-policy APIs are removed or moved downstream |
| **R5 — config registration API** | assign settings, register behavior, return nothing | missing values stay unset, handlers repeat, keyed maps replace, failures isolate |
| **R6 — plugin API** | ordered roots, plugin/lua/after split, reload safety, clean mode | optional plugins extend primitives without becoming dependencies |
| **R7 — native runtime audit** | keep Lua off engine-critical paths | profiler and tests show no Lua in frame, layout, paint, damage, raw routing, or security |
| **R8 — reload and IPC** | atomic VM/config replacement and bounded dispatch | bad reload keeps previous state and handler failures remain isolated |
| **R9 — hardware acceptance** | validate engine primitives on target hardware | Mali/panfrost rendering, surfaces, input, scaling, and hotplug pass on device |

---

## 11. Acceptance gates

### Architecture

- mold contains no widget library;
- mold contains no patin implementation or dependency;
- mold contains no complete shell composition;
- the repository has no runtime/ implementation directory;
- every built-in module and primitive is implemented in Rust;
- mold and mold.ui are preloaded native modules;
- downstream code depends on mold, never the reverse.

### Primitive API

- scene, layout, rendering, input, surfaces, models, IO, and native services are
  usable without any widget abstraction;
- primitives contain no product styling or shell policy;
- downstream code can compose controls without mold recognizing their names;
- generic component helpers are either truly primitive or external.

### Lua API

- settings are assigned and missing values stay unset;
- nested configuration stays nested;
- behavior registrations repeat in order;
- identified registrations use replaceable maps;
- init.lua and auto-run plugins return nothing;
- load failures name file and line;
- handler failures are isolated with protected calls;
- plugin/ runs, lua/ is required, and after/ runs last;
- reload does not duplicate registrations;
- clean and no-plugin modes exist.

### Runtime and performance

- no Lua executes in frame, layout, paint, damage, GPU submission, raw input
  routing, authentication, or lock supervision;
- a runaway handler is interrupted without hanging the engine;
- virtualized views handle at least 500 entries at target refresh rate;
- animations retarget without position or velocity jumps;
- GPU acceptance runs on real Mali/panfrost hardware, not only llvmpipe.

Use the repository recipes:

    oslo make test
    oslo make verify
    oslo make gpu-smoke
    oslo make wayland-smoke

Hardware-only failures remain explicit. Local tests do not pretend to prove
unavailable device behavior.

---

## 12. Risks

- **Current tests treat patin as an engine feature.** Delete or move those tests;
  replace them with primitive-boundary tests.
- **Primitives can quietly become widgets.** Reject APIs with product styling,
  control semantics, or shell policy.
- **Removing runtime/lua can accidentally remove user require.** Preserve
  filesystem loading for user and plugin roots while removing built-in roots.
- **A broad plugin API can expose internal ownership.** Bind typed public values,
  models, actions, and events rather than scene internals.
- **A Lua handler can stall the event loop.** Fuel-limit and interrupt it, and
  keep handlers off frame and security paths.
- **Reload can duplicate behavior.** Replace VM registries atomically and use
  keyed registration where identity exists.
- **Luna is pre-1.0.** Contain its churn inside mold-lua and pin exact revisions.
- **wgpu differs on target Mali hardware.** Keep real-device checks from the
  first useful primitive renderer.

---

## 13. Salvage

Keep Rust work that matches the engine boundary:

- reactive graph, scene arena, properties, layout, models, and views;
- wgpu rendering, text, images, paths, layers, clips, blur, and shadows;
- Wayland surfaces, protocols, frame callbacks, outputs, and input routing;
- low-level PipeWire, PAM, greetd, tray, udev, xkb, and IO mechanisms;
- fractional scale in 120ths and viewporter support;
- capability-driven behavior instead of hardware-name branching;
- Luna embedding as a Rust-hosted public primitive interface.

Remove or move downstream:

- every Lua patin module;
- every widget, indicator, and complete shell implementation;
- patin-specific tests and examples;
- runtime/lua as an engine implementation layer;
- any proposed mold-widgets or in-repository patin crate;
- any architecture where mold knows the names or policies of consumer controls.

The final product is a pure-Rust Wayland engine exposing native primitives through
Rust and Lua. Widgets and shells are built elsewhere.
