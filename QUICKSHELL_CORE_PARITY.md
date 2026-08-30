# Quickshell parity contract

Reference: `xtra/quickshell` at
`2d3b3e9c70ef380dff751b61d334dc88df016c29`.

Mold uses Quickshell as a source reference for reusable shell-engine
capabilities. Mold does not embed QML, copy Quickshell's implementation, move
engine behavior into Lua, or own widgets and complete shells.

The exhaustive current-state ledger is
[`QUICKSHELL_CORE_AUDIT.md`](QUICKSHELL_CORE_AUDIT.md). That audit covers the
whole pinned checkout and is authoritative when this summary and the detailed
ledger differ.

The focused animation comparison and runnable transformation proof are in
[`docs/ANIMATION_PARITY.md`](docs/ANIMATION_PARITY.md).

## Boundary

Included in the engine target:

- lifecycle, reload, persistence, reactive state, models, components, IO, and
  IPC;
- scene, rendering, text, images, paths, effects, layout, animation, focus,
  input, and views;
- panel, floating, popup, session-lock, output, and reusable Wayland protocol
  primitives;
- low-level typed service integrations selected for Mold's platform layer;
- Rust-owned Lua bindings that configure and compose the native mechanisms.

Excluded:

- Mold-owned buttons, sliders, fields, cards, menus as visual controls,
  indicators, bars, launchers, lock-screen presentation, notification centers,
  settings panels, and complete shells;
- X11-only, i3/Sway-specific, Hyprland-specific, and other vendor APIs unless a
  separate milestone explicitly includes them;
- Qt/QML/C++ implementation ABI details that are not public engine mechanisms.

Service and protocol data primitives are not widgets. They may be deferred,
but the widget boundary cannot be used to call an absent integration complete.

## Completion rule

An area is complete only when every included public capability has:

1. an equivalent native Rust mechanism;
2. a typed, bounded Lua configuration surface where user access is required;
3. deterministic ownership, cleanup, reload, mutation, and failure behavior;
4. local tests and live evidence where external state is required;
5. accurate documentation and a primitive consumer example.

Names may follow Mold conventions. Behavioral reductions and missing lifecycle
semantics do not count as parity.

## Current status

| Area | Status |
|---|---|
| runtime topology and lifecycle | incomplete; shell-global ownership and activation barriers are missing |
| reactive core | partial; graph is strong, effect ownership and animation notification are incomplete |
| core models/components | partial; Variants and BoundComponent are missing, Loader and models are reduced |
| IO and IPC | partial; async, ownership, boundedness, adapter, and introspection gaps remain |
| visual primitives | partial; core rendering works, but item/text/image/shape/effect breadth is reduced |
| layout, views, input, focus, animation | partial |
| general windows | partial/close for layer, own toplevel, and popup surfaces |
| general Wayland extras | incomplete; several reusable protocols are missing |
| optional desktop integrations | mixed and mostly absent or reduced |
| widget and shell compositions | intentionally downstream |
| X11/i3/Hyprland APIs | intentionally outside the current general target |

There is currently no broad “implemented, acceptance pending” parity claim.
Build and smoke gates validate only the lanes they exercise.
