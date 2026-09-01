# WGSL coverage

What a configuration's Lua shader can express, against what WGSL can, and the
ordered plan for closing the gap.

The compiler itself is built — see `PLAN_LUA_SHADER.md`. This is the tracking
document for the *language surface*: which types, operators, statements and
builtins are reachable from Lua today, which are not, and in what order the
missing ones are worth adding.

---

## 1. What "100% coverage" should mean

Not all of WGSL. WGSL is a language for vertex, fragment and **compute**
shaders, and a compute dialect cannot mean anything here: a per-node fragment
shader has no workgroup to synchronise, no storage buffer to write, and no
dispatch to be part of. Aiming at literal totality would mean building
`workgroupBarrier` for nobody.

The coherent target is **everything a fragment shader can express**, and against
that the gap is real and finite. Section 6 is what is out of scope and why, so
that "not done" and "not applicable" never get confused for one another.

Counts below are measured against `naga 30.0.1`, which is what actually accepts
our output — `MathFunction` has **79** variants, plus derivatives and the four
relational functions.

---

## 2. Where it stands

| area | have | missing | note |
|---|---|---|---|
| types | `f32` `i32` `u32` `bool`, float and integer vectors, `mat2/3/4`, `array<T, N>` | `f16`, user structs | §4.5, §6 |
| operators | everything Lua has except `..` | — | done |
| statements | `local` assign `if` `while` `for` `repeat` `break` `continue` `discard` `return` | `switch`, deliberately | §4.3 |
| functions | entry point, helpers (monomorphised) | — | done |
| math builtins | **79 of 79** | — | §3 |
| derivatives | `dpdx` `dpdy` `fwidth` | the coarse/fine variants, deliberately | §4.4 |
| relational | `all` `any` `isNan` `isInf` | bool vectors to fold | §4.4 |
| textures | the layer beneath, plus any a configuration declares | `textureLoad`, explicit LOD | §5.1 |
| stages | fragment, and vertex displacement | — | §5.2 |
| storage | read-only data blocks the host fills | writable, deliberately | §5.3 |

Every one of naga's 79 `MathFunction` variants is now reachable by name, plus
`select`, `texture`, the conversions and the bitcasts. `outer` is the only one
restricted — to the square case, which is the only shape this language has a
matrix type for.

---

## 3. The builtins

Exact, from naga's own enum. All 79 are implemented; the grouping records what
each was blocked on and why it landed when it did.

### 3.1 Worth adding — ordinary shader arithmetic (13) — **W1 done**

- [x] `saturate` — `clamp(x, 0, 1)`, written constantly
- [x] `trunc`
- [x] `inverseSqrt` — the fast reciprocal length every normalisation wants.
      Spelled `inversesqrt` and `inverse_sqrt`: WGSL writes it one way and
      GLSL, which is what a shader author has read more of, writes it the other
- [x] `fma`
- [x] `faceForward`
- [x] `refract` — we had `reflect` and not this, which was an odd pair to split
- [x] `asinh` `acosh` `atanh` — completing the set already half-present
- [x] `quantizeToF16`
- [x] `modf` `frexp` — §4.5
- [x] `ldexp`

**What W1 caught.** `refract` is the only one of these with a shape of its own —
two vectors and a scalar ratio — and the emitter widened its scalar to the
call's vector type, producing WGSL naga rejects. That is the worst failure this
compiler can produce, because the author gets a validation error with no line
number; the fix was to list the arguments that are *meant* to be a different
type beside `select`'s condition. The GPU test in §8.3 is what found it, which
is the argument for that test existing at all.

### 3.2 Worth adding — matrix (4) — **W2 done**

- [x] `transpose`
- [x] `determinant`
- [x] `inverse`
- [x] `outer` — restricted to the square case, which is the only one this
      language has a type for. **WGSL has no `outer`**: naga carries one for
      its GLSL frontend and the WGSL grammar does not name it, so calling it
      emits code no driver accepts. It is a matrix of scaled copies of one
      vector, and it is emitted as exactly that.

### 3.3 Worth adding — integer and bitwise (8) — **W3 done**

- [x] `countTrailingZeros` `countLeadingZeros` `countOneBits` `reverseBits`
- [x] `extractBits` `insertBits` — the latter is the one builtin in this
      language taking four arguments, which the first attempt got wrong
- [x] `firstTrailingBit` `firstLeadingBit`

### 3.4 Packing (18) — **done**

These were filed as low value, on the grounds that a shader here has no buffer
of its own to pack for. That was a reason to do them last, not a reason to skip
them: they are eighteen table entries, and six of them are what forced integer
vector types to exist, which is a capability rather than a footnote.

- [x] `pack4x8snorm` `pack4x8unorm` `pack2x16snorm` `pack2x16unorm`
      `pack2x16float` `pack4xI8` `pack4xU8` `pack4xI8Clamp` `pack4xU8Clamp`
- [x] `unpack4x8snorm` `unpack4x8unorm` `unpack2x16snorm` `unpack2x16unorm`
      `unpack2x16float` `unpack4xI8` `unpack4xU8`
- [x] `dot4I8Packed` `dot4U8Packed`

---

## 4. Language features

### 4.1 Matrices — `mat2x2` … `mat4x4` — **W2 done**

**The single highest-value gap.** Rotation is the first thing anyone reaches
for in a shader, and before this it could only be written by expanding the
arithmetic by hand.

- [x] `Type::Mat2`, `Mat3`, `Mat4` (square only; `matCxR` still unbuilt)
- [x] Constructors from columns *and* from every component at once — WGSL
      accepts both, and both get written: columns are how a rotation is
      composed, the flat form is how one gets pasted out of somebody else's
      shader
- [x] `matrix * vector`, `vector * matrix` (the row form, which WGSL defines
      and which is not a mistake), `matrix * matrix`, `matrix * scalar`
- [x] `transpose` `determinant` `inverse`
- [x] Uniform layout: a `mat3x3` is three `vec4`-aligned columns — forty-eight
      bytes, not thirty-six — asserted by a test on both sides
- [x] Column indexing `m[0]` — done in W5 with the rest of §4.6

**Two rules that had to be stated rather than inherited.** A matrix is
deliberately *not* "numeric": `m * v` is a linear map applied to a vector, not a
componentwise multiply, so routing it through the same path would make `m + v`
silently mean something. And a scalar never widens into a matrix — the emitter
widens scalars against vector calls, which is right almost everywhere and would
produce `mat2x2<f32>(0.5)` here, which is not even legal WGSL.

Non-square types can wait; nobody writes `mat2x3` by hand.

### 4.2 Integers and bitwise — **W3 done**

- [x] `Type::U32`, and `i32` promoted from loop-counter-only to a real type
- [x] **Abstract integer literals.** The interesting part, and it went the way
      WGSL itself does it: an integer literal has no type until something asks.
      `1 / 2` is `0.5` because nothing in it asks for a whole number, and
      `1 << 2` is four because a shift does. `Type::defaulted` commits an
      undecided literal to `f32` at the points where a type has to stop being
      undecided — a local's declared type, a loop bound, a constructor argument.
- [x] `& | ~ << >>`
- [x] `bitcast_f32` / `bitcast_i32` / `bitcast_u32` — spelled with the target in
      the name, because Lua has no `bitcast<T>` syntax to put it in
- [x] Conversions: `f32(x)`, `i32(x)`, `u32(x)`
- [x] §3.3's eight builtins

**Why abstract literals were not optional.** A hash multiplier like
`2654435769` needs thirty-two bits and an `f32` has twenty-four of mantissa. Had
literals stayed floats, there would have been no way to write a hash at all —
and therefore no noise except the `sin(dot(p, k)) * 43758.5453` trick, which is
what the port suite used because it had no choice. `integer_hash_noise_ports`
is the port that could not be written before this, and
`integer_hash_noise_paints` is the same thing through a real adapter.

### 4.3 Statements — **W6 done**

- [x] `continue`, spelled `goto continue` with a `::continue::` label. The
      question was whether to invent a keyword; the answer was that Lua authors
      already have an idiom for this and it is real Lua syntax, so nothing had
      to be invented. Any other `goto` is still refused, and says which one is
      available.
- [x] `discard`, spelled as a call — Lua has no keyword to spare, and it is the
      one call whose entire point is its effect rather than its value.
- [x] `switch` — **recognised, not spelled.** Lua has no `switch` and no syntax
      to spell one, so it comes from the shape instead: an `if`/`elseif` chain
      testing one whole number against distinct constants, with an `else` to
      land in, is emitted as a WGSL `switch`. The author writes what they would
      have written anyway and the driver gets a jump table rather than a ladder
      of comparisons. The recognition is narrow on purpose — different subjects,
      a float subject, a repeated case or a missing `else` all stay an `if`
      chain, because in an `if` chain the first matching arm wins and in a
      `switch` a repeated case is an error.

**The bug `continue` could have had.** A `continue` jumps past the tail of the
loop body, so a numeric `for` whose counter advanced there would never advance
at all — bounded by the guard, but silently wrong. WGSL's `continuing` block
exists for exactly this, and the counter lives in it now. A `while` loop gets
none, because its condition is already re-checked at the top and an empty
`continuing` would be noise in every generated shader.

### 4.4 Derivatives and relational — **W4 done**

- [x] `dpdx` `dpdy` `fwidth`
- [x] The `Coarse`/`Fine` variants. The argument for leaving them out — a
      precision hint nobody writes — was worth less than six table entries.
- [x] `all` `any` `isNan` `isInf`. `all` and `any` fold a single bool for now —
      WGSL folds a bool *vector*, and this language has no such type until
      comparisons on vectors exist.

Derivatives mattered more than they looked: `field.wgsl` antialiases its own
edges with `fwidth`, and before this a configuration's shader could not — so a
shader that drew its own shape had no way to soften it at the resolution it was
actually being drawn at. That asymmetry is closed.

**Uniformity — one check, not two.** WGSL forbids a derivative under non-uniform
control flow, the same rule `texture` already lived under, and both are refused
by the same walk in `validate.rs` naming whichever call it found. A check per
builtin is how the two would have drifted. The diagnostic says why the rule
exists — the call reads neighbouring pixels, which have to have taken the same
path — and what to do instead, and there is a test proving the suggested fix
actually compiles.

### 4.5 Arrays and structs — **W5 mostly done**

- [x] `array<T, N>` with a constant length, written as a Lua list — which is
      what a palette or a convolution kernel wants to be, and what an author
      will write without being told
- [x] Indexing `a[i]`, on arrays, vectors and matrix columns
- [x] `modf` and `frexp`, **without** a user-facing struct. WGSL returns one
      from both and names it internally — `__modf_result_f32` — which is not
      something this compiler should be spelling. The result is a type whose
      only operation is reading `.fract`, `.whole` or `.exp`, which is the whole
      of what anybody does with one, and a `local` holding it is emitted as an
      un-annotated `let` so WGSL infers the name itself.
- [x] Records, from a Lua table with named keys. **Structurally typed**: there
      is nowhere in Lua to *declare* a struct, so identity comes from the shape,
      and `{x = 1, y = 2}` and `{y = 3, x = 4}` are one type. The fields are
      sorted for the same reason a Lua table has no order of its own.

**A bug only the GPU test could see.** The interner is process-wide, so emitting
the record list from it put every record any shader had ever used into every
shader compiled after it. Records are collected from the program being emitted
now. The same test found that a helper's parameter list spelled types with
`wgsl()`, which cannot name a record or an array — an array parameter would have
hit it too.

**Uniform stride.** In the uniform address space an array's stride is a
multiple of sixteen whatever the element is, so four `f32` occupy sixty-four
bytes rather than sixteen. That is the same rule that rejected the first attempt
at padding the parameter block back in M4, and it is asserted on both sides.

**A gap this step exposed.** `clamp(i32(...), 0, 3)` was refused, because the
componentwise shapes excluded whole numbers — WGSL defines `abs`, `clamp`,
`min`, `max` and `sign` for integers and this language did not. Those five have
their own shape now, kept separate rather than flagged, because `sin` of an
integer really is undefined and letting it through would mean a driver refusing
it with no line number.

### 4.6 Indexing — **W5 done**

`v[i]`, `m[i]` and `a[i]` all work. The note that used to point at `v.x` was
right until arrays existed and then became a wrong answer, which is what it was
flagged as here.

A float index is refused by name rather than rounded: reading the wrong element
silently is how a shader goes subtly wrong and nobody finds out.

---

## 5. Host-side surface — **done**

These were not compiler work: each needed a decision about what the *renderer*
offers before the language could name it. All three are implemented.

### 5.1 Named textures

A configuration declares them and the host binds them:

```lua
morf.shader("tinted", {
  textures = { mask = "~/wall.png", ramp = "~/ramp.png" },
  fragment = [[
    function fragment(uv, time, resolution, coverage)
      local m = texture(mask, uv)
      return texture(ramp, vec2(m.r, 0.5))
    end
  ]],
})
```

- [x] Named samplers a configuration declares, bound by the host
- [x] `texture(name, uv)` beside the existing `texture(uv)`, which still means
      what is underneath in effect mode. Arity decides which, and sampling a
      *named* texture does not make a node into a layer — there is nothing
      being read from beneath it.
- [x] `texture_size`, `texture_load`, `texture_level`.

      "Sampling is what a shader does with a texture" was wrong, and the example
      three paragraphs up is why: a `ramp` is a *palette*, and sampling a
      palette interpolates between entries — so the colour halfway between two
      swatches is a colour that is in neither. `texture_load` reads an exact
      texel and is what a lookup table needs. `texture_size` is what indexing
      one needs, and `texture_level` is sampling at a chosen mip.

A texture is not a value — it cannot be added, stored or returned, and trying
says so. The only thing a shader does with one is sample it, because that is the
only thing a binding *is*.

### 5.2 Vertex displacement

```lua
morf.shader("wave", {
  vertex = [[
    function vertex(corner, size, time)
      return corner + vec2(0.0, sin(time + corner.x * 0.05) * 6.0)
    end
  ]],
  fragment = [[ ... ]],
})
```

- [x] A second stage, compiled separately — a different signature, and a shader
      may have one without the other.

**It moves the quad, not the shape inside it.** The fragment stage still walks
the field in the node's own space, so a displaced node keeps its geometry and
takes it somewhere else. The first attempt displaced both, which cancelled out
exactly, and the GPU test is what noticed.

A vertex shader has no `uv` and no `coverage` — it runs once per corner, before
there is a fragment — and it cannot take a derivative or sample what is
underneath, both refused by name. It also declares no uniform block: it takes
the clock as an argument, and two blocks of one name in a module is a
redefinition.

### 5.3 Data blocks

```lua
morf.shader("bars", { data = { spectrum = 64 }, fragment = [[ ... ]] })
morf.shader_data(node, "spectrum", levels)
```

- [x] Read-only storage the host fills each frame. Larger than a uniform can
      hold — a spectrum, a lookup table, a history.

**Read-only on purpose, and this is the design decision.** Every pixel of a node
runs the fragment shader, so a writable shared block would be a race between all
of them: offering one would be offering a bug. A shader that needs to *keep*
state between frames needs ping-pong targets and a defined update order, which
is a different feature from this one and should be planned as such.

The values are copied at the declared length — truncated or zero-padded — so a
configuration handing over the wrong number of them is a mistake that survives
rather than a frame reading past the end of a buffer.

## 6. Out of scope, and why

Not missing. Meaningless in a per-node fragment shader, and building them would
be building for nobody:

- **Compute shaders, workgroups, `workgroupBarrier`, `storageBarrier`.** There
  is no dispatch here to belong to.
- **Atomics.** Nothing to contend over.
- **Pointers and `ptr<>`.** A shader here has no aliasing to express.
- **`f16`.** Wants a device feature and buys nothing at this size.
- **Override declarations / pipeline-overridable constants.** Our parameters are
  uniforms, which is the same capability by a different route, and already
  animatable through the scene.

If any of these ever becomes relevant it will be because §5.2 or §5.3 landed
first, and it should be reconsidered then rather than pre-emptively.

---

## 7. Order

Each step is self-contained: a type, its operators, its builtins, its
diagnostics, its tests. Each ends green on `oslo make verify` and is one commit.

**W1 — the thirteen easy builtins** (§3.1, minus `modf`/`frexp`). No new types,
no new syntax; a table entry and a test each. Gets the arithmetic surface from
34/79 to 45/79 in an afternoon and makes the next steps' tests easier to write.

**W2 — matrices** (§4.1, §3.2). The highest-value gap. Uniform layout is the
part that will bite: a `mat3x3` is not nine floats, and the packer and a test
have to say so together.

**W3 — integers and bitwise** (§4.2, §3.3). The literal-typing rules are the
delicate part — `1 / 2` must stay `0.5` in a float context while `1 << 2` is an
integer — and the diagnostics matter more than the feature.

**W4 — derivatives and relational** (§4.4). Small, and it closes a real
asymmetry: the engine's own shader antialiases with `fwidth` and a user's
cannot. Generalise the uniformity check while here.

**W5 — arrays, indexing, structs** (§4.5, §4.6). Unlocks `modf` and `frexp`,
and lets a configuration write a palette as a Lua table, which is what it will
try to do anyway.

**W6 — `continue` and `discard`** (§4.3). Needs a syntax decision for
`continue`, which is why it is not first despite being small.

**Then** §5, which is host design rather than compiler work and should be
planned separately once there is a use asking for it.

W1 and W2 are most of the value. If this stops after W2, the language can
express rotation, and that is the thing people notice missing.

---

## 8. How each step is checked

The same three, per step, because the ports are the only measure that has
actually caught anything:

1. **Golden tests** — the emitted WGSL for each new form.
2. **Diagnostic tests** — what a wrong use says, with its line number. A feature
   whose error message is bad is a feature people cannot use.
3. **A port that needed it.** W2 is done when a shader that rotates something
   compiles and paints; W3 when integer-hash noise does. A builtin added without
   a shader asking for it is how §3.5 of the shader plan got written short in
   the first place.

---

## 9. Status

**Everything in this document is implemented. No `- [ ]` boxes remain.**

The last two were mine rather than the plan's, and both arguments were weaker
than they looked. `switch` did not need syntax invented, only a shape
recognised. And "nothing is asking for" the texture reads was contradicted by
this document's own palette example, which is wrong without `texture_load`.

§6 remains what it always was: things that cannot mean anything in a per-node
fragment shader, listed so "out of scope" is never mistaken for "undone".

- [x] **W1 — the ordinary builtins.** 42 of 79 math functions, from 34.
- [x] **W2 — matrices.** 45 of 79, and rotation is writable.
- [x] **W3 — integers and bitwise.** 53 of 79, and a real hash is writable.
- [x] **W4 — derivatives and relational.** A shader can antialias its own edge.
- [x] **W5 — arrays and indexing.** Structs deferred; nothing wants one yet.
- [x] **W6 — `continue` and `discard`.** No new syntax was invented.
- [x] **W7 — the deferrals.** `ldexp`, `outer`, the coarse and fine
      derivatives, integer vector types, and all eighteen packing builtins.
      **79 of 79.**
- [x] **W8 — records, `modf` and `frexp`.** The last compiler-side boxes.
- [x] **W9 — §5.** Named textures, vertex displacement, read-only data blocks.
- [x] **W10 — the last two.** `switch` by recognition, and the texture reads.

W7 and W8 were not in the original order. They exist because the first six
steps deferred eleven items with reasons, and a reason is not the same as being
done — a plan with unchecked boxes in it is not an implemented plan. Of those
eleven, one (`m[0]`) was already done and the box was stale, one (`ldexp`) was a
straight miss that W1 handed to W3 and W3 never collected, and the rest were
judgement calls that turned out to cost less than the arguments for skipping
them.
