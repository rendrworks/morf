# Writing UI in morf

How a configuration puts things on screen: the node kinds, how they are
sized and placed, how state reaches them, and what makes a frame. Every
section has a snippet you can paste into a config and run.

Three questions have three separate answers, and every feature here
belongs to exactly one of them:

- **Where does a node go?** Layout. Decided by the node's *parent*, and by
  only one kind of parent at a time.
- **Where does a property's value come from?** State. Decided per property:
  a literal, a binding, or a state block.
- **How is a piece of UI organised?** Structure: a function returning a
  node, a component with a model, a list with a delegate.

Layout kinds compete with each other for one node's placement. Value
sources compete with each other for one property. Everything else nests.

## 1. Nodes

`local ui = require("morf.ui")`. A node is a table constructor: named keys
are properties, the array part is the children.

```lua
ui.Rect {
  width = 200, height = 40, radius = 8, color = "#1b2128",
  ui.Text { x = 12, y = 10, text = "hello", color = "#ffffff" },
}
```

| group | kinds |
|---|---|
| containers | `Item`, `Inset`, `Flickable`, `Loader`, `Layout` |
| painting | `Rect`, `ClipRect`, `Text`, `Image`, `Icon`, `Sdf`, `SdfShape` |
| input | `MouseArea` (the only kind the pointer can hit) |
| positioners | `Row`, `Column`, `Grid` with `columns` |
| layouts | `Flex`, `Grid` with tracks (`RowLayout`, `ColumnLayout`, `GridLayout` are their older names) |
| lists | `Repeater`, `ListView`, `GridView`, `each` |
| other | `Timer`; `reparent`, `spring`, `smoothed` |

Every property of every kind is in one schema
(`crates/morf-scene/src/schema.rs`); an unknown name is an error at
construction, with the kind and the name in it.

A node without a parent is a root, and a surface draws its roots. A child
of a node is drawn inside it, after it. Paint order among siblings is tree
order, then `z`: a higher `z` paints over, and is hit before, its siblings
whatever its place in the tree.

## 2. Sizes

A node's requested size is, in order: its own `width`/`height` when
positive; `layout.preferred_width`/`preferred_height`; its
`implicit_width`/`implicit_height` when positive; else its measured
implicit size. Then it is clamped by `layout.minimum_*` and `maximum_*`.

Implicit sizes: text is shaped (at its own width when it has one, else
unconstrained, then again at the width its parent gave it if it wraps or
elides); an image is its pixel size, overridable by `source_width` and
`source_height`; a positioner is the sum or max of its children; anything
else is the bounding box of its children's `x + width` and `y + height`.

`width = 0` and no width are the same thing. A node cannot ask to be zero
wide; it can be `visible = false`.

What the frame actually gave a node is readable as `node.layout_x`,
`layout_y`, `layout_width`, `layout_height`. A binding that reads one is
re-run when a frame moves the node:

```lua
local box = ui.Item { anchors = { fill = true } }
ui.Rect { height = 10, width = function() return (box.layout_width or 0) / 2 end }
```

Text: `wrap`, `elide = "left" | "middle" | "right"` for a single line,
`max_lines` for wrapped text (it keeps that many lines and elides the
last), `horizontal_alignment`, `vertical_alignment`. `max_lines` without
`wrap`, or `elide` with it, is an error rather than nothing.

## 3. Placement

Each container kind owns the placement of its direct children, and reads
particular keys on them. The wrong kind's keys are errors.

| parent kind | what places the child | child-side keys |
|---|---|---|
| `Item`, `Rect`, any plain node | the child's own `x`, `y`, `anchors` | `anchors` |
| `Row`, `Column`, `Grid` (fixed `columns`) | packing order, `gap`, `align`, `justify` | `layout.align_self` |
| `Flex`, `Grid` with `template_columns/rows` | flexbox, grid tracks | `layout.grow`, `shrink`, `basis`, `align_self`, `margin`, `width`, `height`, `minimum_*`, `maximum_*`, `column`, `row`, `column_span`, `row_span` |
| `Inset` | margins around its one child | — |
| `Layout { measure, place }` | your two functions | whatever they read |

### Anchors

Relative to the parent only: `fill`, `center_in`, `left`, `right`, `top`,
`bottom` as booleans, `margins` and `left_margin` etc. as numbers.
`left` and `right` together stretch the width; likewise `top`/`bottom`.

```lua
ui.Rect { anchors = { left = true, right = true, top = true, margins = 8 }, height = 30 }
```

Anchors inside a positioner on the axis it packs, or anywhere inside a
`Flex` or a track `Grid`, are errors: one kind places a node.

### One vocabulary

Every container that packs children reads the same three words. `gap` is
the space between children. `align` places them across the packed axis:
`start`, `center`, `end`, `stretch`. `justify` distributes leftover room
along it: `start`, `center`, `end`, `space_between`, `space_around`,
`space_evenly`. A child overrides `align` with `layout = { align_self =
"end" }`, and asks for leftover room with `layout = { grow = 1 }`.

The older words still work and mean the same: `spacing` is `gap`,
`alignment` is `align`, `layout.alignment` is `align_self`,
`layout.fill_width`/`fill_height` is `grow = 1` (or, across the axis,
`align_self = "stretch"`), and `layout.stretch` is `grow`.

### Positioners

`Row` and `Column` pack children at their own sizes: `gap` between,
`align` across, `justify` along. They never resize a child (`align =
"stretch"` is the one exception). `Grid` with a numeric `columns` fills
row-major; tracks are as wide as their widest cell.

```lua
ui.Row {
  gap = 10, align = "center", justify = "space_between",
  ui.Text { text = "Password:", width = 90 },
  ui.Rect { width = 300, height = 30, radius = 6 },
}
```

Reach for `Flex` the moment a child should take leftover space.

### Flex and track grids

`ui.Flex` is flexbox: `direction = "row" | "column" | "row_reverse" |
"column_reverse"`, `wrap`, `gap`, `padding`, `align` (across the
direction: `start`, `center`, `end`, `stretch`, `baseline`), `justify`
(along it: those plus `space_between`, `space_around`, `space_evenly`),
`align_content` for wrapped lines.

```lua
ui.Flex {
  direction = "row", gap = 8, align = "center", justify = "space_between",
  anchors = { fill = true },
  ui.Text { text = function() return morf.clock:get() end },
  ui.Rect { layout = { grow = 1, minimum_width = 0 }, height = 4, color = "#333" },
  ui.Image { source = "battery.svg", width = 16, height = 16 },
}
```

`ui.Grid` with `template_columns` or `template_rows` is a CSS grid. A
track is `"1fr"`, `"auto"`, a number, `"min_content"`, `"max_content"`,
`{ min = 40, max = "1fr" }`, or `"repeat(3, 1fr)"`. Children place
themselves with `layout = { column = 2, row = 1, column_span = 2 }`;
without placement they flow in order. `column_spacing`/`row_spacing` are
the gaps. Sizes in the child's `layout` may be a number, a percent string
or `"auto"`.

```lua
ui.Grid {
  template_columns = { "repeat(3, 1fr)" }, column_spacing = 20, row_spacing = 20,
  cell(), cell(), cell(), cell(), cell(),
}
```

A layout stretches children across its axis by default, as CSS does; a
positioner does not. A flex item never shrinks below its content unless
it says `layout = { minimum_width = 0 }`, also as CSS does.

Everything under a `Flex` or track `Grid` that is itself one is laid out
in the same pass; anything else is a leaf, sized by the rules above, whose
own children then follow the ordinary rules. A card in a grid cell still
anchors its label.

`ui.RowLayout`, `ui.ColumnLayout` and `ui.GridLayout` are the older names:
a `Flex` in that direction, and a `Grid` whose numeric `columns` becomes
that many `auto` tracks.

### Your own container

`ui.Layout` takes two functions. `measure(available, children)` returns
width and height; `place(bounds, children)` returns one `{ x, y, width,
height }` per child (width and height optional). `available` and `bounds`
are `{ width, height }` (`available` may be `math.huge`), `children` a
list of measured `{ width, height }` in tree order. A child is measured
once, before either is asked. The functions get numbers and return
numbers; writing to a node from inside one is refused.

```lua
ui.Layout {
  anchors = { fill = true },
  measure = function(available, children)
    local h = 0
    for _, c in ipairs(children) do h = math.max(h, c.height) end
    return available.width, h
  end,
  place = function(bounds, children)
    local out = {}
    for i, c in ipairs(children) do
      local x = i == 1 and 0 or i == #children and bounds.width - c.width
        or (bounds.width - c.width) / 2
      out[i] = { x = x, y = (bounds.height - c.height) / 2 }
    end
    return out
  end,
  left, middle, right,
}
```

`examples/lib/align.lua` is exactly that bar.

## 4. Lists

`morf.list_model(rows)` holds rows with stable identity. `model:replace(rows,
"id")` matches by that field, so rows that stayed keep their nodes; without
a key it matches by value. `insert`, `remove`, `move`, `set`, `get`, `len`
as expected.

`ui.Repeater { model, delegate }` builds one node per row and follows the
model: rows that go are destroyed, rows that come are built, rows that
move are moved, and the children's order is the model's. A delegate may
return a second value, an updater `function(row, index)`, in which case a
changed row is patched in place rather than rebuilt: a caption changes, a
thumbnail stays. `as = "row" | "column" | "grid" | "flex"` lays the delegates
out as that container, with its properties.

```lua
local windows = morf.list_model({})
ui.Repeater {
  as = "grid", columns = 3, row_spacing = 20, column_spacing = 20,
  model = windows,
  delegate = function(window)
    local caption = ui.Text { text = window.title }
    return ui.Item { width = 320, height = 200, caption },
      function(next) caption.text = next.title end
  end,
}
-- later
windows:replace(rows, "identifier")
```

`ui.ListView` and `ui.GridView` virtualise long lists; scroll them with
`morf.sync_view(node, offset)`. `ui.each(list, delegate, options)` is a
Repeater over a `morf.state` list (below).

## 5. State

### Bindings

Any property given a function is a binding. It runs once at construction
and again whenever something it read changes: a signal, a `morf.state`
field, another node's property (`other.width`, or `other.width_target`
for the animation's destination), `layout_*`, `morf.clock`. It returns a
scalar. It never runs per frame.

### Signals and state tables

`morf.signal(name, value)` holds one scalar with `get`/`set`.

`morf.state(table)` keeps a shape: each named field is a signal read and
written through the proxy, a nested table is nested, an array is a list
model. Fields are fixed at creation; an unknown name is an error.
`morf.state(table, { reloadable = "name" })` keeps the scalar fields
across a configuration reload under that name, like `morf.reloadable`.

```lua
local model = morf.state { screen = "list", typed = 0, user = { name = "" }, rows = {} }
ui.Text { text = function() return model.user.name end }
model.screen = "prompt"          -- one field
model.rows = { { id = "a" } }    -- a list, replaced whole (by value)
model.rows:replace(rows, "id")   -- or by key
```

### One flush per handler

Every handler the host runs -- a click, a key, a timer, an IPC verb, a
D-Bus call, a capture -- is one flush: write as many signals, fields and
node properties as you like, and the bindings that depend on them run
once, when the handler returns. A bare `node.text = "x"` in a handler
reaches its readers before the next frame. Top-level configuration code
flushes on each write, as it always did.

### Components

`examples/lib/component.lua` gives Elm's shape on these pieces: `init(args)`
returns the model's table, `view(model, send)` runs once and returns a
tree of bindings on the model, and `update(model, msg, send)` is the one
place the model changes. `send(msg)` returns a handler; `send_with(fn)`
builds the message from the handler's arguments; `dispatch(msg)` delivers
one from code that is not a handler. A message that is a function runs
and its return is the message. See `examples/polkit.lua`.

### States

A node's `states` are named tables of `property_changes`,
`anchor_changes` and `parent_change`; `transitions` say how to animate
between them. Select a state by writing `state = "name"`, by a `state`
binding, or let states choose themselves with `when`:

```lua
ui.Rect {
  color = "#222",
  states = {
    default = { property_changes = { color = "#222" } },
    hovered = { when = function() return hover:get() end, property_changes = { color = "#333" } },
    pressed = { when = function() return down:get() end, property_changes = { color = "#444", scale = 0.97 } },
  },
  transitions = { { from = "*", to = "*", duration = 120, easing = "out_quad" } },
}
```

States with `when` are asked by `order` (lowest first, ties by name); the
first true wins; none true selects `default` if there is one, else the
node stays as it is.

### Animation

A `behavior = { property = { duration = 200, easing = "out_quad" } }`
makes writes to that property animate; `ui.spring { stiffness = 300 }`
and `ui.smoothed { velocity = 1000 }` are the other kinds. Behaviors are
installed after construction, so nothing animates its own creation.
Animated *current* values do not re-run bindings; read `_target` for the
destination, or use a `morf.transform_watcher` for the moving value.
`morf.animation.play { ... }` runs groups and keyframes;
`morf.animation.fling` coasts a property.

## 6. What makes a frame

- A property write that lands on a new value marks the surface dirty.
- Anything but transforms, opacity, colours, radii and blur also marks the
  *layout* dirty; the next paint re-lays the whole surface (a `Flex` or
  `Grid` subtree through Taffy, a `Layout` through your functions).
  Animate `translate_x` rather than `x` when only the picture moves.
- Bindings run on invalidation, never per frame. Animations run in Rust
  per frame and run no Lua but `on_finished`.
- A handler gets 100k Lua instructions; effects share a frame budget of
  1M. Exhaustion is logged, not fatal.

## 7. Idioms to prefer

- Reach for a container before a coordinate: a `Row` with `alignment`, a
  `Flex` with `gap`, a `Grid` with tracks, a `Layout` of your own.
- Read `layout_width` instead of recomputing a parent's arithmetic.
- Keep a shape in one `morf.state`, change it in one place, and let
  `when` states and bindings do the reading.
- Keep lists in a model with a key, and let the `Repeater` follow it.
- A secret stays a plain local. Signals are named and observable.
