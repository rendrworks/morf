# Lua shaders

Let a configuration write a shader in Lua, compile it to WGSL, and run it on the
GPU at native speed.

This is an *addition* to the SDF field vocabulary, not a replacement for it. A
node's geometry still comes from the shape families in `morf-region`; a shader
decides what happens inside, on top of, or underneath that geometry.

---

## 1. The decision, and why

There are exactly three ways to get a user's Lua onto a GPU.

| | What runs per pixel | Where | Cost to build | Expressiveness |
|---|---|---|---|---|
| **Interpret** | the user's Lua | CPU | tiny | full |
| **Trace** | generated WGSL | GPU | small | no data-dependent control flow |
| **Compile** | generated WGSL | GPU | medium | full |

**Interpreting** is what [RbxShader](https://github.com/AnotherSubatomo/RbxShader)
does — `src/Worker.client.luau` calls the user's `mainImage` once per pixel in a
nested Luau loop, spread over Roblox actors, with interlacing to skip rows it
cannot afford. Every part of that design is scaffolding around one constraint:
Roblox exposes no shader access, so the CPU is the only place to run one. We
have wgpu. Interpreting would be orders of magnitude slower, would burn the CPU
a compositor needs for layout and input, and would buy nothing.

**Tracing** runs the Lua once with symbolic values whose metamethods record an
expression graph. It is cheap and it is safe by construction, but it has one
wall that is not a matter of taste: in Lua, `__lt` and `__eq` results are
**always coerced to a boolean by the VM**, so `d > 0.5` cannot return a symbolic
node. Worse, a userdata is truthy, so `if d > 0.5 then A else B end` silently
takes `A` — wrong shader, no error. Comparisons would have to be
`d:gt(0.5)` forever, and no loop could carry a data-dependent `break`, which is
the entire algorithm in a raymarcher.

**Compiling** is what this plan builds, for two reasons that only became clear
on inspection:

1. **The front end already exists.** `luna::compiler::parser` is public, and
   `parse_chunk` returns a complete Lua AST with every node wrapped in
   `LineAnnotated<T> { inner, line_number }`. The lexer, the parser, the syntax
   errors and the guarantee of accepting exactly Lua are all already in the
   dependency tree. What is left is the half you would have to write anyway.
2. **The safety argument does not favour tracing once you own codegen.** The
   reason to fear a compiler is that `while true do end` hangs the GPU, wgpu
   loses the device, and the compositor dies — the user's bar, their lock
   screen, their session, from a typo. But *we* decide what WGSL comes out, so
   every emitted loop carries an iteration guard (§6.2). The user writes a
   natural `while`; it cannot hang, because they never got to choose what the
   loop looks like. That is tracing's safety property without tracing's cost.

---

## 2. Architecture

```
Lua source ──► luna parser ──► morf-shader ──► WGSL text ──► wgpu
                  (AST)         (typed IR)       (String)
```

Three crates, one direction:

| crate | job | depends on |
|---|---|---|
| `morf-shader` | Lua AST → typed IR → WGSL | `luna` **and nothing else in this workspace** |
| `morf-lua` | reads the config, calls the compiler, owns the API surface | `morf-shader`, … |
| `morf-render` | compiles the WGSL, binds the params, runs the pass | `morf-shader`, … |

`morf-shader` is a leaf. It does not know what a scene is, what a node is, or
that wgpu exists. That boundary is the one worth defending, and Cargo enforces
it: if the crate ever needs `morf-scene` or `naga`, the abstraction is wrong.

### 2.1 Why WGSL text and not naga IR

wgpu 30 accepts pre-parsed IR — `ShaderSource::Naga(Cow<'static, naga::Module>)`,
behind the `naga-ir` feature — so emitting IR directly is possible. We emit text
anyway:

- Emitting `naga::Module` forces `morf-shader` to depend on `naga`, breaking the
  rule above and coupling a language front end to a graphics stack.
- Text is readable. When a generated shader misbehaves you print it, diff it,
  paste it into a validator. naga IR is arena handles pointing at arena handles,
  and a malformed module gives a validation error with no source location.
- WGSL is a stable spec; naga's IR shifts between wgpu releases.
- There is no per-frame cost to save. Parsing happens once per distinct shader
  at config load, in microseconds, against tens of milliseconds of driver
  pipeline compilation.

**No file is ever written.** `include_str!` already bakes today's `.wgsl` files
into the binary at Rust compile time (`gpu/shaders.rs`); a compiled shader is a
`String` in memory handed straight to `create_shader_module`. The `.wgsl` files
in the tree are source code, not runtime assets.

The typed IR is the real product. WGSL is its serialisation on the last hop, and
swapping it later is one function:

```rust
fn emit_wgsl(program: &Program) -> String        // now
fn emit_naga(program: &Program) -> naga::Module  // later, if it ever pays
```

Same IR, same checker, same tests.

---

## 3. The language

A strict subset of Lua. Everything outside it is a compile error with a line
number, never silently accepted.

### 3.1 Types

```
f32   vec2   vec3   vec4   bool   i32 (loop counters only)
```

No `nil`, no strings, no tables, no closures, no metatables, no varargs, no
recursion, no garbage collection. `mat2`/`mat3` are deferred to §12.

A Lua integer literal in a float context becomes `f32`. `1 / 2` is `0.5`, as in
Lua and unlike C — the emitter forces float division for `/` and maps Lua's `//`
to `floor(a / b)`.

### 3.2 Declarations

Function signatures are **annotated**; bodies are **inferred**.

```lua
function fragment(f: Frag) -> vec4
  local d = length(f.uv - vec2(0.5, 0.5))   -- inferred f32
  return vec4(d, d, d, 1.0)
end
```

Annotation on signatures only is deliberate: it makes inference local and
tractable, and it makes error messages point at a declared intent rather than a
guess. Inferring parameter types from a single call site is possible and is not
worth the diagnostic quality it costs.

Lua has no type-annotation syntax, and Luna's parser will reject `f: Frag`. Two
ways out, decided at implementation time in Milestone 1:

- **(a) Comment annotations** — `---@param f Frag` / `---@return vec4`, the
  LuaLS convention. Parses as Lua, reads naturally, tooling already understands
  it. Requires reading comments, which Luna's lexer discards.
- **(b) A wrapper table** — the signature lives in the Lua table around the
  function, not in its syntax:

  ```lua
  morf.shader("plasma", {
    inputs = { uv = "vec2", time = "f32" },
    params = { intensity = 1.0, tint = "#3b82f6" },
    returns = "vec4",
    fragment = function(uv, time, intensity, tint) … end,
  })
  ```

**(b) is the recommendation.** It needs no lexer change, it is ordinary Lua, the
parameter list is checked against `inputs`+`params` by arity and name, and
`params` doubles as the declaration of animatable properties (§8). Adopt (a)
later as sugar if it is missed.

### 3.3 Statements supported

| Lua | WGSL | note |
|---|---|---|
| `local x = e` | `var x = e;` | `let` when never reassigned |
| `x = e` | `x = e;` | type must match declaration |
| `if/elseif/else` | `if/else if/else` | condition must be `bool` |
| `while c do` | `loop { if !c { break } … }` | **guarded**, §6.2 |
| `for i = a, b, s do` | `for (var i…)` | **guarded**; `a`,`b`,`s` need not be constant |
| `repeat … until c` | `loop { … if c { break } }` | **guarded** |
| `break` | `break` | |
| `return e` | `return e;` | one value only |
| `do … end` | `{ … }` | scope |

Rejected with a diagnostic: `goto`, labels, `for … in` (generic for), function
definitions inside a shader body, table constructors, string literals except as
`params` colour defaults, method calls on anything but the input struct.

### 3.4 Operators

`Expression<S>` is `{ head, tail: Vec<(BinaryOperator, Expression<S>)> }` — but
**precedence is already resolved by the parser**, which nests higher-precedence
subexpressions into each tail entry's right-hand `Expression`. A plain left-fold
is therefore correct, and this is exactly what Luna's own compiler does
(`compiler.rs:1204`):

```rust
let mut expr = self.head_expression(&expression.head)?;
for (binop, right) in &expression.tail {
    expr = self.binary(expr, *binop, self.expression(right)?)?;
}
```

Do the same. Do **not** write a precedence climber; it would be wrong.

Supported: `+ - * / % ^ // == ~= < <= > >= and or not` and unary `-`.
Rejected: `..` (concat), `& | ~ << >>` (bitwise), `#` (length).

`and`/`or` on `bool` compile to `&&`/`||`. Lua's value-returning `and`/`or`
idiom (`a or b` where `a` is a number) is rejected — there is no `nil` to make
it meaningful.

### 3.5 Builtins

One vocabulary, ordinary function calls, no method syntax:

```
abs ceil clamp cos degrees distance dot exp exp2 floor fract length log log2
max min mix mod normalize pow radians reflect round sign sin smoothstep sqrt
step tan
vec2 vec3 vec4               constructors, with scalar broadcast
select(a, b, cond)           both arms evaluated, like WGSL
texture(sampler, uv)         effect mode only, §7.3
```

`math.sin` is **also** accepted and lowered identically, because a user will
write it. `math.pi` and `math.huge` are constants. Anything else under `math.`
is a diagnostic naming the supported set.

Swizzles are field access: `v.x`, `v.xy`, `v.rgb`, `v.wzyx`. Mixing `xyzw` with
`rgba` in one swizzle is an error, as in WGSL.

---

## 4. `morf-shader` internals

```
crates/morf-shader/
  Cargo.toml            deps: luna
  src/
    lib.rs              public API, ~60 lines
    types.rs            Type, Value, ShaderKind, Signature
    ir.rs               Program, Function, Block, Stmt, Expr, BinOp, Builtin
    lower.rs            AST → IR, the type checker              (largest file)
    lower_expr.rs       expression lowering (split for the 500-line gate)
    builtins.rs         signature table for every builtin
    validate.rs         loop guards, node caps, uniformity
    emit.rs             IR → WGSL
    diagnostics.rs      Diagnostic { line, message, note }
    tests/              golden tests, §11
```

### 4.1 Public API

```rust
pub fn compile(source: &str, spec: &ShaderSpec) -> Result<Compiled, Vec<Diagnostic>>;

pub struct ShaderSpec {
    pub kind: ShaderKind,                 // Material | Surface | Effect
    pub inputs: Vec<(String, Type)>,      // uv, time, resolution, …
    pub params: Vec<(String, Type)>,      // user-declared, animatable
    pub entry: String,                    // "fragment"
}

pub struct Compiled {
    pub wgsl: String,
    pub params: Vec<ParamSlot>,           // name → byte offset in the uniform block
    pub uniform_size: u32,                // std140-compatible, padded
    pub reads_time: bool,                 // drives per-frame repaint, §9
    pub samples_behind: bool,             // Effect only
    pub hash: u64,                        // pipeline cache key, §8.3
}

pub struct Diagnostic {
    pub line: u32,
    pub message: String,                  // "cannot add vec3 and vec2"
    pub note: Option<String>,             // "shaders have no tables"
}
```

`compile` is a pure function. No Lua VM, no wgpu, no filesystem, no globals —
which is what makes the whole crate testable with `assert_eq!` on a string.

### 4.2 Parsing

```rust
use luna::compiler::{interning::BasicInterner, parser::parse_chunk};

let chunk = parse_chunk(source.as_bytes(), BasicInterner::default())
    .map_err(|error| vec![Diagnostic::from_parse(error)])?;
```

`BasicInterner` is `Default` and public, and `S::String` is `Rc<[u8]>`. Names
are compared as bytes; store them as `Rc<[u8]>` through lowering and convert to
`String` only for diagnostics and emitted identifiers.

### 4.3 IR

Statement-structured, not SSA — WGSL is a structured language, so keeping
statements means the emitter is a direct print rather than a
control-flow-reconstruction problem.

```rust
pub struct Program {
    pub entry: Function,
    pub uniforms: Vec<(String, Type)>,
}

pub struct Function {
    pub params: Vec<(Name, Type)>,
    pub returns: Type,
    pub body: Block,
}

pub struct Block(pub Vec<Stmt>);

pub enum Stmt {
    Let   { name: Name, ty: Type, value: Expr, mutable: bool },
    Assign{ target: Name, value: Expr },
    If    { arms: Vec<(Expr, Block)>, otherwise: Option<Block> },
    Loop  { guard: u32, body: Block },        // every loop shape lowers to this
    Break,
    Return(Expr),
}

pub enum Expr {
    Literal(Value),
    Local(Name),
    Uniform(usize),
    Input(InputSlot),
    Unary  { op: UnOp, ty: Type, value: Box<Expr> },
    Binary { op: BinOp, ty: Type, left: Box<Expr>, right: Box<Expr> },
    Call   { builtin: Builtin, ty: Type, args: Vec<Expr> },
    Construct { ty: Type, args: Vec<Expr> },
    Swizzle{ ty: Type, value: Box<Expr>, components: [u8; 4], len: u8 },
}
```

Every `Expr` carries its resolved `Type`. Lowering computes it once; the emitter
never infers, and `validate` never guesses.

`while`, numeric `for` and `repeat` all lower to `Stmt::Loop`, so §6.2's guard
has exactly one place to live. A numeric `for i = a, b, s` becomes:

```
Let i = a (mutable)
Loop { guard, body: [ If !(i <= b) { Break }, …user body…, Assign i = i + s ] }
```

with the comparison flipped when `s` is a negative constant. A non-constant
step is a diagnostic — Lua evaluates the step once and its sign decides the
comparison, and reproducing that faithfully in WGSL needs a runtime branch that
is not worth the surface.

### 4.4 Lowering and type checking

One pass, `lower.rs`, holding:

```rust
struct Lowerer<'a> {
    scopes: Vec<HashMap<Name, Local>>,   // lexical, pushed per Block
    uniforms: &'a [(String, Type)],
    inputs: &'a [(String, Type)],
    diagnostics: Vec<Diagnostic>,
    loop_depth: u32,                     // `break` outside a loop is an error
    nodes: u32,                          // §6.3 cap
}
```

Rules:

- A `local` with an initialiser takes the initialiser's type. Without one, it is
  a diagnostic — there is no `nil` to hold the place.
- Reassignment must match the declared type exactly. No implicit vec widening on
  assignment; `v = 1.0` where `v : vec3` is an error, and `v = vec3(1.0)` is
  how you say it.
- Arithmetic follows WGSL: same-type componentwise, or vector-scalar broadcast
  for `* /` and (as a deliberate convenience) `+ -`. `vec3 + vec2` is an error
  naming both types.
- Comparison operands must be `f32` or `i32` and produce `bool`.
- `if` and `while` conditions must be `bool`. **A non-bool condition is a hard
  error**, not a truthiness coercion — this is the single most important
  diagnostic in the compiler, because Lua users expect `if x then` to work and
  the shader semantics cannot provide it.
- Builtin calls resolve against `builtins.rs`, which lists every overload as
  `(&[Type], Type)`. On no match, the diagnostic prints the call's actual
  argument types and the available overloads.
- Local shadowing is allowed (Lua semantics); locals are renamed to
  `name_<scope>` on emit so WGSL sees unique identifiers.

Diagnostics **accumulate**. Lowering does not stop at the first error; it
substitutes a poison type that suppresses cascades and keeps going, so a user
sees every mistake in one run.

### 4.5 Emission

`emit.rs` walks the IR and prints. It never makes a decision — every type is
already resolved — so it is mechanical and short.

The generated module for a Material shader:

```wgsl
struct MorfShaderUniforms {
    resolution: vec2<f32>,
    time: f32,
    _pad0: f32,
    intensity: f32,
    tint: vec4<f32>,
    // …params, padded to 16-byte alignment
};
@group(1) @binding(0) var<uniform> u: MorfShaderUniforms;

fn morf_shader_main(uv: vec2<f32>, local: vec2<f32>, coverage: f32) -> vec4<f32> {
    …lowered body…
}
```

The entry point is a **function, not `@fragment`**. It is concatenated into the
field shader by `shader_source()` and called from the existing fragment stage
(§7.1). That is what keeps clipping, damage, transforms, the input region and
the whole geometry path working untouched.

Uniform layout follows std140-ish rules: `f32` at 4, `vec2` at 8, `vec3`/`vec4`
at 16, struct padded to 16. `ParamSlot` records the computed offset so the host
writes params without recomputing the layout — one source of truth, and a test
asserts the Rust-side offsets match a hand-written expectation.

---

## 5. What a config writes

```lua
morf.shader("plasma", {
  kind = "material",
  params = { intensity = 1.0, tint = "#3b82f6" },
  fragment = [[
    function fragment(uv, time, intensity, tint)
      local d = length(uv - vec2(0.5, 0.5))
      local wave = sin(d * 10.0 - time) * 0.5 + 0.5
      return vec4(tint.rgb * wave * intensity, tint.a)
    end
  ]],
})

morf.rect{
  width = 200, height = 80,
  shader = "plasma",
  shader_params = { intensity = 2.0 },
}
```

The shader body is a **string**, not a Lua function. That is deliberate and
worth stating plainly: it is not Lua that will ever run, it is a shader that
happens to use Lua's syntax, and pretending otherwise invites `print` debugging
that cannot work. A string makes the boundary honest, and it is what lets the
compiler own the whole text including line numbers.

Animation falls out of `params` being ordinary node properties:

```lua
morf.animate(node, "shader_params.intensity", { to = 3.0, duration = 400 })
```

This is the real advantage over Shadertoy: `iTime` and a mouse position versus
every binding, spring and signal morf already has.

---

## 6. Safety

### 6.1 Why it matters more here than in a game

A hung shader loses the wgpu device. Losing the device kills the compositor —
the bar, the lock screen, the session. There is no "restart the app". Every rule
below exists because the blast radius is the user's whole desktop.

### 6.2 Loop guards

Every `Stmt::Loop` emits a counter:

```wgsl
var _g3: u32 = 0u;
loop {
    if (_g3 >= 4096u) { break; }
    _g3 += 1u;
    …
}
```

The bound is per-loop, defaulting to `MAX_ITERATIONS = 4096`, lowered to the
static trip count when a numeric `for` has constant bounds (the common case, and
it costs nothing at runtime because the driver folds it). Nested loops multiply,
so a separate `MAX_TOTAL_ITERATIONS` rejects a nest whose product exceeds
1 << 22 at compile time.

This cannot be defeated from Lua, because the user never writes the loop —
they write a `while`, and we decide what it becomes.

### 6.3 Caps

| cap | value | why |
|---|---|---|
| source length | 64 KiB | a config file, not a program |
| IR nodes | 100 000 | shader compile time is superlinear |
| loop nesting | 4 | past this the trip-count product is meaningless |
| params per shader | 32 | uniform block size |
| distinct shaders | 64 | pipeline count, §8.3 |

Each is a diagnostic naming the cap and the measured value, never a panic.

### 6.4 Uniformity

WGSL forbids `textureSample` under non-uniform control flow. In Effect mode
(§7.3), a `texture()` call inside an `if` or a loop is rejected by `validate.rs`
with our own diagnostic and a line number, because naga's message for this is
close to unreadable. `validate` walks the IR tracking whether it is inside
non-uniform control flow; the check is a dozen lines and saves a class of
inscrutable failure.

---

## 7. The three modes

### 7.1 Material — colour inside the existing shape

The field decides coverage; the shader decides colour. Everything about
geometry, clipping, damage and hit-testing is unchanged.

Seam: `field.wgsl`'s fragment stage, immediately after `gradient_fill` resolves
`fill_color`. The shader function is appended to the module and called:

```wgsl
var fill_color = gradient_fill(material, input.local, surface.fill);
if (material.shader.x > 0.5) {
    fill_color = morf_shader_main(shader_uv, input.local, coverage);
}
```

Cheapest mode, and the one to build first. A distinct pipeline per shader,
because the shader body is concatenated into the source.

### 7.2 Surface — the shader owns coverage

The shader returns alpha too, over the node's rectangle, Shadertoy-style. The
field composition is skipped for that node; `coverage` is whatever the shader
returns in `.a`.

Same pipeline, a different generated wrapper: the SDF layer loop is not emitted
at all and `distance` is not computed. Geometry and shader stop composing here —
that is inherent, not a limitation to fix, and the docs must say so.

`area` (the quad the fragment stage walks) is the node rect, since there is no
layer reach to compute.

### 7.3 Effect — read what is underneath

The shader samples already-rendered content: distortion, chromatic aberration, a
custom blur.

The machinery exists. `backend_render.rs:82-200` already renders a `Layer`'s
subtree into its own texture, optionally runs it through `create_blur_chain`,
and composites it back with a mask. **An effect shader is another kind of layer
composite** — instead of the plain texture pipeline, run the user's shader over
the layer's target view.

Changes:
- `Layer` (`commands.rs:317`) gains `shader: Option<ShaderId>` and
  `shader_params: Range<usize>`.
- `draw_layer!` selects a shader pipeline instead of `self.glyph_pipeline` when
  the layer carries one; the layer's target view binds where the glyph atlas
  would.
- The generated module gets `@group(0) @binding(0) var behind: texture_2d<f32>;`
  plus a sampler, and `texture(uv)` lowers to `textureSample(behind, samp, uv)`.
- `samples_behind` in `Compiled` tells the host this shader needs a layer, so a
  node with an effect shader implies a layer even without opacity or blur.

Build this **last**. It is the only mode that touches the layer graph.

---

## 8. Host integration

### 8.1 Scene schema

`morf-scene/src/schema.rs`, on `Element::Rect | ClipRect | Sdf`:

```rust
string("shader", ""),                    // registered shader name, empty = none
```

Params cannot be schema properties, because their names are per-shader. They
live in a side table on the node, keyed by name, holding `f32`/`vec4` — the same
place `shader_params.intensity` resolves to for animation.

### 8.2 Compilation point

At config load, `morf.shader(name, spec)` compiles immediately and raises a Lua
error carrying the diagnostics on failure. **Not lazily at first paint** — a
shader error must surface while the config author is looking at their terminal,
not on the frame a node first becomes visible.

Diagnostics render as:

```
plasma:3: cannot add vec3 and vec2
  note: use vec3(v.xy, 0.0) to widen
```

### 8.3 Pipeline cache

`Compiled::hash` is the key: FNV over the emitted WGSL, not the Lua source, so
two shaders that differ only in comments share a pipeline. `WgpuBackend` holds
`HashMap<u64, ShaderPipeline>`, capped at 64 with an error past that rather than
eviction — evicting a pipeline that a visible node needs would recompile it
mid-frame, and a compositor cannot afford tens of milliseconds at paint time.

Compilation happens on registration, never during `render`.

### 8.4 Damage

`Compiled::reads_time` decides repaint. A shader that never reads `time`
repaints only when its node or params change — a gradient-ish shader on a static
bar costs nothing after the first frame. A shader that does read `time` marks
its node perpetually damaged.

This must be **derived, not declared**: a `reads_time` a user has to remember to
set is a `reads_time` that will be wrong. Lowering records it when it resolves
the `time` input, which cannot be forgotten.

---

## 9. Errors the user will actually hit

Ranked by how often, with the diagnostic each must produce:

1. `if x then` where `x` is `f32` — *"a shader condition must be a bool; write
   `if x > 0.0 then`"*. Never coerce.
2. `vec3 * vec2` — *"cannot multiply vec3 and vec2"*, with both types named.
3. `local t = {}` — *"shaders have no tables"*.
4. `print(d)` — *"shaders cannot print; there is no host to print to"*.
5. `math.random()` — *"not available; use a hash of uv for noise"* with a
   worked one-liner in the note.
6. A `while` with no exit — compiles, runs, hits the guard, produces a wrong
   image rather than a dead session. Acceptable, and the alternative is a
   halting-problem analysis.

Every diagnostic carries the line number Luna already recorded, offset by where
the shader string starts in the config file so it points at the real file.

---

## 10. Testing

`morf-shader` is a pure function, so most of it tests without a GPU or a VM.

**Golden tests** (`crates/morf-shader/src/tests/golden.rs`): a table of
`(lua, expected_wgsl_fragment)`. Assert on a substring, not the whole module, so
adding a uniform does not break forty tests.

**Diagnostic tests**: every entry in §9, asserting line number *and* message.
This is the suite that keeps error quality from rotting.

**Cap tests**: a 5-deep loop nest, a 200-param shader, a 70 KiB source — each
must produce its named diagnostic and not a panic.

**GPU conformance** (`morf-render`, `#[ignore]` like the existing 25): compile
a shader, render 64×64, assert pixels. The first three:

- a shader returning a constant fills exactly the SDF coverage and nothing
  outside it — proves the Material seam;
- `vec4(uv.x, 0, 0, 1)` produces a horizontal red ramp — proves uv orientation,
  which is the thing that is silently upside-down otherwise;
- a `while` with no exit condition terminates and the frame completes — proves
  the guard, and is the single most important test in the plan.

**Ports**: `Plasma.luau`, `Cosmic.luau`, `ShaderArt.luau` from the RbxShader
repo, translated by hand. They are the honest measure of whether the subset is
expressive enough. `NaiveRaycast.luau` and `MicroRayMarcher.luau` are the
stretch targets — they need real loops with `break`, which is exactly what the
compiler buys over a tracer, so they are the proof the decision was right.

---

## 11. Milestones

Each ends green on `oslo make verify`, and each is a commit.

**M1 — the skeleton compiles nothing.** `morf-shader` crate, `ShaderSpec`,
`Diagnostic`, `parse_chunk` wired up, `compile` returning
"not implemented" for every input. Decides §3.2's annotation question. *Proves
the Luna parser dependency works in isolation.*

**M2 — expressions.** Literals, locals, arithmetic, comparisons, builtins,
constructors, swizzles. `return` only, no control flow. Golden tests for each
operator and the type errors around them. *This is the largest single chunk and
where the type checker is built.*

**M3 — control flow.** `if`/`elseif`/`else`, `while`, numeric `for`, `repeat`,
`break`, with the loop guard from the first commit. Cap enforcement.

**M4 — Material mode end to end.** `morf.shader` in Lua, the `shader` schema
property, uniform packing, the pipeline cache, the `field.wgsl` seam, the first
three GPU tests. *First frame a user-written shader paints.*

**M5 — params and animation.** `shader_params`, the side table, offset
computation, `morf.animate` reaching a param, `reads_time` damage.

**M6 — Surface mode.** The alternate wrapper, node-rect `area`.

**M7 — Effect mode.** `Layer.shader`, the composite seam, texture sampling, the
uniformity check.

**M8 — the ports.** Plasma, Cosmic, ShaderArt; then the raymarchers. Whatever
they need that the subset lacks becomes the backlog, and the honest answer to
"is this actually usable".

M1–M4 is the useful spine. If the project stops after M4 it is still a feature.

---

## 12. Deferred

- `mat2`/`mat3` and matrix multiply. Wanted for rotation; `vec2` rotation by
  hand covers most of it until then.
- Multiple functions per shader. One entry point only at first; user-defined
  helper functions are an obvious M9 and need no new theory, just inlining or
  real WGSL function emission.
- Compute shaders. Different pipeline, different plan.
- `---@param` comment annotations as sugar over §3.2(b).
- Emitting `naga::Module`. Only if profiling ever shows WGSL parsing matters,
  which it will not.

---

## 13. Risks

**The subset is too small to be fun.** The real risk, and M8 is the test. A
shader language you cannot port a Shadertoy into is a checkbox. Mitigation: do
M8 early enough to still change the design.

**Type inference is worse than it looks.** Vector-scalar broadcast rules have
more corners than expected, and every wrong one is a confusing error. Mitigation:
`builtins.rs` as an explicit overload table rather than clever unification —
verbose, boring, checkable.

**Pipeline count.** Sixty-four distinct shaders in one config would be unusual,
but a config that generates shaders in a loop would hit it instantly.
Mitigation: hash on emitted WGSL so identical shaders collapse, and a clear
diagnostic at the cap.

**Effect mode and damage.** A shader that samples underneath is only correct if
what is underneath was rendered this frame — partial damage could composite
stale content. Mitigation: a node with an effect shader forces its layer to full
repaint. Cheap, and correct.
