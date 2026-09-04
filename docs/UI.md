# Writing UI in morf

How a configuration puts things on screen: the node kinds, how they are
sized and placed, how state reaches them, and what makes a frame. Every
section has a snippet you can paste into a config and run.

Four questions have four separate answers, and every feature here
belongs to exactly one of them:

- **Where does a node go?** Layout. Decided by the node's *parent*, and by
  only one kind of parent at a time.
- **Where does a property's value come from?** State. Decided per property:
  a literal, a binding, or a state block.
- **How is a piece of UI organised?** Structure: a function returning a
  node, a component with a model, a list with a delegate.
- **What does a node look like?** Appearance. Numbers and colours on the
  node itself, every one of which animates by existing.

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
| layouts | `Flex`, `Grid` with tracks |
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
positive; `layout.width`/`layout.height`; its
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
without placement they flow in order. `gap`, or `column_gap` and
`row_gap`, are the gaps. Sizes in the child's `layout` may be a number, a percent string
or `"auto"`.

```lua
ui.Grid {
  template_columns = { "repeat(3, 1fr)" }, gap = 20,
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
  as = "grid", columns = 3, gap = 20,
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
value: a number, a string, a colour, or a table for the properties that
take one (a gradient, a decoration). It never runs per frame.

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
installed after construction, so nothing animates its own creation;
`enter` (section 6) says where a first frame starts instead.
Animated *current* values do not re-run bindings; read `_target` for the
destination, or use a `morf.transform_watcher` for the moving value.
`morf.animation.play { ... }` runs groups and keyframes;
`morf.animation.fling` coasts a property.

## 6. Appearance

One rule covers every property in this section: it is a number, a colour,
or a table of those, and a `behavior` on it animates it. Nothing here has
a second mechanism.

### Colour values

`morf.color(x)` reads any notation: hex, `rgb()`, `hsl()`, `hwb()`,
`lab()`, `oklch()`, a named colour, `transparent`, a `/ alpha` at the end
of any of them. Every property that takes a colour takes a string or a
value, and reads back as a value. A value has fields `r g b a h s l`, and
methods for nearly everything a colour can do:

```lua
local accent = morf.color "#3366cc"
accent:lighten(0.1)  accent:darken(0.1)  accent:saturate(0.2)  accent:rotate(30)
accent:alpha(0.5)    accent:complement() accent:gray()         accent:invert()
accent:mix(other, 0.5, "oklch")          accent:composite(over)
accent:luminance()   accent:is_light()   accent:contrast(paper)  accent:text_color()
accent:distance(other, "ciede2000")      accent:blind("deuteranopia")
accent:with { l = accent.l - 0.1, space = "oklch" }
accent:hex()  accent:oklch_string()  accent:rgb8()  accent:nearest_name()
```

Constructors live beside it: `morf.color.rgb`, `hsl`, `hsv`, `lab`,
`oklab`, `lch`, `oklch`, `xyz`, `cmyk`, `gray`, `named`, `random("vivid")`,
`mix(a, b, t, space)`, `scale { stops }:sample(t)`, and
`distinct(n, { fixed, metric, iterations })` for a palette whose members
stay apart. `c:ansi_style { bold = true }` and `c:paint(text)` colour a
terminal.

A colour animates in a space. The default is OkLab, which is what a
crossfade between two saturated colours should look like; `space` and
`hue` on the behavior choose otherwise:

```lua
ui.Rect { color = accent, behavior = { color = { duration = 300, space = "oklch", hue = "longer" } } }
```

### Gradients

`gradient` on a `Rect` or an `Sdf` is one table: a `kind` (`linear`,
`radial`, `conic`), an `angle` (0 points up, 90 right, as a stylesheet's
does), a centre `at = { x, y }` in fractions, a `radius` for radial, the
`space` neighbouring stops mix in, and up to sixteen `stops`. A stop is a
colour, a `{ color, position }` pair, or a `{ color = , position = }`
table; a bare list spreads evenly, a missing position sits between its
neighbours, and two stops at one position are a hard edge.

```lua
ui.Rect {
  gradient = { angle = 135, space = "oklch", stops = { "#e6f7fa", { accent, 0.5 }, "#5fa8d3" } },
  behavior = { gradient = { duration = 400 } },  -- every stop moves
}
```

The whole table is one property: a binding may return it, and a behavior
on it moves every stop's colour and position at once.

### Themes and preferences

`morf.theme(tokens, options)` is a `morf.state` for appearance. A string
that names a colour becomes one. A function field is derived: read
inside a binding, whatever it touches is what the binding follows, so
`hover` below re-derives when `accent` changes with no wiring. A `source`
is a JSON file whose leaf keys are tokens -- the file a palette generator
writes -- read now and again whenever it is rewritten.

```lua
local theme = morf.theme({
  accent = "#3366cc", paper = "#f6f5f4",
  hover = function(t) return t.accent:alpha(0.12) end,
  ink = function(t) return t.paper:text_color() end,
}, { source = "~/.cache/wal/colors.json" })
ui.Rect { color = function() return theme.hover end }
theme.accent = "#ff6600"   -- and every reader of hover follows
```

`morf.prefers` is the desktop's own settings, read from the settings
portal and kept current: `color_scheme` (`"dark"`, `"light"`, `"none"`),
`contrast`, `accent_color` (a colour or nil), `reduced_motion`, and the
driven output's `scale`. Each is a field a binding follows. A derived
token that reads one switches palette with the desktop. When
`reduced_motion` is on, every behavior, group and spring lands on its
target on the next tick: a configuration written with motion ends up in
the same place, without the travel.

`Text` takes `color = "inherit"`: the nearest ancestor with a colour.
An `Item` carries one for the purpose without painting anything.

### Text

Beyond `font_family`, `font_size` and `font_weight`: `line_height` is a
multiple of the size, or a `"24px"` size; `letter_spacing` and
`word_spacing` are pixels; `font_style` is `normal`, `italic` or
`oblique`; `font_stretch` runs from `ultra_condensed` to
`ultra_expanded`. A `decoration = { line, thickness, offset, color }`
draws a band `under`, `over` or `through` the text from the face's own
metrics; thickness and colour default to the face's and the text's.

```lua
ui.Text {
  text = "Sorry, that didn't work", line_height = 1.4, font_style = "italic",
  decoration = function() return refused:get() and { line = "under", color = theme.alert } or {} end,
}
```

### Entering

`enter = { opacity = 0, translate_x = 32 }` on any node is where its
first frame starts. The behaviors carry it from there to the declared
values; a property with no behavior simply arrives. This is the one way
creation animates.

```lua
ui.Rect {
  opacity = 1, translate_x = 0,
  enter = { opacity = 0, translate_x = 32 },
  behavior = { opacity = { duration = 220 }, translate_x = { kind = "spring", stiffness = 260 } },
}
```

### Cursors

`cursor = "pointer"` on a `MouseArea` is the pointer's shape while it is
over the area, drawn by the compositor from its own theme. The names are
the cursor-shape protocol's: `default`, `pointer`, `text`, `grab`,
`grabbing`, `move`, `not_allowed`, `crosshair`, `ew_resize`, and the
rest, spelled with underscores.

## 7. What makes a frame

- A property write that lands on a new value marks the surface dirty.
- Anything but transforms, opacity, colours, radii and blur also marks the
  *layout* dirty; the next paint re-lays the whole surface (a `Flex` or
  `Grid` subtree through Taffy, a `Layout` through your functions).
  Animate `translate_x` rather than `x` when only the picture moves.
- Bindings run on invalidation, never per frame. Animations run in Rust
  per frame and run no Lua but `on_finished`.
- A handler gets 100k Lua instructions; effects share a frame budget of
  1M. Exhaustion is logged, not fatal.

## 8. Idioms to prefer

- Reach for a container before a coordinate: a `Row` with `align`, a
  `Flex` with `gap`, a `Grid` with tracks, a `Layout` of your own.
- Read `layout_width` instead of recomputing a parent's arithmetic.
- Keep a shape in one `morf.state`, change it in one place, and let
  `when` states and bindings do the reading.
- Keep lists in a model with a key, and let the `Repeater` follow it.
- Keep a palette in a `morf.theme` and derive from it; let
  `morf.prefers` choose the scheme.
- A secret stays a plain local. Signals are named and observable.
